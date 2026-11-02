//! "Run audit": finds the person's coding agents, runs one read-only over a connection's
//! knowledge sources (local folders, git repositories, files and web pages, each fetched or
//! read again first) with the audit instructions,
//! streams its progress to the window as `audit://event`, and opens the same run in a terminal
//! instead. Without Claude Code it runs SayQL's built-in OpenCode, downloaded on first use. See
//! src/audit_runner.rs and src/audit_opencode.rs for what's enforced, src/repos.rs for git, and
//! src/knowledge.rs for files and web pages.

use crate::agent_process::{TerminalRun, Tree, kill_tree, launch_terminal, no_window, version_of};
use crate::ai::{KeyNeed, SAYQL_CREDITS, endpoint};
use crate::audit::{self, Target};
use crate::audit_opencode::{self as opencode, OpenCodeStream};
use crate::audit_runner::run_dir::{self, RunDir};
use crate::audit_runner::{
    self as runner, AgentSpec, AuditModel, AuditPayment, ClaudeStream, RunFiles, RunStream,
};
use crate::commands::settings::mcp_binary;
use crate::error::{AppError, AppResult};
use crate::knowledge;
use crate::repos;
use crate::state::AppState;
use crate::store::apps::AppGate;
use crate::store::code_sources::{KnowledgeSource, SourceKind, default_label};
use crate::store::secrets;
use serde::Serialize;
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio_util::sync::CancellationToken;

/// Must match AUDIT_EVENT in src/api/events.ts.
const EVENT: &str = "audit://event";
/// The stderr kept for an error message.
const STDERR_MAX: usize = 16 * 1024;

/// Running audits the window can cancel, by the id it gave each run.
#[derive(Default)]
pub struct AuditRuns(Mutex<HashMap<String, CancellationToken>>);

/// A coding agent SayQL knows about, as found on this computer.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentView {
    id: &'static str,
    label: &'static str,
    /// Where it's installed; `None` when it isn't found.
    path: Option<String>,
    version: Option<String>,
    /// SayQL can keep it read-only (only these can be run).
    supported: bool,
    /// Found, supported, and recent enough: it can run an audit now.
    ready: bool,
    /// Why it can't run, when found but not ready.
    problem: Option<String>,
    install_url: &'static str,
    /// SayQL's built-in OpenCode, rather than an agent the person installed.
    built_in: bool,
    /// What the first run downloads, in bytes, while it isn't downloaded yet.
    download_bytes: Option<u64>,
    /// The AI it talks to: "SayQL AI credits · claude-sonnet-5-5".
    detail: Option<String>,
}

#[derive(Serialize, Clone)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
enum AuditEvent {
    Line {
        run_id: String,
        kind: &'static str,
        text: String,
    },
    /// Downloading the built-in OpenCode before the run.
    Download {
        run_id: String,
        received: u64,
        total: u64,
    },
    Done {
        run_id: String,
        /// "ok", "error", "cancelled" or "timeout".
        outcome: &'static str,
        /// Suggestions SayQL received for review.
        suggested: u32,
        /// The agent's closing summary.
        summary: Option<String>,
        error: Option<String>,
    },
}

async fn view(spec: &'static AgentSpec) -> AgentView {
    let path = runner::locate(spec);
    let (version, problem) = match (&path, spec.unsupported) {
        (Some(p), None) => {
            let v = version_of(p).await;
            let problem =
                runner::version_problem(spec.id, v.as_deref().and_then(runner::parse_version));
            (v, problem)
        }
        (Some(_), Some(why)) => (None, Some(why.to_string())),
        (None, _) => (None, None),
    };
    AgentView {
        id: spec.id,
        label: spec.label,
        ready: path.is_some() && spec.unsupported.is_none() && problem.is_none(),
        path: path.map(|p| p.to_string_lossy().into_owned()),
        version,
        supported: spec.unsupported.is_none(),
        problem,
        install_url: spec.install_url,
        built_in: false,
        download_bytes: None,
        detail: None,
    }
}

/// The model the built-in OpenCode would use, from Settings → AI provider (the keychain isn't
/// read for this).
fn opencode_model(state: &AppState) -> AppResult<opencode::Model> {
    let settings = state.store.settings().unwrap_or_default();
    let p = endpoint::provider(&settings.provider)?;
    Ok(opencode::model_for(p, &settings.provider_settings(p.id))?)
}

/// The built-in OpenCode: always offered, ready unless this computer or the settings rule it out.
fn opencode_view(state: &AppState) -> AgentView {
    let dir = opencode::install_dir(&state.runners_dir);
    let installed = opencode::present(&dir);
    let model = opencode_model(state);
    let problem = match (
        opencode::this_platform(),
        opencode::managed_policy(),
        &model,
    ) {
        (Err(e), _, _) => Some(e),
        (_, Some(e), _) => Some(e),
        (_, _, Err(e)) => Some(e.to_string()),
        _ => None,
    };
    AgentView {
        id: opencode::ID,
        label: opencode::LABEL,
        path: installed.as_ref().map(|p| p.to_string_lossy().into_owned()),
        version: Some(opencode::VERSION.into()),
        supported: true,
        ready: problem.is_none(),
        problem,
        install_url: "https://opencode.ai",
        built_in: true,
        download_bytes: match installed {
            Some(_) => None,
            None => opencode::this_platform().ok().map(|p| p.download_bytes()),
        },
        detail: model.ok().map(|m| format!("{} · {}", m.label, m.model)),
    }
}

