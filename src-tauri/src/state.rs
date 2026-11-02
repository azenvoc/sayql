//! Process-wide state shared by the Tauri commands.

use crate::ai::Endpoint;
use crate::ai::agent::run::{History, Reply};
use crate::db::{self, DbPool};
use crate::error::AppResult;
use crate::schema::{self, Schema};
use crate::store::agent::Note;
use crate::store::{Store, secrets};
use crate::team::ai::AiStatus;
use crate::team::{self, TeamClient, TeamConnection, TeamPool};
use crate::voice::Voice;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;
use tauri::async_runtime::JoinHandle;
use tokio_util::sync::CancellationToken;

/// Locks `m`, recovering the data if another thread panicked while holding it. Every map
/// guarded here stays valid between operations, so a panic elsewhere shouldn't take the
/// whole app down with it.
pub(crate) fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// One database on a saved connection. `database: None` is the connection's default
/// database (the one in its settings, or the server's default for the login).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Target {
    pub connection_id: String,
    pub database: Option<String>,
}

impl Target {
    pub fn new(connection_id: String, database: Option<String>) -> Self {
        Target {
            connection_id,
            database: database.filter(|d| !d.trim().is_empty()),
        }
    }

    pub fn default_of(connection_id: &str) -> Self {
        Target {
            connection_id: connection_id.to_string(),
            database: None,
        }
    }
}

/// An agent run in progress: how to reach it.
pub struct AgentRun {
    pub run_id: String,
    pub replies: tokio::sync::mpsc::UnboundedSender<Reply>,
    pub stop: CancellationToken,
}

/// A schema read from a target.
pub struct LoadedSchema {
    pub schema: Schema,
    /// Rendered once per load so the prompt prefix is byte-identical (cacheable).
    pub schema_text: String,
}

pub struct AppState {
    pub store: Store,
    pub demo_path: PathBuf,
    /// Where runners SayQL downloads are kept (`<app data>/runners`): the built-in OpenCode.
    pub runners_dir: PathBuf,
    pub http: reqwest::Client,
    /// Serializes `init` so a double call (React StrictMode in dev) can't add the demo twice.
    pub init_lock: tokio::sync::Mutex<()>,
    pools: Mutex<HashMap<Target, DbPool>>,
    schemas: Mutex<HashMap<Target, Arc<LoadedSchema>>>,
    /// When each cached schema was last confirmed current, and the database's schema
    /// fingerprint then (`None` where it has none).
    schema_checks: Mutex<HashMap<Target, (std::time::Instant, Option<String>)>>,
    /// The running AI generation per slot (one slot per editor tab).
    generations: Mutex<HashMap<String, JoinHandle<()>>>,
    /// Running queries the UI can stop, by the id it gave each run.
    queries: Mutex<HashMap<String, CancellationToken>>,
    /// The agent run per slot (one slot per editor tab).
    agents: Mutex<HashMap<String, AgentRun>>,
    /// Each slot's last finished agent conversation, for follow-ups.
    agent_histories: Mutex<HashMap<String, History>>,
    /// The team server's connections as last listed; empty when signed out.
    team_connections: Mutex<Vec<TeamConnection>>,
    /// Team AI as the server last described it; `None` when signed out or not yet asked.
    team_ai: Mutex<Option<AiStatus>>,
    /// The server said team AI is off since then: use the person's own AI until it's asked again.
    team_ai_off: Arc<AtomicBool>,
    /// The team's shared notes per team connection, and whether the server keeps them.
    pub knowledge: Arc<team::knowledge::Cache>,
    pub voice: Voice,
}

impl AppState {
    pub fn new(config_dir: PathBuf, data_dir: PathBuf) -> AppResult<Self> {
        let http = reqwest::Client::builder()
            // Keep connections to the API warm between keystrokes.
            .pool_idle_timeout(Duration::from_secs(300))
            .tcp_keepalive(Duration::from_secs(30))
            .build()
            .map_err(|e| format!("Could not set up networking: {e}"))?;
        Ok(AppState {
            store: Store::new(config_dir),
            demo_path: data_dir.join("demo.sqlite"),
            runners_dir: data_dir.join("runners"),
            voice: Voice::new(&data_dir),
            http,
            init_lock: tokio::sync::Mutex::new(()),
            pools: Mutex::new(HashMap::new()),
            schemas: Mutex::new(HashMap::new()),
            schema_checks: Mutex::new(HashMap::new()),
            generations: Mutex::new(HashMap::new()),
            queries: Mutex::new(HashMap::new()),
            agents: Mutex::new(HashMap::new()),
            agent_histories: Mutex::new(HashMap::new()),
            team_connections: Mutex::new(Vec::new()),
            team_ai: Mutex::new(None),
            team_ai_off: Arc::new(AtomicBool::new(false)),
            knowledge: Arc::default(),
        })
    }

