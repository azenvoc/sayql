//! "Run audit": the window's commands for the person's coding agents, a connection's knowledge
//! sources (local folders, git repositories, files and web pages), and audit runs, whose
//! progress arrives as `audit://event`. Thin: the file and folder pickers are here, the rest is
//! in src/audit/ (run.rs for runs, sources.rs for sources).

use crate::audit::knowledge;
use crate::audit::run::{self, AgentView, AuditEvent, AuditPaymentView, Emit};
use crate::audit::runner::{AuditModel, AuditPayment};
use crate::audit::sources;
use crate::error::{AppError, AppResult};
use crate::state::AppState;
use crate::store::code_sources::KnowledgeSource;
use std::path::PathBuf;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager, State};

pub use crate::audit::run::AuditRuns;

/// Must match AUDIT_EVENT in src/api/events.ts.
const EVENT: &str = "audit://event";

/// Sends a run's events to the window as `audit://event`.
fn emitter(app: &AppHandle) -> Emit {
    let app = app.clone();
    Arc::new(move |e: AuditEvent| {
        let _ = app.emit(EVENT, e);
    })
}

/// The coding agents SayQL knows, whether each is installed, and whether it can run an audit;
/// SayQL's built-in OpenCode last.
#[tauri::command]
pub async fn audit_agents(state: State<'_, AppState>) -> AppResult<Vec<AgentView>> {
    Ok(run::agents(state.inner()).await)
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
    sources::add_folder(&state.store, &id, path).await.map(Some)
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
    sources::add_repo(&state.store, &id, &url, &branch, &subdir, token, token_user).await
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
    let mut paths: Vec<PathBuf> = Vec::new();
    for p in picked {
        paths.push(
            p.into_path()
                .map_err(|e| AppError::User(format!("Could not use that file: {e}")))?,
        );
    }
    sources::add_files(&state.store, &id, paths).await.map(Some)
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
    sources::add_url(&state.store, &id, &url, crawl).await
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
    sources::refresh(&state.store, &id, &source_id).await
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
#[tauri::command]
pub fn audit_payment(state: State<'_, AppState>) -> AppResult<AuditPaymentView> {
    Ok(run::payment_view(&state.store))
}

/// Remembers who pays for audits.
#[tauri::command]
pub fn set_audit_payment(state: State<'_, AppState>, payment: AuditPayment) -> AppResult<()> {
    state.store.set_audit_payment(payment)
}

/// Where runs keep their files: `<app cache>/audit`, this user's alone (see
/// [`crate::audit::runner::run_dir::RunDir`]), never the shared temp folder.
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
    if let Ok(dir) = audit_dir(app) {
        run::clean_leftover_runs(dir);
    }
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
    let driver = run::start(
        &state,
        &runs,
        audit_dir(&app),
        emitter(&app),
        run_id,
        &agent,
        &id,
        database.as_deref(),
    )
    .await?;
    tauri::async_runtime::spawn(driver.run());
    Ok(())
}

/// Stops a running audit and the agent's processes.
#[tauri::command]
pub fn cancel_audit(runs: State<'_, AuditRuns>, run_id: String) {
    runs.cancel(&run_id);
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
    run::open_in_terminal(&state, audit_dir(&app), &agent, &id, database.as_deref()).await
}