/// The coding agents SayQL knows, whether each is installed, and whether it can run an audit;
/// SayQL's built-in OpenCode last.
#[tauri::command]
pub async fn audit_agents(state: State<'_, AppState>) -> AppResult<Vec<AgentView>> {
    let mut out = Vec::new();
    for spec in &runner::AGENTS {
        out.push(view(spec).await);
    }
    let state = state.inner();
    out.push(opencode_view(state));
    Ok(out)
}

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

/// The knowledge an audit of this connection reads, in the order added.
#[tauri::command]
pub fn knowledge_sources(
    state: State<'_, AppState>,
    id: String,
) -> AppResult<Vec<KnowledgeSource>> {
    state.store.knowledge_sources(&id)
}

/// Asks for a folder and adds it as a source; `None` when cancelled.
#[tauri::command]
pub async fn add_knowledge_folder(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
) -> AppResult<Option<KnowledgeSource>> {
    use tauri_plugin_dialog::DialogExt;
    let dialog = app
        .dialog()
        .file()
        .set_title("Choose a folder with this database's code");
    let (tx, rx) = tokio::sync::oneshot::channel();
    dialog.pick_folder(move |picked| {
        let _ = tx.send(picked);
    });
    let Some(picked) = rx.await.ok().flatten() else {
        return Ok(None);
    };
    let path = picked
        .into_path()
        .map_err(|e| AppError::User(format!("Could not use that folder: {e}")))?;
    let path = dunce::canonicalize(&path).unwrap_or(path);
    let path = path.to_string_lossy().into_owned();
    let label = default_label(&path);
    let source =
        state
            .store
            .add_knowledge_source(&id, SourceKind::Local { path: path.clone() }, &label)?;
    let commit = repos::head(std::path::Path::new(&path)).await;
    Ok(state
        .store
        .update_knowledge_source(&id, &source.id, |s| {
            s.last_fetch = Some(now_ms());
            s.last_commit = commit;
        })?
        .or(Some(source)))
}

/// Adds a git repository as a source once git can read it (`git ls-remote`). `token`:
/// an optional read-only token, kept in the keychain; `token_user`: the user name sent with
/// it, needed for hosts SayQL doesn't know.
#[tauri::command]
#[allow(
    clippy::too_many_arguments,
    reason = "a Tauri command takes each webview argument by name"
)]
pub async fn add_knowledge_repo(
    state: State<'_, AppState>,
    id: String,
    url: String,
    branch: String,
    subdir: String,
    token: Option<String>,
    token_user: Option<String>,
) -> AppResult<KnowledgeSource> {
    let url = repos::check_url(&url)?;
    let branch = repos::check_branch(&branch)?;
    let subdir = repos::check_subdir(&subdir)?;
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
    let source = state.store.add_knowledge_source(
        &id,
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
        let _ = state.store.remove_knowledge_source(&id, &source.id);
        return Err(e);
    }
    Ok(source)
}

/// Asks for files (documents, exports) and adds them as one source, their text read now;
/// `None` when cancelled.
#[tauri::command]
pub async fn add_knowledge_files(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
) -> AppResult<Option<KnowledgeSource>> {
    use tauri_plugin_dialog::DialogExt;
    let dialog = app
        .dialog()
        .file()
        .set_title("Choose files about this database")
        .add_filter("Documents", knowledge::FILE_EXTENSIONS);
    let (tx, rx) = tokio::sync::oneshot::channel();
    dialog.pick_files(move |picked| {
        let _ = tx.send(picked);
    });
    let Some(picked) = rx.await.ok().flatten().filter(|p| !p.is_empty()) else {
        return Ok(None);
    };
    if picked.len() > knowledge::MAX_FILES {
        return Err(format!(
            "Choose at most {} files for one source.",
            knowledge::MAX_FILES
        )
        .into());
    }
    let mut paths = Vec::new();
    for p in picked {
        let path = p
            .into_path()
            .map_err(|e| AppError::User(format!("Could not use that file: {e}")))?;
        let path = dunce::canonicalize(&path).unwrap_or(path);
        paths.push(path.to_string_lossy().into_owned());
    }
    let label = match paths.as_slice() {
        [one] => default_label(
            &std::path::Path::new(one)
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default(),
        ),
        _ => "docs".to_string(),
    };
    let source = state
        .store
        .add_knowledge_source(&id, SourceKind::Files { paths }, &label)?;
    added(&state, &id, source).await.map(Some)
}