    pub fn pool(&self, target: &Target) -> Option<DbPool> {
        lock(&self.pools).get(target).cloned()
    }

    pub fn set_pool(&self, target: Target, pool: DbPool) {
        lock(&self.pools).insert(target, pool);
    }

    /// The pool of an open target, for running queries.
    pub fn open_pool(&self, target: &Target) -> AppResult<DbPool> {
        self.pool(target)
            .ok_or_else(|| "Not connected. Open the database first.".into())
    }

    pub fn schema(&self, target: &Target) -> AppResult<Arc<LoadedSchema>> {
        lock(&self.schemas)
            .get(target)
            .cloned()
            .ok_or_else(|| "Not connected. Open the database first.".into())
    }

    pub fn set_schema(&self, target: Target, loaded: LoadedSchema) {
        lock(&self.schemas).insert(target, Arc::new(loaded));
    }

    /// Forgets every open database of a connection; the next open reconnects.
    pub fn close(&self, connection_id: &str) {
        lock(&self.pools).retain(|t, _| t.connection_id != connection_id);
        lock(&self.schemas).retain(|t, _| t.connection_id != connection_id);
    }

    /// Makes `handle` the slot's AI generation, aborting the one it replaces.
    pub fn replace_generation(&self, slot: &str, handle: JoinHandle<()>) {
        if let Some(prev) = lock(&self.generations).insert(slot.to_string(), handle) {
            prev.abort();
        }
    }

    /// A token that `cancel_query(run_id)` fires, for the query about to run.
    pub fn start_query(&self, run_id: &str) -> CancellationToken {
        let token = CancellationToken::new();
        lock(&self.queries).insert(run_id.to_string(), token.clone());
        token
    }

    pub fn finish_query(&self, run_id: &str) {
        lock(&self.queries).remove(run_id);
    }

    pub fn cancel_query(&self, run_id: &str) {
        if let Some(token) = lock(&self.queries).remove(run_id) {
            token.cancel();
        }
    }

    pub fn cancel_generation(&self, slot: &str) {
        if let Some(prev) = lock(&self.generations).remove(slot) {
            prev.abort();
        }
    }

    /// Makes `run` the slot's agent run, stopping the one it replaces.
    pub fn start_agent(&self, slot: &str, run: AgentRun) {
        if let Some(prev) = lock(&self.agents).insert(slot.to_string(), run) {
            prev.stop.cancel();
        }
    }

    /// Keeps a finished run's conversation for the slot's next follow-up, unless a newer
    /// run has taken the slot since.
    pub fn finish_agent(&self, slot: &str, run_id: &str, history: History) {
        let mut agents = lock(&self.agents);
        if agents.get(slot).is_some_and(|r| r.run_id == run_id) {
            agents.remove(slot);
            lock(&self.agent_histories).insert(slot.to_string(), history);
        }
    }

    pub fn agent_history(&self, slot: &str) -> Option<History> {
        lock(&self.agent_histories).get(slot).cloned()
    }

    pub fn stop_agent(&self, run_id: &str) {
        if let Some(run) = lock(&self.agents).values().find(|r| r.run_id == run_id) {
            run.stop.cancel();
        }
    }

    /// Passes the person's reply to a waiting run. False when the run has ended.
    pub fn reply_agent(&self, run_id: &str, reply: Reply) -> bool {
        lock(&self.agents)
            .values()
            .find(|r| r.run_id == run_id)
            .is_some_and(|r| r.replies.send(reply).is_ok())
    }

