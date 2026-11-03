//! A connection's knowledge sources, as the audit reads them: adding each kind (a folder, a git
//! repository, files, web pages), checking or reading one again, and bringing them all up to
//! date before a run. No Tauri types: the commands in commands/audit.rs pick files and folders
//! and call these. See repos.rs for git and knowledge.rs for files and web pages.

use super::{knowledge, repos};
use crate::error::AppResult;
use crate::store::Store;
use crate::store::code_sources::{KnowledgeSource, SourceKind, default_label};
use crate::store::secrets;
use std::path::{Path, PathBuf};

/// The message when a connection has no sources yet.
pub const NONE_YET: &str =
    "Add a source first: the folder, repository, files or web pages about this database.";

fn now_ms() -> u64 {
    chrono::Utc::now().timestamp_millis().max(0) as u64
}

/// A git source's token from the keychain, with the user name its host expects. The keychain
/// is only read when a token was saved.
fn token_for(source: &KnowledgeSource) -> Option<repos::Token> {
    let SourceKind::Git {
        url,
        has_token: true,
        token_user,
        ..
    } = &source.kind
    else {
        return None;
    };
    let user = token_user
        .clone()
        .or_else(|| repos::token_user(url).map(String::from))?;
    Some(repos::Token {
        user,
        secret: secrets::repo_token(&source.id)?,
    })
}

/// Adds a folder the person picked as a source, with its git commit if it has one.
pub async fn add_folder(store: &Store, id: &str, path: PathBuf) -> AppResult<KnowledgeSource> {
    let path = dunce::canonicalize(&path).unwrap_or(path);
    let path = path.to_string_lossy().into_owned();
    let label = default_label(&path);
    let source =
        store.add_knowledge_source(id, SourceKind::Local { path: path.clone() }, &label)?;
    let commit = repos::head(Path::new(&path)).await;
    Ok(store
        .update_knowledge_source(id, &source.id, |s| {
            s.last_fetch = Some(now_ms());
            s.last_commit = commit;
        })?
        .unwrap_or(source))
}

/// Adds a git repository as a source once git can read it (`git ls-remote`). `token`: an
/// optional read-only token, kept in the keychain; `token_user`: the user name sent with it,
/// needed for hosts SayQL doesn't know.
pub async fn add_repo(
    store: &Store,
    id: &str,
    url: &str,
    branch: &str,
    subdir: &str,
    token: Option<String>,
    token_user: Option<String>,
) -> AppResult<KnowledgeSource> {
    let url = repos::check_url(url)?;
    let branch = repos::check_branch(branch)?;
    let subdir = repos::check_subdir(subdir)?;
    let token = token
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty());
    let custom_user = token_user
        .map(|u| u.trim().to_string())
        .filter(|u| !u.is_empty());
    let token = match token {
        None => None,
        Some(_) if !url.starts_with("https://") => {
            return Err("A token works with an https:// address. For an SSH address, git uses your SSH key.".into());
        }
        Some(secret) => {
            let user = match (&custom_user, repos::token_user(&url)) {
                (Some(u), _) => u.clone(),
                (None, Some(u)) => u.to_string(),
                (None, None) => {
                    return Err(format!(
                        "Enter the user name {} expects with a token.",
                        repos::host(&url).unwrap_or_else(|| "this server".into())
                    )
                    .into());
                }
            };
            Some(repos::Token { user, secret })
        }
    };
    repos::check_access(&url, &branch, token.as_ref()).await?;
    let label = default_label(subdir.as_deref().unwrap_or(&url));
    let source = store.add_knowledge_source(
        id,
        SourceKind::Git {
            url,
            branch,
            subdir,
            has_token: token.is_some(),
            token_user: custom_user.filter(|_| token.is_some()),
        },
        &label,
    )?;
    if let Some(t) = &token
        && let Err(e) = secrets::set_repo_token(&source.id, &t.secret)
    {
        let _ = store.remove_knowledge_source(id, &source.id);
        return Err(e);
    }
    Ok(source)
}

/// Adds files the person picked (at most `knowledge::MAX_FILES`, checked by the caller) as one
/// source, their text read now.
pub async fn add_files(store: &Store, id: &str, paths: Vec<PathBuf>) -> AppResult<KnowledgeSource> {
    let paths: Vec<String> = paths
        .into_iter()
        .map(|path| {
            let path = dunce::canonicalize(&path).unwrap_or(path);
            path.to_string_lossy().into_owned()
        })
        .collect();
    let label = match paths.as_slice() {
        [one] => default_label(
            &Path::new(one)
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default(),
        ),
        _ => "docs".to_string(),
    };
    let source = store.add_knowledge_source(id, SourceKind::Files { paths }, &label)?;
    added(store, id, source).await
}

