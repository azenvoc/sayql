//! An audit run from its checks to its end, without Tauri: which coding agents can run one,
//! the checks before a run (the agent, its version, the MCP gate, who pays), the run's private
//! folder and instructions, starting Claude Code or SayQL's built-in OpenCode (downloaded on
//! first use, with its private HOME), reading its output into progress events, and opening the
//! same run in a terminal instead. The commands in commands/audit.rs pass in where runs keep
//! their files and how events reach the window, and spawn the run. See runner.rs and
//! opencode.rs for what's enforced, and sources.rs for the knowledge sources.

use super::opencode::{self, OpenCodeStream};
use super::process::{TerminalRun, Tree, kill_tree, launch_terminal, no_window, version_of};
use super::runner::run_dir::{self, RunDir};
use super::runner::{self, AgentSpec, AuditModel, AuditPayment, ClaudeStream, RunFiles, RunStream};
use super::sources::{self, NONE_YET};
use super::{Target, instructions_for, target_database};
use crate::ai::{KeyNeed, SAYQL_CREDITS, endpoint};
use crate::error::{AppError, AppResult};
use crate::state::AppState;
use crate::store::Store;
use crate::store::apps::AppGate;
use crate::store::secrets;
use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio_util::sync::CancellationToken;

/// The stderr kept for an error message.
const STDERR_MAX: usize = 16 * 1024;

/// Running audits the window can cancel, by the id it gave each run. Clones share the runs.
#[derive(Default, Clone)]
pub struct AuditRuns(Arc<Mutex<HashMap<String, CancellationToken>>>);

impl AuditRuns {
    fn insert(&self, run_id: &str, stop: CancellationToken) {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(run_id.to_string(), stop);
    }

    fn remove(&self, run_id: &str) {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(run_id);
    }

    /// Stops a running audit and the agent's processes.
    pub fn cancel(&self, run_id: &str) {
        if let Some(stop) = self.0.lock().unwrap_or_else(|e| e.into_inner()).get(run_id) {
            stop.cancel();
        }
    }
}

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

/// What a run tells the window (`audit://event`, see src/api/events.ts).
#[derive(Serialize, Clone)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum AuditEvent {
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