    /// The signed-in team server. Reads the keychain only when settings say there is one.
    pub fn team_client(&self) -> AppResult<Option<TeamClient>> {
        let Some(account) = self.store.settings()?.team else {
            return Ok(None);
        };
        let token = secrets::team_token().ok_or(team::SIGNED_OUT)?;
        Ok(Some(TeamClient::new(
            self.http.clone(),
            account.server,
            token,
            Some(self.store.clone()),
        )))
    }

    pub fn set_team_connections(&self, list: Vec<TeamConnection>) {
        *lock(&self.team_connections) = list;
    }

    /// A team connection by its id in the app (`team:…`).
    pub fn team_connection(&self, id: &str) -> Option<TeamConnection> {
        let server_id = id.strip_prefix(team::ID_PREFIX)?;
        lock(&self.team_connections)
            .iter()
            .find(|c| c.id == server_id)
            .cloned()
    }

    pub fn set_team_ai(&self, status: Option<AiStatus>) {
        self.team_ai_off.store(false, Ordering::Relaxed);
        *lock(&self.team_ai) = status;
    }

    /// Asks the team server about team AI and remembers the answer; `None` when signed out.
    pub async fn refresh_team_ai(&self) -> AppResult<Option<AiStatus>> {
        let Some(client) = self.team_client()? else {
            self.set_team_ai(None);
            return Ok(None);
        };
        let status = client.ai_status().await?;
        self.set_team_ai(Some(status.clone()));
        Ok(Some(status))
    }

    /// Team AI, if it applies to `connection_id`: a team connection, signed in, with team AI on.
    /// Personal connections always get `None`, and keep the person's own provider and key.
    /// `own_ready`: the person has their own AI to fall back on if the server turns team AI off.
    pub fn team_ai_route(
        &self,
        connection_id: &str,
        source: &str,
        own_ready: bool,
    ) -> Option<(Endpoint, AiStatus)> {
        // Personal connections never reach the keychain or the team server from here.
        if !connection_id.starts_with(team::ID_PREFIX) || self.team_ai_off.load(Ordering::Relaxed) {
            return None;
        }
        let status = lock(&self.team_ai).clone()?;
        let client = self.team_client().ok().flatten()?;
        team::ai::route(
            connection_id,
            &status,
            &client,
            source,
            self.team_ai_off.clone(),
            own_ready,
        )
    }

    /// The team's shared notes on a team connection whose server keeps them; `None` to use
    /// the notes saved on this computer. Personal connections never reach the keychain or the
    /// team server from here.
    pub async fn team_notes(&self, connection_id: &str) -> Option<team::knowledge::Shared> {
        if !connection_id.starts_with(team::ID_PREFIX) {
            return None;
        }
        let client = self.team_client().ok().flatten()?;
        self.knowledge.notes(&client, connection_id).await
    }

    /// The notes the Ask box sends with the schema: those about any of `tables` (qualified
    /// names), plus those about no table. On a team connection whose server keeps shared
    /// notes, the team's; else (or when the server can't be reached) the ones saved here.
    pub async fn prompt_notes(&self, target: &Target, tables: &[String]) -> Vec<Note> {
        match self.team_notes(&target.connection_id).await {
            Some(shared) => team::knowledge::about(&shared.notes, tables),
            None => self
                .store
                .notes_for(&target.connection_id, target.database.as_deref(), tables)
                .unwrap_or_default(),
        }
    }

    /// Forgets the team connections and closes their tabs' pools, after signing out.
    pub fn forget_team(&self) {
        self.set_team_ai(None);
        self.knowledge.clear();
        lock(&self.team_connections).clear();
        lock(&self.pools).retain(|t, _| !t.connection_id.starts_with(team::ID_PREFIX));
        lock(&self.schemas).retain(|t, _| !t.connection_id.starts_with(team::ID_PREFIX));
    }

    /// Fails unless `id` is a saved connection or a listed team connection.
    pub fn check_connection(&self, id: &str) -> AppResult<()> {
        if self.team_connection(id).is_some() {
            return Ok(());
        }
        self.store.connection(id).map(|_| ())
    }

    fn kind_of(&self, id: &str) -> AppResult<db::Kind> {
        if id.starts_with(team::ID_PREFIX) {
            return self
                .team_connection(id)
                .and_then(|c| c.kind())
                .ok_or_else(|| team::SIGNED_OUT.into());
        }
        Ok(self.store.connection(id)?.kind)
    }