/// Adds a web page as a source, or with `crawl` the pages in its folder on the same site,
/// once they've been read.
pub async fn add_url(
    store: &Store,
    id: &str,
    url: &str,
    crawl: bool,
) -> AppResult<KnowledgeSource> {
    let url = knowledge::check_web_url(url)?;
    let label = default_label(url.host_str().unwrap_or("docs"));
    let source = store.add_knowledge_source(
        id,
        SourceKind::Url {
            url: url.to_string(),
            crawl,
        },
        &label,
    )?;
    added(store, id, source).await
}

/// Reads a just-added files or web source; it's removed again when nothing could be read.
async fn added(store: &Store, id: &str, source: KnowledgeSource) -> AppResult<KnowledgeSource> {
    match snapshot(store, &source).await {
        Ok((version, problem)) => Ok(store
            .update_knowledge_source(id, &source.id, |s| {
                s.last_fetch = Some(now_ms());
                s.last_commit = Some(version);
                s.error = problem;
            })?
            .unwrap_or(source)),
        Err(e) => {
            let _ = store.remove_knowledge_source(id, &source.id);
            Err(e.into())
        }
    }
}

/// Reads a files or web source again into its copy: the new version, and which files
/// couldn't be read if some couldn't. An error when none could; the copy is kept then.
async fn snapshot(
    store: &Store,
    source: &KnowledgeSource,
) -> Result<(String, Option<String>), String> {
    let (docs, problem) = match &source.kind {
        SourceKind::Files { paths } => {
            let paths = paths.clone();
            let (docs, problems) =
                tokio::task::spawn_blocking(move || knowledge::read_files(&paths))
                    .await
                    .map_err(|e| format!("Couldn't read the files: {e}"))?;
            if docs.is_empty() {
                return Err(problems
                    .into_iter()
                    .next()
                    .unwrap_or_else(|| "None of its files could be read.".into()));
            }
            let problem = match problems.as_slice() {
                [] => None,
                [one] => Some(one.clone()),
                [first, rest @ ..] => Some(format!(
                    "{first} ({} more files couldn't be read.)",
                    rest.len()
                )),
            };
            (docs, problem)
        }
        SourceKind::Url { url, crawl } => (
            knowledge::fetch_web(url, *crawl)
                .await
                .map_err(|e| e.to_string())?,
            None,
        ),
        SourceKind::Local { .. } | SourceKind::Git { .. } => {
            return Err("Only files and web pages are read into a copy.".into());
        }
    };
    let dest = store.repo_dir(&source.id).map_err(|e| e.to_string())?;
    let version = knowledge::write_snapshot(&dest, &docs).map_err(|e| e.to_string())?;
    Ok((version, problem))
}

/// Checks a source can still be read: the folder is there, or git can reach the repository.
/// Files and web pages are read again. Its status is updated either way.
pub async fn refresh(store: &Store, id: &str, source_id: &str) -> AppResult<KnowledgeSource> {
    let source = store.knowledge_source(id, source_id)?;
    // Read just now (a folder found, files or pages read), rather than only reachable.
    let mut read_now = false;
    let (commit, error) = match &source.kind {
        SourceKind::Local { path } => {
            let dir = Path::new(path);
            if dir.is_dir() {
                read_now = true;
                (repos::head(dir).await, None)
            } else {
                (
                    None,
                    Some(format!("The folder {path} isn't there any more.")),
                )
            }
        }
        SourceKind::Git { url, branch, .. } => {
            let token = token_for(&source);
            match repos::check_access(url, branch, token.as_ref()).await {
                Ok(()) => (source.last_commit.clone(), None),
                Err(e) => (source.last_commit.clone(), Some(e.to_string())),
            }
        }
        SourceKind::Files { .. } | SourceKind::Url { .. } => match snapshot(store, &source).await {
            Ok((version, problem)) => {
                read_now = true;
                (Some(version), problem)
            }
            Err(e) => (source.last_commit.clone(), Some(e)),
        },
    };
    Ok(store
        .update_knowledge_source(id, source_id, |s| {
            if read_now {
                s.last_fetch = Some(now_ms());
            }
            s.last_commit = commit;
            s.error = error;
        })?
        .unwrap_or(source))
}

/// A source ready for the run: where its code or text is on this computer.
pub struct Ready {
    pub label: String,
    pub kind: &'static str,
    /// Documents (files, web pages) rather than code.
    pub document: bool,
    pub dir: PathBuf,
    pub url: Option<String>,
    pub commit: Option<String>,
}