/// Adds a web page as a source, or with `crawl` the pages in its folder on the same site,
/// once they've been read.
#[tauri::command]
pub async fn add_knowledge_url(
    state: State<'_, AppState>,
    id: String,
    url: String,
    crawl: bool,
) -> AppResult<KnowledgeSource> {
    let url = knowledge::check_web_url(&url)?;
    let label = default_label(url.host_str().unwrap_or("docs"));
    let source = state.store.add_knowledge_source(
        &id,
        SourceKind::Url {
            url: url.to_string(),
            crawl,
        },
        &label,
    )?;
    added(&state, &id, source).await
}

/// Reads a just-added files or web source; it's removed again when nothing could be read.
async fn added(state: &AppState, id: &str, source: KnowledgeSource) -> AppResult<KnowledgeSource> {
    match snapshot(state, &source).await {
        Ok((version, problem)) => Ok(state
            .store
            .update_knowledge_source(id, &source.id, |s| {
                s.last_fetch = Some(now_ms());
                s.last_commit = Some(version);
                s.error = problem;
            })?
            .unwrap_or(source)),
        Err(e) => {
            let _ = state.store.remove_knowledge_source(id, &source.id);
            Err(e.into())
        }
    }
}

/// Reads a files or web source again into its copy: the new version, and which files
/// couldn't be read if some couldn't. An error when none could; the copy is kept then.
async fn snapshot(
    state: &AppState,
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
    let dest = state
        .store
        .repo_dir(&source.id)
        .map_err(|e| e.to_string())?;
    let version = knowledge::write_snapshot(&dest, &docs).map_err(|e| e.to_string())?;
    Ok((version, problem))
}

/// Removes a source, its token and SayQL's copy of it.
#[tauri::command]
pub fn remove_knowledge_source(
    state: State<'_, AppState>,
    id: String,
    source_id: String,
) -> AppResult<()> {
    state.store.remove_knowledge_source(&id, &source_id)
}