/// How a run's events reach the window.
pub type Emit = Arc<dyn Fn(AuditEvent) + Send + Sync>;

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
pub async fn agents(state: &AppState) -> Vec<AgentView> {
    let mut out = Vec::new();
    for spec in &runner::AGENTS {
        out.push(view(spec).await);
    }
    out.push(opencode_view(state));
    out
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

/// Who pays for audits, as Settings shows it.
pub fn payment_view(store: &Store) -> AuditPaymentView {
    let has_credit_key = secrets::api_key(SAYQL_CREDITS).is_some();
    AuditPaymentView {
        payment: effective(store.audit_payment().unwrap_or_default(), has_credit_key),
        has_credit_key,
    }
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
    let database = target_database(&state.store, &conn, database);
    let ready = sources::ready(&state.store, &conn.id, say).await?;
    let dirs: Vec<String> = ready
        .iter()
        .map(|r| r.dir.to_string_lossy().into_owned())
        .collect();
    let listed: Vec<super::Source> = ready
        .iter()
        .zip(&dirs)
        .map(|(r, dir)| super::Source {
            label: &r.label,
            kind: r.kind,
            document: r.document,
            dir,
            url: r.url.as_deref(),
            commit: r.commit.as_deref(),
        })
        .collect();
    let mut prompt = instructions_for(
        Some(&Target {
            connection_id: &conn.id,
            name: &conn.name,
            database: database.as_deref(),
        }),
        false,
        &listed,
    );
    prompt.push_str(runner::run_notes(headless));
    let sayql_mcp = crate::mcp::binary_path()?;
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

/// Removes the run folders an earlier start left behind (SayQL quit or crashed mid-run), and
/// the ones older versions left in the temp folder, in the background. `audit`: where runs
/// keep their files (`<app cache>/audit`).
pub fn clean_leftover_runs(audit: PathBuf) {
    let started = std::time::SystemTime::now();
    std::thread::spawn(move || {
        run_dir::clean_leftovers(&audit, started);
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
    audit: &Path,
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
        return Err(NONE_YET.into());
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

/// A started run, to be driven to its end (`Driver::run`) on the app's async runtime.
pub struct Driver {
    emit: Emit,
    runs: AuditRuns,
    store: Store,
    run_id: String,
    child: Tree,
    run: Run,
    stream: Box<dyn RunStream>,
    stop: CancellationToken,
}

/// Starts an audit with `agent` over the connection's code sources, fetching repositories
/// first; progress goes to `emit`. `audit`: where runs keep their files (`<app cache>/audit`),
/// or why it couldn't be found, reported when a run first needs it.
#[allow(
    clippy::too_many_arguments,
    reason = "the run's settings, each from a different place"
)]
pub async fn start(
    state: &AppState,
    runs: &AuditRuns,
    audit: AppResult<PathBuf>,
    emit: Emit,
    run_id: String,
    agent: &str,
    id: &str,
    database: Option<&str>,
) -> AppResult<Driver> {
    let say = |kind: &'static str, text: String| {
        emit(AuditEvent::Line {
            run_id: run_id.clone(),
            kind,
            text,
        });
    };
    if agent == opencode::ID {
        return start_opencode(state, runs, audit, emit.clone(), run_id, id, database).await;
    }
    let audit = audit?;
    let p = prepare(
        state,
        &audit,
        run_dir::RUNS,
        agent,
        id,
        database,
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
    runs.insert(&run_id, stop.clone());
    let mut stream = ClaudeStream::with_sources(p.sources.sources.clone());
    stream.credits = p.credits.is_some();
    let run = Run {
        app: p.spec.label,
        dir: p.dir,
        connection_id: p.sources.connection_id,
        database: p.sources.database,
        secret: p.credits.map(|c| c.key),
    };
    Ok(Driver {
        emit,
        runs: runs.clone(),
        store: state.store.clone(),
        run_id,
        child,
        run,
        stream: Box::new(stream),
        stop,
    })
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
    state: &AppState,
    runs: &AuditRuns,
    audit: AppResult<PathBuf>,
    emit: Emit,
    run_id: String,
    id: &str,
    database: Option<&str>,
) -> AppResult<Driver> {
    let say = |kind: &'static str, text: String| {
        emit(AuditEvent::Line {
            run_id: run_id.clone(),
            kind,
            text,
        });
    };
    opencode::this_platform()?;
    if let Some(why) = opencode::managed_policy() {
        return Err(why.into());
    }
    if state.store.knowledge_sources(id)?.is_empty() {
        return Err(NONE_YET.into());
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
    runs.insert(&run_id, stop.clone());
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
                            emit(AuditEvent::Download {
                                run_id: run_id.clone(),
                                received,
                                total,
                            });
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
        let run_dir = RunDir::create(&audit?, run_dir::RUNS)?;
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
        Ok((child, run, stream)) => Ok(Driver {
            emit: emit.clone(),
            runs: runs.clone(),
            store: state.store.clone(),
            run_id,
            child,
            run,
            stream: Box::new(stream),
            stop,
        }),
        Err(e) => {
            runs.remove(&run_id);
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

impl Driver {
    /// Reads the agent's output into progress events until it ends, is cancelled or runs out
    /// of time, then records a successful audit, removes the run's files and says how it ended.
    pub async fn run(self) {
        let Driver {
            emit,
            runs,
            store,
            run_id,
            mut child,
            run: p,
            mut stream,
            stop,
        } = self;
        // Nothing shown in the window carries a key.
        let key = p.secret.clone();
        let clean = |s: String| {
            let s = match key.as_deref().map(str::trim).filter(|k| k.len() >= 8) {
                Some(k) if !k.starts_with("sqlc_") => s.replace(k, "…"),
                _ => s,
            };
            runner::redact(&s, key.as_deref())
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
            let _ = store.record_audit(&p.connection_id, p.database.as_deref(), p.app);
        }
        drop(p);
        runs.remove(&run_id);
        emit(AuditEvent::Done {
            run_id,
            outcome,
            suggested: stream.suggested(),
            summary,
            error,
        });
    }
}

/// Opens the system terminal in the first code source (the others added with `--add-dir`),
/// running the agent interactively with the same read-only options and the audit
/// instructions as its first message. Repositories are fetched first.
pub async fn open_in_terminal(
    state: &AppState,
    audit: AppResult<PathBuf>,
    agent: &str,
    id: &str,
    database: Option<&str>,
) -> AppResult<()> {
    if agent == opencode::ID {
        return Err(
            "SayQL's built-in OpenCode runs inside SayQL only. Start the audit here instead."
                .into(),
        );
    }
    let audit = audit?;
    let p = prepare(
        state,
        &audit,
        run_dir::TERMINAL,
        agent,
        id,
        database,
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
