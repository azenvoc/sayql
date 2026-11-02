//! "Run audit": finds the person's coding agents, runs one read-only over a connection's
//! knowledge sources (local folders, git repositories, files and web pages, each fetched or
//! read again first) with the audit instructions,
//! streams its progress to the window as `audit://event`, and opens the same run in a terminal
//! instead. See src/audit_runner.rs for what's enforced, src/repos.rs for git, and
//! src/knowledge.rs for files and web pages.

use crate::agent_process::{TerminalRun, kill_tree, launch_terminal, no_window, version_of};
use crate::ai::{SAYQL_CREDITS, endpoint};
use crate::audit::{self, Target};
use crate::audit_runner::{
    self as runner, AgentSpec, AuditModel, AuditPayment, ClaudeStream, RunFiles,
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
    }
}

/// The coding agents SayQL knows, whether each is installed, and whether it can run an audit.
#[tauri::command]
pub async fn audit_agents() -> AppResult<Vec<AgentView>> {
    let mut out = Vec::new();
    for spec in &runner::AGENTS {
        out.push(view(spec).await);
    }
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
    let path = path.canonicalize().unwrap_or(path);
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
#[allow(clippy::too_many_arguments)]
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
        let path = path.canonicalize().unwrap_or(path);
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
                match dir.canonicalize() {
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
                match (&commit, dir.canonicalize()) {
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
                match (&version, dir.canonicalize()) {
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
    /// The first code source: the agent's working directory.
    folder: PathBuf,
    /// The other code sources, given as `--add-dir`.
    more_dirs: Vec<PathBuf>,
    /// Every source by label, for the progress lines.
    sources: Vec<(String, PathBuf)>,
    connection_id: String,
    database: Option<String>,
    prompt: String,
    files: RunFiles,
    dir: PathBuf,
    model: AuditModel,
    payment: AuditPayment,
    /// Set when the run is paid with SayQL credits.
    credits: Option<Credits>,
}

async fn prepare(
    state: &AppState,
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
    match state.store.app_gate(spec.label)? {
        AppGate::Allowed => {}
        AppGate::Blocked => {
            return Err(format!(
                "{} is blocked from using SayQL. Allow it in Settings → AI apps → Activity first.",
                spec.label
            )
            .into());
        }
        AppGate::NotApproved => {
            return Err(format!(
                "Only approved apps may use SayQL, and {} isn't approved. Approve it in Settings → AI apps first.",
                spec.label
            )
            .into());
        }
    }
    let conn = state.store.connection(id)?;
    let database = audit::target_database(&state.store, &conn, database);
    // Fetched last, once everything else checks out.
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
    let dir = std::env::temp_dir()
        .join("sayql-audit")
        .join(uuid::Uuid::new_v4().to_string());
    std::fs::create_dir_all(&dir)?;
    let files = RunFiles {
        mcp_config: dir.join("mcp.json"),
        settings: dir.join("settings.json"),
    };
    std::fs::write(&files.mcp_config, runner::mcp_config_json(&sayql_mcp))?;
    std::fs::write(&files.settings, runner::SETTINGS_JSON)?;
    let sources: Vec<(String, PathBuf)> = ready.into_iter().map(|r| (r.label, r.dir)).collect();
    Ok(Prepared {
        spec,
        bin,
        folder: sources[0].1.clone(),
        more_dirs: sources[1..].iter().map(|(_, d)| d.clone()).collect(),
        sources,
        connection_id: conn.id,
        database,
        prompt,
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
    let p = prepare(&state, &agent, &id, database.as_deref(), true, &say).await?;
    let mut cmd = tokio::process::Command::new(&p.bin);
    cmd.args(runner::claude_args(
        &p.files,
        &p.more_dirs,
        true,
        p.model,
        p.payment,
    ))
    .current_dir(&p.folder)
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
    let mut child = cmd.spawn().map_err(|e| {
        let _ = std::fs::remove_dir_all(&p.dir);
        AppError::User(format!("Could not start {}: {e}", p.spec.label))
    })?;
    // The prompt goes on stdin: nothing about the run is on the command line but file paths.
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(p.prompt.as_bytes()).await?;
        drop(stdin);
    }
    let stop = CancellationToken::new();
    runs.0
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(run_id.clone(), stop.clone());
    tauri::async_runtime::spawn(drive(app, run_id, child, p, stop));
    Ok(())
}

async fn drive(
    app: AppHandle,
    run_id: String,
    mut child: tokio::process::Child,
    p: Prepared,
    stop: CancellationToken,
) {
    // Nothing shown in the window carries the credit key.
    let key = p.credits.as_ref().map(|c| c.key.clone());
    let clean = |s: String| runner::redact(&s, key.as_deref());
    let emit = |e: AuditEvent| {
        let _ = app.emit(EVENT, e);
    };
    let stderr = child.stderr.take().map(|mut err| {
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
    let mut stream = ClaudeStream::with_sources(p.sources.clone());
    stream.credits = p.credits.is_some();
    let mut lines = child.stdout.take().map(|o| BufReader::new(o).lines());
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
                        // Claude Code retries a refused key for minutes; a credit key that was
                        // refused once won't be accepted on a retry.
                        if stream.credits && stream.api_error.as_deref() == Some("authentication_failed") {
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
        kill_tree(&mut child).await;
        None
    } else {
        match tokio::time::timeout(std::time::Duration::from_secs(30), child.wait()).await {
            Ok(Ok(s)) => Some(s),
            _ => {
                kill_tree(&mut child).await;
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
        .ending
        .as_ref()
        .filter(|e| !e.is_error && !e.text.trim().is_empty())
        .map(|e| clean(e.text.chars().take(4000).collect::<String>()));
    let ok = ended.is_none()
        && status.is_some_and(|s| s.success())
        && stream.ending.as_ref().is_some_and(|e| !e.is_error);
    let failure = || {
        clean(runner::failure(
            stream.ending.as_ref(),
            &stderr,
            stream.api_error.as_deref(),
            status.and_then(|s| s.code()),
            p.payment,
        ))
    };
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
            .record_audit(&p.connection_id, p.database.as_deref(), p.spec.label);
    }
    let _ = std::fs::remove_dir_all(&p.dir);
    app.state::<AuditRuns>()
        .0
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&run_id);
    emit(AuditEvent::Done {
        run_id,
        outcome,
        suggested: stream.suggested,
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
    state: State<'_, AppState>,
    agent: String,
    id: String,
    database: Option<String>,
) -> AppResult<()> {
    let p = prepare(&state, &agent, &id, database.as_deref(), false, &|_, _| {}).await?;
    let args = runner::claude_args(&p.files, &p.more_dirs, false, p.model, p.payment);
    let prompt = p.dir.join("prompt.txt");
    std::fs::write(&prompt, &p.prompt)?;
    let path_var = runner::child_path(&p.bin);
    // The files stay: the terminal reads them after this returns (they're in the temp folder).
    launch_terminal(&TerminalRun {
        dir: &p.dir,
        folder: &p.folder,
        bin: &p.bin,
        path_var: &path_var,
        args: &args,
        prompt: &prompt,
    })
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