    /// The pool for `target`, connecting if it isn't open yet.
    pub async fn target_pool(&self, target: &Target) -> AppResult<DbPool> {
        if let Some(pool) = self.pool(target) {
            return Ok(pool);
        }
        let id = &target.connection_id;
        if id.starts_with(team::ID_PREFIX) {
            let conn = self.team_connection(id).ok_or(team::SIGNED_OUT)?;
            let kind = conn.kind().ok_or(
                "Your team's server lists this connection with a database type SayQL doesn't know.",
            )?;
            let client = self.team_client()?.ok_or(team::SIGNED_OUT)?;
            let pool = DbPool::Team(TeamPool {
                client,
                id: conn.id,
                kind,
                source: team::Source::Editor,
                question: None,
            });
            self.set_pool(target.clone(), pool.clone());
            return Ok(pool);
        }
        let cfg = self.store.connection(id)?;
        let pool = match &target.database {
            None => db::connect(&cfg, secrets::password_async(id).await).await?,
            Some(database) => {
                let base = Box::pin(self.target_pool(&Target::default_of(id))).await?;
                db::open_database(&base, &cfg, secrets::password_async(id).await, database).await?
            }
        };
        self.set_pool(target.clone(), pool.clone());
        Ok(pool)
    }

    /// Reads `target`'s schema and caches it, with its prompt text, then reviews the
    /// database's notes against it (see [`crate::notes::review`]).
    pub async fn load_schema(&self, target: &Target) -> AppResult<Schema> {
        let kind = self.kind_of(&target.connection_id)?;
        let pool = self.target_pool(target).await?;
        let schema = db::introspect(&pool, kind).await?;
        let schema_text = schema::render_full(&schema);
        let version = db::schema_hash::read(&pool).await;
        self.set_schema(
            target.clone(),
            LoadedSchema {
                schema: schema.clone(),
                schema_text,
            },
        );
        lock(&self.schema_checks).insert(target.clone(), (std::time::Instant::now(), version));
        let database = self.notes_database(target);
        if let Err(e) = crate::notes::review(
            &self.store,
            &target.connection_id,
            database.as_deref(),
            &schema,
        ) {
            eprintln!("Couldn't review the notes against the schema: {e}");
        }
        Ok(schema)
    }

    /// The database `target`'s notes are kept under (see [`crate::audit::notes_database`]).
    pub fn notes_database(&self, target: &Target) -> Option<String> {
        crate::audit::notes_database(
            &self.store,
            &target.connection_id,
            target.database.as_deref(),
        )
    }

    /// Makes sure the cached schema of an open `target` is current: at most once a minute,
    /// asks the database for a cheap fingerprint of its schema and reads the schema again
    /// when it changed. Errors leave the cached schema as it is.
    pub async fn ensure_fresh(&self, target: &Target) {
        const EVERY: Duration = Duration::from_secs(60);
        let last = lock(&self.schema_checks).get(target).cloned();
        if last.as_ref().is_some_and(|(at, _)| at.elapsed() < EVERY) || self.pool(target).is_none()
        {
            return;
        }
        let Ok(pool) = self.target_pool(target).await else {
            return;
        };
        let now = db::schema_hash::read(&pool).await;
        let unchanged = match (&last, &now) {
            (Some((_, Some(before))), Some(after)) => before == after,
            // No fingerprint for this database: read the whole schema again.
            _ => false,
        };
        if unchanged {
            lock(&self.schema_checks).insert(target.clone(), (std::time::Instant::now(), now));
            return;
        }
        let _ = self.load_schema(target).await;
    }

    /// Reads the schema again now, whatever the last check: after the database said a table
    /// or column doesn't exist.
    pub async fn refresh_now(&self, target: &Target) -> AppResult<Arc<LoadedSchema>> {
        self.load_schema(target).await?;
        self.schema(target)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lock_recovers_from_a_poisoned_mutex() {
        let m = Arc::new(Mutex::new(vec![1]));
        let m2 = Arc::clone(&m);
        let _ = std::thread::spawn(move || {
            let mut g = m2.lock().unwrap();
            g.push(2);
            panic!("poison it");
        })
        .join();
        assert!(m.is_poisoned());
        let mut g = lock(&m);
        g.push(3);
        assert_eq!(*g, vec![1, 2, 3]);
    }
}