/// Brings each source up to date (git sources are fetched, files and web pages read again)
/// and records how it went. A source that fails is left out with a warning; one that can't
/// be fetched is read from the copy made before, if any. `say(kind, text)`: progress for the window.
pub async fn ready(
    store: &Store,
    connection_id: &str,
    say: &(dyn Fn(&'static str, String) + Send + Sync),
) -> AppResult<Vec<Ready>> {
    let sources = store.knowledge_sources(connection_id)?;
    if sources.is_empty() {
        return Err(NONE_YET.into());
    }
    let mut out = Vec::new();
    let mut first_error = None;
    for source in &sources {
        // Files read just now, even when some of them couldn't be.
        let mut read_now = false;
        let (ready, error) = match &source.kind {
            SourceKind::Local { path } => {
                let dir = Path::new(path);
                // Not std's canonicalize: on Windows it gives `\\?\C:\…`, which OpenCode's
                // path rules don't match.
                match dunce::canonicalize(dir) {
                    Ok(dir) if dir.is_dir() => {
                        let commit = repos::head(&dir).await;
                        (
                            Some(Ready {
                                label: source.label.clone(),
                                kind: source.kind.name(),
                                document: false,
                                dir,
                                url: None,
                                commit,
                            }),
                            None,
                        )
                    }
                    _ => (
                        None,
                        Some(format!("The folder {path} isn't there any more.")),
                    ),
                }
            }
            SourceKind::Git {
                url,
                branch,
                subdir,
                ..
            } => {
                say("step", format!("Fetching {}…", source.label));
                let dest = store.repo_dir(&source.id)?;
                let token = token_for(source);
                let (commit, error) = match repos::sync(url, branch, &dest, token.as_ref()).await {
                    Ok(sha) => (Some(sha), None),
                    Err(e) => {
                        let old = repos::head(&dest).await;
                        if old.is_some() {
                            say(
                                "warn",
                                format!(
                                    "Couldn't fetch {}: {e} Reading the copy fetched before.",
                                    source.label
                                ),
                            );
                        }
                        (old, Some(e.to_string()))
                    }
                };
                let dir = match subdir {
                    Some(sub) => dest.join(sub),
                    None => dest.clone(),
                };
                match (&commit, dunce::canonicalize(&dir)) {
                    (Some(_), Ok(dir)) if dir.is_dir() => (
                        Some(Ready {
                            label: source.label.clone(),
                            kind: source.kind.name(),
                            document: false,
                            dir,
                            url: Some(url.clone()),
                            commit: commit.clone(),
                        }),
                        error,
                    ),
                    (Some(_), _) => (
                        None,
                        Some(format!(
                            "The repository has no folder {}.",
                            subdir.as_deref().unwrap_or("")
                        )),
                    ),
                    (None, _) => (None, error),
                }
            }
            SourceKind::Files { .. } | SourceKind::Url { .. } => {
                say("step", format!("Reading {}…", source.label));
                let (version, error) = match snapshot(store, source).await {
                    Ok((version, problem)) => {
                        if let Some(p) = &problem {
                            say("warn", format!("{}: {p}", source.label));
                        }
                        read_now = true;
                        (Some(version), problem)
                    }
                    Err(e) => {
                        let old = source.last_commit.clone();
                        if old.is_some() {
                            say(
                                "warn",
                                format!(
                                    "Couldn't read {}: {e} Reading the copy made before.",
                                    source.label
                                ),
                            );
                        }
                        (old, Some(e))
                    }
                };
                let dir = store.repo_dir(&source.id)?;
                match (&version, dunce::canonicalize(&dir)) {
                    (Some(_), Ok(dir)) if dir.is_dir() => (
                        Some(Ready {
                            label: source.label.clone(),
                            kind: source.kind.name(),
                            document: true,
                            dir,
                            url: match &source.kind {
                                SourceKind::Url { url, .. } => Some(url.clone()),
                                _ => None,
                            },
                            commit: version.clone(),
                        }),
                        error,
                    ),
                    _ => (None, error),
                }
            }
        };
        let fetched = ready.is_some() && (error.is_none() || read_now);
        let commit = ready.as_ref().and_then(|r| r.commit.clone());
        let _ = store.update_knowledge_source(connection_id, &source.id, |s| {
            if fetched {
                s.last_fetch = Some(now_ms());
            }
            if commit.is_some() {
                s.last_commit = commit;
            }
            s.error = error.clone();
        });
        match ready {
            Some(r) => out.push(r),
            None => {
                if let Some(e) = &error {
                    say("warn", format!("Leaving out {}: {e}", source.label));
                    first_error.get_or_insert_with(|| e.clone());
                }
            }
        }
    }
    if out.is_empty() {
        return Err(format!(
            "None of this connection's sources could be read. {}",
            first_error.unwrap_or_default()
        )
        .trim()
        .to_string()
        .into());
    }
    Ok(out)
}