/// Checks a source can still be read: the folder is there, or git can reach the repository.
/// Files and web pages are read again. Its status is updated either way.
#[tauri::command]
pub async fn refresh_knowledge_source(
    state: State<'_, AppState>,
    id: String,
    source_id: String,
) -> AppResult<KnowledgeSource> {
    let source = state.store.knowledge_source(&id, &source_id)?;
    // Read just now (a folder found, files or pages read), rather than only reachable.
    let mut read_now = false;
    let (commit, error) = match &source.kind {
        SourceKind::Local { path } => {
            let dir = std::path::Path::new(path);
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
        SourceKind::Files { .. } | SourceKind::Url { .. } => {
            match snapshot(&state, &source).await {
                Ok((version, problem)) => {
                    read_now = true;
                    (Some(version), problem)
                }
                Err(e) => (source.last_commit.clone(), Some(e)),
            }
        }
    };
    Ok(state
        .store
        .update_knowledge_source(&id, &source_id, |s| {
            if read_now {
                s.last_fetch = Some(now_ms());
            }
            s.last_commit = commit;
            s.error = error;
        })?
        .unwrap_or(source))
}

/// A source ready for the run: where its code or text is on this computer.
struct Ready {
    label: String,
    kind: &'static str,
    /// Documents (files, web pages) rather than code.
    document: bool,
    dir: PathBuf,
    url: Option<String>,
    commit: Option<String>,
}

/// Brings each source up to date (git sources are fetched, files and web pages read again)
/// and records how it went. A source that fails is left out with a warning; one that can't
/// be fetched is read from the copy made before, if any. `say(kind, text)`: progress for the window.
async fn ready_sources(
    state: &AppState,
    connection_id: &str,
    say: &(dyn Fn(&'static str, String) + Send + Sync),
) -> AppResult<Vec<Ready>> {
    let sources = state.store.knowledge_sources(connection_id)?;
    if sources.is_empty() {
        return Err(
            "Add a source first: the folder, repository, files or web pages about this database."
                .into(),
        );
    }
    let mut out = Vec::new();
    let mut first_error = None;
    for source in &sources {
        // Files read just now, even when some of them couldn't be.
        let mut read_now = false;
        let (ready, error) = match &source.kind {
            SourceKind::Local { path } => {
                let dir = std::path::Path::new(path);
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
                let dest = state.store.repo_dir(&source.id)?;
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
                let (version, error) = match snapshot(state, source).await {
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
                let dir = state.store.repo_dir(&source.id)?;
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
        let _ = state
            .store
            .update_knowledge_source(connection_id, &source.id, |s| {
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

/// The model audits run on.
#[tauri::command]
pub fn audit_model(state: State<'_, AppState>) -> AppResult<AuditModel> {
    state.store.audit_model()
}

/// Remembers the model audits run on.
#[tauri::command]
pub fn set_audit_model(state: State<'_, AppState>, model: AuditModel) -> AppResult<()> {
    state.store.set_audit_model(model)
}

/// Who pays for audits, and whether a SayQL credit key is saved (so credits can be chosen).
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditPaymentView {
    payment: AuditPayment,
    has_credit_key: bool,
}

/// Credits count only while paying with them is on ([`runner::CREDITS_RUNS`], off for now) and
/// a credit key is saved; otherwise the run uses the person's own sign-in.
fn effective(stored: AuditPayment, has_credit_key: bool) -> AuditPayment {
    if runner::CREDITS_RUNS && has_credit_key {
        stored
    } else {
        AuditPayment::Subscription
    }
}

#[tauri::command]
pub fn audit_payment(state: State<'_, AppState>) -> AppResult<AuditPaymentView> {
    let has_credit_key = secrets::api_key(SAYQL_CREDITS).is_some();
    Ok(AuditPaymentView {
        payment: effective(
            state.store.audit_payment().unwrap_or_default(),
            has_credit_key,
        ),
        has_credit_key,
    })
}

/// Remembers who pays for audits.
#[tauri::command]
pub fn set_audit_payment(state: State<'_, AppState>, payment: AuditPayment) -> AppResult<()> {
    state.store.set_audit_payment(payment)
}

/// The SayQL credits service a run paid with credits talks to: its address (the one set in
/// Settings → AI provider, else sayql.dev) and the person's credit key from the keychain.
struct Credits {
    base_url: String,
    key: String,
}

/// The credits a run uses: `None` when it's paid with the person's own sign-in.
fn credits(state: &AppState) -> AppResult<Option<Credits>> {
    // Off for now (see `runner::CREDITS_RUNS`): a saved credits preference counts as the
    // person's own sign-in, and nothing points Claude Code at the credits service.
    if !runner::CREDITS_RUNS
        || state.store.audit_payment().unwrap_or_default() != AuditPayment::Credits
    {
        return Ok(None);
    }
    // No key saved any more: the person's own sign-in, as the dialog shows (see `effective`).
    let Some(key) = secrets::api_key(SAYQL_CREDITS) else {
        return Ok(None);
    };
    let provider = endpoint::provider(SAYQL_CREDITS)?;
    let settings = state.store.settings().unwrap_or_default();
    let base_url = endpoint::address(provider, &settings.provider_settings(SAYQL_CREDITS));
    Ok(Some(Credits { base_url, key }))
}

/// Everything a run needs, checked before anything starts.
struct Prepared {
    spec: &'static AgentSpec,
    bin: PathBuf,
    sources: Sources,
    files: RunFiles,
    dir: RunDir,
    model: AuditModel,
    payment: AuditPayment,
    /// Set when the run is paid with SayQL credits.
    credits: Option<Credits>,
}

/// The run's code sources, brought up to date, and the audit instructions over them.
struct Sources {
    /// The first code source: the agent's working directory.
    folder: PathBuf,
    /// The other code sources (Claude Code's `--add-dir`).
    more_dirs: Vec<PathBuf>,
    /// Every source by label, for the progress lines.
    sources: Vec<(String, PathBuf)>,
    connection_id: String,
    database: Option<String>,
    prompt: String,
    sayql_mcp: PathBuf,
}

/// Whether `app` may use SayQL now, as its MCP server would decide.
fn check_gate(state: &AppState, app: &str) -> AppResult<()> {
    match state.store.app_gate(app)? {
        AppGate::Allowed => Ok(()),
        AppGate::Blocked => Err(format!(
            "{app} is blocked from using SayQL. Allow it in Settings → AI apps → Activity first."
        )
        .into()),
        AppGate::NotApproved => Err(format!(
            "Only approved apps may use SayQL, and {app} isn't approved. Approve it in Settings → AI apps first."
        )
        .into()),
    }
}

/// Fetches the sources (last, once everything else checks out) and writes the instructions.
async fn prepare_sources(
    state: &AppState,
    id: &str,
    database: Option<&str>,
    headless: bool,
    say: &(dyn Fn(&'static str, String) + Send + Sync),
) -> AppResult<Sources> {
    let conn = state.store.connection(id)?;
    let database = audit::target_database(&state.store, &conn, database);
    let ready = ready_sources(state, &conn.id, say).await?;
    let dirs: Vec<String> = ready
        .iter()
        .map(|r| r.dir.to_string_lossy().into_owned())
        .collect();
    let listed: Vec<audit::Source> = ready
        .iter()
        .zip(&dirs)
        .map(|(r, dir)| audit::Source {
            label: &r.label,
            kind: r.kind,
            document: r.document,
            dir,
            url: r.url.as_deref(),
            commit: r.commit.as_deref(),
        })
        .collect();
    let mut prompt = audit::instructions_for(
        Some(&Target {
            connection_id: &conn.id,
            name: &conn.name,
            database: database.as_deref(),
        }),
        false,
        &listed,
    );
    prompt.push_str(runner::run_notes(headless));
    let sayql_mcp = mcp_binary()?;
    if !sayql_mcp.exists() {
        return Err(format!(
            "SayQL's MCP server isn't next to the app ({}). Reinstall SayQL; in a dev build, run `cargo build --bin sayql-mcp`.",
            sayql_mcp.display()
        )
        .into());
    }
    let sources: Vec<(String, PathBuf)> = ready.into_iter().map(|r| (r.label, r.dir)).collect();
    Ok(Sources {
        folder: sources[0].1.clone(),
        more_dirs: sources[1..].iter().map(|(_, d)| d.clone()).collect(),
        sources,
        connection_id: conn.id,
        database,
        prompt,
        sayql_mcp,
    })
}

/// Where runs keep their files: `<app cache>/audit`, this user's alone (see
/// [`run_dir::RunDir`]), never the shared temp folder.
fn audit_dir(app: &AppHandle) -> AppResult<PathBuf> {
    let cache = app
        .path()
        .app_cache_dir()
        .map_err(|e| AppError::User(format!("Could not find SayQL's cache folder: {e}")))?;
    Ok(cache.join("audit"))
}

/// Removes the run folders an earlier start left behind (SayQL quit or crashed mid-run), and
/// the ones older versions left in the temp folder. Runs in the background at startup.
pub fn clean_leftover_runs(app: &AppHandle) {
    let started = std::time::SystemTime::now();
    let Ok(dir) = audit_dir(app) else { return };
    std::thread::spawn(move || {
        run_dir::clean_leftovers(&dir, started);
        // Older versions kept runs in `<temp>/sayql-audit`.
        let _ = std::fs::remove_dir_all(std::env::temp_dir().join("sayql-audit"));
    });
}

#[allow(
    clippy::too_many_arguments,
    reason = "the run's settings, each from a different place"
)]
async fn prepare(
    state: &AppState,
    audit: &std::path::Path,
    kind: &str,
    agent: &str,
    id: &str,
    database: Option<&str>,
    headless: bool,
    say: &(dyn Fn(&'static str, String) + Send + Sync),
) -> AppResult<Prepared> {
    let spec = runner::spec(agent).ok_or("SayQL doesn't know this coding agent.")?;
    if let Some(why) = spec.unsupported {
        return Err(why.into());
    }
    if state.store.knowledge_sources(id)?.is_empty() {
        return Err(
            "Add a source first: the folder, repository, files or web pages about this database."
                .into(),
        );
    }
    let credits = credits(state)?;
    let payment = if credits.is_some() {
        AuditPayment::Credits
    } else {
        AuditPayment::Subscription
    };
    // A terminal would need the key in a script file or the shell: the app's own run keeps it
    // in the agent's environment only.
    if credits.is_some() && !headless {
        return Err(
            "Paying with SayQL credits works for runs inside SayQL. Start the audit here, or pay with your Claude Code sign-in to open it in Terminal."
                .into(),
        );
    }
    let bin = runner::locate(spec).ok_or_else(|| {
        format!(
            "{} isn't installed on this computer, or SayQL can't find it.",
            spec.label
        )
    })?;
    let version = version_of(&bin).await;
    if let Some(problem) =
        runner::version_problem(spec.id, version.as_deref().and_then(runner::parse_version))
    {
        return Err(problem.into());
    }
    // The agent reaches the database through SayQL's MCP server, which lets it in only as the
    // person allows: say so now rather than after a run that couldn't read anything.
    check_gate(state, spec.label)?;
    let sources = prepare_sources(state, id, database, headless, say).await?;
    let dir = RunDir::create(audit, kind)?;
    let files = RunFiles {
        mcp_config: dir.path().join("mcp.json"),
        settings: dir.path().join("settings.json"),
    };
    std::fs::write(
        &files.mcp_config,
        runner::mcp_config_json(&sources.sayql_mcp),
    )?;
    std::fs::write(&files.settings, runner::SETTINGS_JSON)?;
    Ok(Prepared {
        spec,
        bin,
        sources,
        files,
        dir,
        model: state.store.audit_model().unwrap_or_default(),
        payment,
        credits,
    })
}

/// Starts an audit with `agent` over the connection's code sources, fetching repositories
/// first; progress arrives as `audit://event` with `runId`.
#[tauri::command]
pub async fn start_audit(
    app: AppHandle,
    state: State<'_, AppState>,
    runs: State<'_, AuditRuns>,
    run_id: String,
    agent: String,
    id: String,
    database: Option<String>,
) -> AppResult<()> {
    let say = |kind: &'static str, text: String| {
        let _ = app.emit(
            EVENT,
            AuditEvent::Line {
                run_id: run_id.clone(),
                kind,
                text,
            },
        );
    };
    if agent == opencode::ID {
        return start_opencode(&app, &state, &runs, run_id, &id, database.as_deref()).await;
    }
    let audit = audit_dir(&app)?;
    let p = prepare(
        &state,
        &audit,
        run_dir::RUNS,
        &agent,
        &id,
        database.as_deref(),
        true,
        &say,
    )
    .await?;
    let mut cmd = tokio::process::Command::new(&p.bin);
    cmd.args(runner::claude_args(
        &p.files,
        &p.sources.more_dirs,
        true,
        p.model,
        p.payment,
    ))
    .current_dir(&p.sources.folder)
    .env("PATH", runner::child_path(&p.bin))
    // Started from inside a Claude Code session (a dev build), it would refuse to nest.
    .env_remove("CLAUDECODE")
    .stdin(Stdio::piped())
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
    .kill_on_drop(true);
    if let Some(c) = &p.credits {
        // The credit key goes to the agent's environment only: not its arguments, not a file.
        let env = runner::credits_env(
            &c.base_url,
            &c.key,
            p.model,
            std::env::vars_os().map(|(k, _)| k),
        );
        for name in &env.remove {
            cmd.env_remove(name);
        }
        for (name, value) in &env.set {
            cmd.env(name, value);
        }
    }
    #[cfg(unix)]
    cmd.process_group(0);
    no_window(&mut cmd);
    // From here on, dropping `child` stops the agent with everything it started, and dropping
    // `p.dir` removes the run's files.
    let mut child = Tree(
        cmd.spawn()
            .map_err(|e| AppError::User(format!("Could not start {}: {e}", p.spec.label)))?,
    );
    // The prompt goes on stdin: nothing about the run is on the command line but file paths.
    give_prompt(&mut child, &p.sources.prompt).await;
    let stop = CancellationToken::new();
    runs.0
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(run_id.clone(), stop.clone());
    let mut stream = ClaudeStream::with_sources(p.sources.sources.clone());
    stream.credits = p.credits.is_some();
    let run = Run {
        app: p.spec.label,
        dir: p.dir,
        connection_id: p.sources.connection_id,
        database: p.sources.database,
        secret: p.credits.map(|c| c.key),
    };
    tauri::async_runtime::spawn(drive(app, run_id, child, run, Box::new(stream), stop));
    Ok(())
}

/// Writes the prompt to the agent's stdin and closes it. An agent that stopped before reading
/// it (a bad setting, a refused key) breaks the pipe: the run goes on to read what it printed
/// and why it stopped, which says more than "Broken pipe".
async fn give_prompt(child: &mut Tree, prompt: &str) {
    if let Some(mut stdin) = child.0.stdin.take() {
        let _ = stdin.write_all(prompt.as_bytes()).await;
        drop(stdin);
    }
}

/// The environment SayQL's MCP server gets inside an OpenCode run: the person's real home and
/// app folders (OpenCode itself has a private home), so it finds SayQL's files and the keychain.
fn mcp_env() -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut put = |k: &str, v: Option<PathBuf>| {
        if let Some(v) = v {
            out.push((k.to_string(), v.to_string_lossy().into_owned()));
        }
    };
    put("HOME", dirs::home_dir());
    if cfg!(windows) {
        put("USERPROFILE", dirs::home_dir());
        put("APPDATA", std::env::var_os("APPDATA").map(PathBuf::from));
        put(
            "LOCALAPPDATA",
            std::env::var_os("LOCALAPPDATA").map(PathBuf::from),
        );
    } else {
        // Where `dirs` found them in the app, so the server's own lookup lands in the same place.
        put("XDG_CONFIG_HOME", dirs::config_dir());
        put("XDG_DATA_HOME", dirs::data_dir());
    }
    put(
        "SAYQL_CONFIG_DIR",
        std::env::var_os("SAYQL_CONFIG_DIR").map(PathBuf::from),
    );
    out
}

/// Starts an audit with SayQL's built-in OpenCode, downloading it first if this is the first
/// run (progress as `Download` events; cancel stops it).
async fn start_opencode(
    app: &AppHandle,
    state: &AppState,
    runs: &AuditRuns,
    run_id: String,
    id: &str,
    database: Option<&str>,
) -> AppResult<()> {
    let say = |kind: &'static str, text: String| {
        let _ = app.emit(
            EVENT,
            AuditEvent::Line {
                run_id: run_id.clone(),
                kind,
                text,
            },
        );
    };
    opencode::this_platform()?;
    if let Some(why) = opencode::managed_policy() {
        return Err(why.into());
    }
    if state.store.knowledge_sources(id)?.is_empty() {
        return Err(
            "Add a source first: the folder, repository, files or web pages about this database."
                .into(),
        );
    }
    let model = opencode_model(state)?;
    let key = if model.key == KeyNeed::None {
        None
    } else {
        let settings = state.store.settings().unwrap_or_default();
        secrets::api_key(&settings.provider)
    };
    if model.key == KeyNeed::Required && key.is_none() {
        return Err(format!(
            "Add your {} key in Settings → AI provider to run the audit with it.",
            model.label
        )
        .into());
    }
    check_gate(state, opencode::APP)?;
    let stop = CancellationToken::new();
    runs.0
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(run_id.clone(), stop.clone());
    let forget = || {
        runs.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&run_id);
    };
    let result = async {
        let dir = opencode::install_dir(&state.runners_dir);
        let installed = match tokio::task::block_in_place(|| opencode::installed(&dir)) {
            Some(i) => i,
            None => {
                say(
                    "step",
                    format!("Downloading OpenCode {} (once)…", opencode::VERSION),
                );
                let mut last = 0u64;
                let i = opencode::download(
                    &state.http,
                    &state.runners_dir,
                    &stop,
                    |received, total| {
                        // About every megabyte.
                        if received - last >= 1 << 20 || received == total {
                            last = received;
                            let _ = app.emit(
                                EVENT,
                                AuditEvent::Download {
                                    run_id: run_id.clone(),
                                    received,
                                    total,
                                },
                            );
                        }
                    },
                )
                .await?;
                say("step", "Downloaded and checked OpenCode".into());
                i
            }
        };
        let sources = prepare_sources(state, id, database, true, &say).await?;
        let dirs: Vec<PathBuf> = sources.sources.iter().map(|(_, d)| d.clone()).collect();
        // OpenCode would follow a link out of the sources: say which one rather than run.
        // And what looks like a secret in any case, which its rules can only deny one by one.
        let scan = dirs.clone();
        let (out, hidden) = tokio::task::spawn_blocking(move || {
            (opencode::link_out(&scan), opencode::secret_paths(&scan))
        })
        .await
        .map_err(|e| AppError::User(e.to_string()))?;
        if let Some((link, target)) = out {
            return Err(format!(
                "{} is a link to {}, outside this connection's knowledge sources, and SayQL's built-in OpenCode would follow it. Remove the link, or add what it points to as a source, then run the audit again.",
                link.display(),
                target.display()
            )
            .into());
        }
        let env_for_mcp = mcp_env();
        let config = opencode::config_json(&opencode::RunConfig {
            model: &model,
            sayql_mcp: &sources.sayql_mcp,
            mcp_env: &env_for_mcp,
            sources: &dirs,
            hidden: &hidden,
        })?;
        // Dropped on any way out below, it removes the run's files.
        let run_dir = RunDir::create(&audit_dir(app)?, run_dir::RUNS)?;
        let home = opencode::Home::new(run_dir.path());
        opencode::prepare_home(&home, &installed.bin_dir.join(opencode::exe("rg")))?;
        let mut cmd = tokio::process::Command::new(&installed.opencode);
        cmd.args(opencode::run_args(&model, &sources.folder))
            .current_dir(&sources.folder)
            .env_clear()
            .envs(opencode::child_env(
                &home,
                &sources.folder,
                &opencode::search_dir(&home, &installed),
                config,
                key.clone(),
                std::env::vars_os(),
            ))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        #[cfg(unix)]
        cmd.process_group(0);
        no_window(&mut cmd);
        let mut child = Tree(
            cmd.spawn()
                .map_err(|e| AppError::User(format!("Could not start OpenCode: {e}")))?,
        );
        // The prompt goes on stdin, never on the command line.
        give_prompt(&mut child, &sources.prompt).await;
        let stream = OpenCodeStream::new(sources.sources.clone(), &model);
        let run = Run {
            app: opencode::APP,
            dir: run_dir,
            connection_id: sources.connection_id,
            database: sources.database,
            secret: key,
        };
        AppResult::Ok((child, run, stream))
    }
    .await;
    match result {
        Ok((child, run, stream)) => {
            tauri::async_runtime::spawn(drive(
                app.clone(),
                run_id,
                child,
                run,
                Box::new(stream),
                stop,
            ));
            Ok(())
        }
        Err(e) => {
            forget();
            Err(e)
        }
    }
}

/// What a run needs after it started, whichever agent runs it.
struct Run {
    /// The app the audit is recorded for, as SayQL's app list names it.
    app: &'static str,
    /// The run's own files, removed when this is dropped.
    #[expect(dead_code, reason = "held for its Drop, which removes the files")]
    dir: RunDir,
    connection_id: String,
    database: Option<String>,
    /// A key the run was given (a credit key, an API key): never shown in the window.
    secret: Option<String>,
}

async fn drive(
    app: AppHandle,
    run_id: String,
    mut child: Tree,
    p: Run,
    mut stream: Box<dyn RunStream>,
    stop: CancellationToken,
) {
    // Nothing shown in the window carries a key.
    let key = p.secret.clone();
    let clean = |s: String| {
        let s = match key.as_deref().map(str::trim).filter(|k| k.len() >= 8) {
            Some(k) if !k.starts_with("sqlc_") => s.replace(k, "…"),
            _ => s,
        };
        runner::redact(&s, key.as_deref())
    };
    let emit = |e: AuditEvent| {
        let _ = app.emit(EVENT, e);
    };
    let stderr = child.0.stderr.take().map(|mut err| {
        tokio::spawn(async move {
            let mut buf = Vec::new();
            let _ = (&mut err)
                .take(STDERR_MAX as u64)
                .read_to_end(&mut buf)
                .await;
            // Keep reading so the agent never blocks on a full pipe.
            let _ = tokio::io::copy(&mut err, &mut tokio::io::sink()).await;
            String::from_utf8_lossy(&buf).into_owned()
        })
    });
    let mut lines = child.0.stdout.take().map(|o| BufReader::new(o).lines());
    let deadline = tokio::time::sleep(runner::TIME_LIMIT);
    tokio::pin!(deadline);
    let mut ended: Option<&'static str> = None;
    if let Some(lines) = lines.as_mut() {
        loop {
            tokio::select! {
                _ = stop.cancelled() => { ended = Some("cancelled"); break; }
                _ = &mut deadline => { ended = Some("timeout"); break; }
                next = lines.next_line() => match next {
                    Ok(Some(line)) => {
                        for l in stream.feed(&line) {
                            emit(AuditEvent::Line { run_id: run_id.clone(), kind: l.kind, text: clean(l.text) });
                        }
                        if stream.give_up() {
                            ended = Some("refused");
                            break;
                        }
                    }
                    _ => break,
                },
            }
        }
    }
    let status = if ended.is_some() {
        kill_tree(&mut child.0).await;
        None
    } else {
        match tokio::time::timeout(std::time::Duration::from_secs(30), child.0.wait()).await {
            Ok(Ok(s)) => Some(s),
            _ => {
                kill_tree(&mut child.0).await;
                None
            }
        }
    };
    let stderr = match stderr {
        Some(h) => tokio::time::timeout(std::time::Duration::from_secs(5), h)
            .await
            .ok()
            .and_then(Result::ok)
            .unwrap_or_default(),
        None => String::new(),
    };
    let summary = stream
        .summary()
        .map(|t| clean(t.chars().take(4000).collect::<String>()));
    let ok = ended.is_none() && status.is_some_and(|s| s.success()) && stream.succeeded();
    let failure = || clean(stream.failure(&stderr, status.and_then(|s| s.code())));
    let (outcome, error) = match ended {
        Some("cancelled") => ("cancelled", None),
        Some("refused") => ("error", Some(failure())),
        Some(_) => (
            "timeout",
            Some(format!(
                "The audit ran for {} minutes and was stopped. The notes it suggested so far are waiting for you.",
                runner::TIME_LIMIT.as_secs() / 60
            )),
        ),
        None if ok => ("ok", None),
        None => ("error", Some(failure())),
    };
    if outcome == "ok" {
        // Last audit, as when an AI app fetches the audit prompt over MCP.
        let state = app.state::<AppState>();
        let _ = state
            .store
            .record_audit(&p.connection_id, p.database.as_deref(), p.app);
    }
    drop(p);
    app.state::<AuditRuns>()
        .0
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&run_id);
    emit(AuditEvent::Done {
        run_id,
        outcome,
        suggested: stream.suggested(),
        summary,
        error,
    });
}

/// Stops a running audit and the agent's processes.
#[tauri::command]
pub fn cancel_audit(runs: State<'_, AuditRuns>, run_id: String) {
    if let Some(stop) = runs
        .0
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&run_id)
    {
        stop.cancel();
    }
}

/// Opens the system terminal in the first code source (the others added with `--add-dir`),
/// running the agent interactively with the same read-only options and the audit
/// instructions as its first message. Repositories are fetched first.
#[tauri::command]
pub async fn open_audit_in_terminal(
    app: AppHandle,
    state: State<'_, AppState>,
    agent: String,
    id: String,
    database: Option<String>,
) -> AppResult<()> {
    if agent == opencode::ID {
        return Err(
            "SayQL's built-in OpenCode runs inside SayQL only. Start the audit here instead."
                .into(),
        );
    }
    let audit = audit_dir(&app)?;
    let p = prepare(
        &state,
        &audit,
        run_dir::TERMINAL,
        &agent,
        &id,
        database.as_deref(),
        false,
        &|_, _| {},
    )
    .await?;
    let args = runner::claude_args(&p.files, &p.sources.more_dirs, false, p.model, p.payment);
    let prompt = p.dir.path().join("prompt.txt");
    std::fs::write(&prompt, &p.sources.prompt)?;
    let path_var = runner::child_path(&p.bin);
    launch_terminal(&TerminalRun {
        dir: p.dir.path(),
        folder: &p.sources.folder,
        bin: &p.bin,
        path_var: &path_var,
        args: &args,
        prompt: &prompt,
    })?;
    // The files stay: the terminal reads them after this returns. A later start of SayQL
    // removes them once they're a week old.
    let _ = p.dir.keep();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Paying with credits is off: a saved credits choice, even with a credit key, runs on the
    /// person's own sign-in.
    #[test]
    fn a_saved_credits_choice_runs_on_the_persons_own_sign_in() {
        assert_eq!(
            effective(AuditPayment::Credits, true),
            AuditPayment::Subscription
        );
        assert_eq!(
            effective(AuditPayment::Credits, false),
            AuditPayment::Subscription
        );
    }
}
