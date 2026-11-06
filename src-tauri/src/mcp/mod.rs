//! An MCP server over stdio (`sayql-mcp`), so Claude (Claude Code, Claude Desktop) can use
//! the person's saved SayQL connections. Read-only: anything that could change data is
//! refused and has to be run in the SayQL app, where the person reviews it first.
//! Three tools hand work to the app instead (see handoff.rs): a prefilled New connection
//! dialog, where the person types the password, and SQL or a saved script in a tab, which
//! they run. `export_csv` writes a read-only query's rows to a new file in Downloads.
//!
//! This file has the server, its tool router and what every tool shares (the app gate, the
//! activity log); `query`, `export`, `notes` and `prompt` hold the larger tools, and `shape`
//! what a result looks like at each "AI apps may see" level.

use crate::audit;
use crate::db::{self, Kind, QueryResult};
use crate::handoff::{self, Action, Handoff, ScriptRef};
use crate::safety::gate::{self, Refusal};
use crate::safety::mask::Masker;
use crate::safety::never_send::{self, Guard};
use crate::safety::readonly::{DEFAULT_ROWS, MAX_ROWS, QueryFailure, WRITE_REFUSED, describe_with};
use crate::schema::Schema;
use crate::state::{AppState, Target};
use crate::store::agent::{AgentAccess, AppResults};
use crate::store::apps::{Activity, AppGate, NOT_APPROVED, UNKNOWN_APP};
use crate::store::layout::Scope;
use crate::store::suggestions::Change;
use rmcp::handler::server::{router::tool::ToolRouter, wrapper::Parameters};
use rmcp::model::{
    GetPromptRequestParams, GetPromptResponse, GetPromptResult, Implementation, ListPromptsResult,
    PaginatedRequestParams, Prompt, PromptArgument, PromptMessage, Role, ServerCapabilities,
    ServerConfig,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData, RoleServer, ServerHandler, ServiceExt, tool, tool_handler, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Map, Value, json};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

mod export;
mod notes;
mod prompt;
mod query;
mod scope;
mod shape;

use prompt::audit_prompt_info;
use shape::{mark_never_send, mask_literals, refused};

/// The Tauri app identifier: where the app keeps its config and data.
const APP_ID: &str = "com.sayql.app";
/// Set by the app when it starts this server (audits): its own identifier, so a build with
/// another one (a test build) gets its own files and keychain item, never the app's.
pub(crate) const APP_ID_VAR: &str = "SAYQL_APP_ID";
/// How long a query may run before SayQL stops it. An AI app has no way to cancel one, and
/// a forgotten query would keep the database busy.
const QUERY_TIMEOUT: Duration = Duration::from_secs(120);

const INSTRUCTIONS: &str = "SayQL gives read-only access to the user's saved database connections \
(PostgreSQL, MySQL, SQLite, SQL Server). Typical flow: list_connections, then list_databases for a \
server connection, then describe_schema (pass `search` terms for large databases), then run_query (page with `offset`). \
Queries run read-only and the database itself enforces it where it can; statements that could change \
data are refused, and the user must run those in the SayQL app. Keep results small (TOP/LIMIT) and \
select only the columns you need. To hand work to the user in the SayQL app: add_connection opens a \
prefilled New connection dialog (never pass a password; the user types it in SayQL), and open_query \
opens SQL in a new SayQL tab for the user to review and run (use it for statements that change data). \
list_scripts shows the user's saved scripts; open_script opens one in SayQL. export_csv saves a read-only \
query's rows (up to 100,000) to a CSV file in the user's Downloads folder without returning them. \
describe_schema includes notes the user saved about the database (what codes mean, how tables relate). \
Use them, but the schema is current: where a note disagrees with it, follow the schema, and verify a note \
marked \"may be out of date\" (SayQL marks a note so when a table or column it names was dropped, renamed \
or changed, or its check failed) before relying on it. It also shows each table's usage from the database's own statistics (reads, writes, last write, \
and the period they cover), so you can tell a live table from an abandoned one. When you learn a lasting \
schema-level fact (a table is unused and replaced, what a status code means, how two tables relate), call \
suggest_note with the note and your evidence: notes describe the database, never row values or personal data, \
and a suggestion only becomes a note when the user accepts it in SayQL. To correct a saved note (describe_schema shows \
each with an id such as [n_1a2b3c4d]), pass its id as `replaces`; add `remove: true` to suggest deleting it. The `audit` \
prompt walks through checking a database against the code in the user's project. Some columns are marked never to send to AI \
(describe_schema shows them as [never sent to AI]): a query that could read them in any form (named, in WHERE or COUNT, \
a whole row such as row_to_json(t) or t::text, * inside a subquery, a column alias list) is refused, so name the \
columns you need and leave those out; SELECT * returns them withheld. \
The user chooses, per connection, what you may see of results (`ai_may_see` in list_connections): `off` (row \
counts and column names only), `counts` (the default: row counts and per-column summaries: type, nulls, distinct \
count, and min/max only for a table's own number or date columns and counts, sums, averages, min or max of them), \
`masked` (rows, with likely personal data replaced by tokens such as ‹email-1›, which can't be used in later \
queries; text whose source SayQL can't trace, after UNION, VALUES or JSON access, is masked too) or `all` (rows as \
they are). Below `all`, database error messages are replaced by a fixed description of the error, since they can \
quote values. Below `all`, answer with queries whose counts, sums or averages answer \
the question, and use open_query when the user wants to see rows: they see them in SayQL, you don't. \
export_csv works only where the user allows `all`. This server serves one workspace, or one group in it: \
the user's other connections and databases aren't available here, and queries that name another database or read the \
server's list of databases are refused.";

/// What `run_query` says along with a result the AI app may not see in full.
const OFF_NOTE: &str = "The user lets AI apps see only row counts and column names on this connection. \
Write a query whose row count answers the question, or use open_query to show the user the rows in SayQL.";
const COUNTS_NOTE: &str = "The user lets AI apps see counts and column summaries on this connection, not row values. \
Min and max are shown only for a table's own number and date columns and for COUNT, SUM, AVG, MIN or MAX of them. \
Write queries whose numbers answer the question (COUNT, SUM, AVG per column), or use open_query to show the user the rows in SayQL.";
/// What every tool says when the app's AI settings can't be read: nothing runs until they can.
const SETTINGS_UNREADABLE: &str = "SayQL's AI app settings couldn't be read, so SayQL refuses every request until \
they can. Ask the user to open SayQL to fix them.";
const MASKED_NOTE: &str = "Likely personal data is replaced by tokens such as ‹email-1›. They stand for real values in \
this result only; they can't be used in later queries.";
#[derive(Debug, Deserialize, JsonSchema)]
struct ConnectionParams {
    /// A connection id from list_connections.
    connection_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct SchemaParams {
    /// A connection id from list_connections.
    connection_id: String,
    /// Database on the server (from list_databases). Omit to use the connection's default.
    database: Option<String>,
    /// Space-separated words; only tables whose name or columns contain one are described.
    search: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct SuggestNoteParams {
    /// A connection id from list_connections.
    connection_id: String,
    /// Database on the server (from list_databases). Omit for the connection's default.
    database: Option<String>,
    /// The note, at most 300 characters: a schema-level fact, e.g. "orders_legacy is unused since
    /// March 2026; use orders_v2". Never values from rows or personal data.
    text: String,
    /// Why it's true, at most 600 characters: what you checked, e.g. "no references in api/ since
    /// commit a1b2c3 (2026-03-12); describe_schema usage: 0 writes since 2026-01-01".
    evidence: String,
    /// To correct a saved note that's wrong or outdated: its id from describe_schema's notes,
    /// e.g. "n_1a2b3c4d". `text` is then the corrected note.
    #[serde(default)]
    replaces: Option<String>,
    /// With `replaces`: suggest deleting that note because it no longer applies; `text` says why.
    #[serde(default)]
    remove: Option<bool>,
    /// Optional: a read-only SQL query that tests the note against the data, returning one row
    /// with one column that is true while the note holds, e.g. "SELECT count(*) = 0 FROM orders
    /// WHERE status NOT IN ('new','paid','cancelled')". SayQL runs it to keep the note honest;
    /// only true or false is kept.
    #[serde(default)]
    check: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct QueryParams {
    /// A connection id from list_connections.
    connection_id: String,
    /// Database on the server (from list_databases). Omit to use the connection's default.
    database: Option<String>,
    /// One read-only statement (SELECT, WITH, EXPLAIN, SHOW, ...), in the connection's SQL dialect.
    sql: String,
    /// Most rows to return (default 100, at most 1000).
    max_rows: Option<usize>,
    /// Rows to skip, for the next page: pass `next_offset` from the previous result.
    offset: Option<usize>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ExportParams {
    /// A connection id from list_connections.
    connection_id: String,
    /// Database on the server (from list_databases). Omit to use the connection's default.
    database: Option<String>,
    /// One read-only statement (SELECT, WITH, ...), in the connection's SQL dialect.
    sql: String,
    /// File name without folder, e.g. "orders-2026". Saved as .csv in Downloads; never overwrites.
    file_name: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ScriptsParams {
    /// Only scripts on this connection (an id from list_connections). Omit for all.
    connection_id: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ScriptParams {
    /// A script id from list_scripts.
    script_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct AddConnectionParams {
    /// Database type: postgres, mysql, sqlite or sqlserver (SQL Server and Azure SQL).
    #[serde(rename = "type")]
    kind: String,
    /// Name shown in SayQL. Optional.
    name: Option<String>,
    /// Server host name or IP (not for sqlite). For SQL Server, `host,port` also works.
    host: Option<String>,
    /// Server port (not for sqlite). Omit for the default.
    port: Option<i64>,
    /// Database to open by default (not for sqlite).
    database: Option<String>,
    /// User name (not for sqlite).
    user: Option<String>,
    /// sqlite only: path to the database file.
    file: Option<String>,
    /// Require SSL (postgres, mysql) or encrypt the connection (sqlserver).
    ssl: Option<bool>,
    /// sqlserver only: trust a self-signed server certificate.
    trust_server_certificate: Option<bool>,
    /// Anything else. Checked so a password is refused with a clear message.
    #[serde(flatten)]
    #[schemars(skip)]
    extra: Map<String, Value>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct OpenQueryParams {
    /// A connection id from list_connections.
    connection_id: String,
    /// Database on the server (from list_databases). Omit to use the connection's default.
    database: Option<String>,
    /// The SQL to put in the editor, in the connection's dialect. At most 10,000 characters.
    sql: String,
    /// Short tab title. Optional.
    title: Option<String>,
    /// The user's question this SQL answers, shown above the editor. Optional.
    question: Option<String>,
}

#[derive(Clone)]
pub struct SayqlServer {
    state: Arc<AppState>,
    /// What this server serves; see scope.rs.
    scope: Scope,
    tool_router: ToolRouter<Self>,
}

#[tool_router]
impl SayqlServer {
    pub fn new(state: AppState) -> Self {
        SayqlServer {
            state: Arc::new(state),
            scope: Scope::default(),
            tool_router: Self::tool_router(),
        }
    }

    /// The server limited to a workspace, or one group in it.
    pub fn with_scope(mut self, scope: Scope) -> Self {
        self.scope = scope;
        self
    }

    #[tool(description = "List the database connections saved in SayQL (no passwords).")]
    async fn list_connections(&self, ctx: RequestContext<RoleServer>) -> Result<String, String> {
        let app = self.gate(&ctx, "list_connections")?;
        let result = self.connections();
        self.log_call(&app, "list_connections", "", "", &result);
        result
    }

    #[tool(
        description = "List the databases on a connection's server, and which one it opens by default."
    )]
    async fn list_databases(
        &self,
        Parameters(p): Parameters<ConnectionParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, String> {
        let app = self.gate(&ctx, "list_databases")?;
        let id = p.connection_id.clone();
        let result = self.databases(p).await;
        self.log_call(&app, "list_databases", &id, "", &result);
        result
    }

    #[tool(
        description = "Describe tables and columns (types, primary keys, foreign keys, enum values, row estimates). \
                          For large databases pass `search` words, e.g. \"customer order invoice\"."
    )]
    async fn describe_schema(
        &self,
        Parameters(p): Parameters<SchemaParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, String> {
        let app = self.gate(&ctx, "describe_schema")?;
        let id = p.connection_id.clone();
        let result = self.describe(p).await;
        self.log_call(&app, "describe_schema", &id, "", &result);
        result
    }

    #[tool(
        description = "Suggest a note about a database for the user to approve: a lasting schema-level fact that \
                          helps later questions (a table is unused and replaced, what a code means, how tables \
                          relate), with the evidence you checked. Never row values or personal data: notes \
                          that look like values are refused. To correct a saved note, pass its id from \
                          describe_schema as `replaces`; add `remove: true` to suggest deleting it (`text` says \
                          why). When the note can be tested against the data, add `check`: read-only SQL \
                          returning one row and one column, true while the note holds. It waits in SayQL until the user accepts it; accepted notes appear in \
                          describe_schema for every later session and AI app."
    )]
    async fn suggest_note(
        &self,
        Parameters(p): Parameters<SuggestNoteParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, String> {
        let app = self.gate(&ctx, "suggest_note")?;
        self.suggest(&app, p)
    }

    #[tool(
        description = "Run one read-only SQL statement. Returns what the user lets AI apps see on this \
                          connection (`ai_may_see` in list_connections): row counts, column summaries, masked \
                          rows or rows as JSON. Statements that could change data are refused. Rows are paged: \
                          when `next_offset` is set, call again with it as `offset` for more."
    )]
    async fn run_query(
        &self,
        Parameters(p): Parameters<QueryParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, String> {
        let app = self.gate(&ctx, "run_query")?;
        let mut log = self.activity(&app, "run_query", &p.connection_id, &p.sql);
        let result = self.query(p, &mut log).await;
        self.record(log, &result);
        result
    }

    #[tool(
        description = "Run one read-only SQL statement and save all its rows (up to 100,000) to a new CSV \
                          file in the user's Downloads folder. Returns the file path, columns and row count, \
                          not the rows. Only on connections where the user lets AI apps see all rows. \
                          Statements that could change data are refused."
    )]
    async fn export_csv(
        &self,
        Parameters(p): Parameters<ExportParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, String> {
        let app = self.gate(&ctx, "export_csv")?;
        let mut log = self.activity(&app, "export_csv", &p.connection_id, &p.sql);
        let result = self.export(p, &mut log).await;
        self.record(log, &result);
        result
    }

    #[tool(
        description = "List the SQL scripts the user saved in SayQL, with their SQL. Read-only ones can be run \
                          with run_query; open_script opens one in SayQL for the user."
    )]
    async fn list_scripts(
        &self,
        Parameters(p): Parameters<ScriptsParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, String> {
        let app = self.gate(&ctx, "list_scripts")?;
        let result = self.scripts(p.connection_id.as_deref());
        self.log_call(
            &app,
            "list_scripts",
            p.connection_id.as_deref().unwrap_or(""),
            "",
            &result,
        );
        result
    }

    #[tool(
        description = "Open one of the user's saved scripts (from list_scripts) in SayQL, without running it. \
                          The user reviews it and clicks Run."
    )]
    async fn open_script(
        &self,
        Parameters(p): Parameters<ScriptParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, String> {
        let app = self.gate(&ctx, "open_script")?;
        let result = async {
            let script_id = handoff::script_id(&p.script_id)?;
            let script = self
                .state
                .store
                .scripts()
                .map_err(|e| e.to_string())?
                .into_iter()
                .find(|s| s.id == script_id && self.script_in_reach(s).unwrap_or(false))
                .ok_or_else(|| {
                    format!("No saved script has id {script_id:?}; use one from list_scripts.")
                })?;
            self.hand_off(Action::OpenScript(ScriptRef { script_id }), &ctx)?;
            Ok::<_, String>((
                script.connection_id,
                format!(
                    "Opened the script {:?} in SayQL. Nothing ran: the user reviews it and clicks Run.",
                    script.name
                ),
            ))
        }
        .await;
        let id = result
            .as_ref()
            .map(|(id, _)| id.clone())
            .unwrap_or_default();
        let result = result.map(|(_, m)| m);
        self.log_call(&app, "open_script", &id, "", &result);
        result
    }

    #[tool(
        description = "Open SayQL's New connection dialog with these details filled in, for the user to finish. \
                          Never pass a password or any other secret: it is refused. The user types the password \
                          in SayQL, tests the connection and saves it. Returns right away; call list_connections \
                          later to use the new connection."
    )]
    async fn add_connection(
        &self,
        Parameters(p): Parameters<AddConnectionParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, String> {
        let app = self.gate(&ctx, "add_connection")?;
        let result = connection_draft(p)
            .map(|draft| handoff::ConnectionDraft {
                workspace: Some(self.scope.workspace.clone()),
                group: self.scope.group.clone(),
                ..draft
            })
            .and_then(|draft| self.hand_off(Action::NewConnection(draft), &ctx))
            .map(|_| {
                "Opened SayQL's New connection dialog with these details. The user enters the password there, \
                 tests the connection and saves it."
                    .to_string()
            });
        self.log_call(&app, "add_connection", "", "", &result);
        result
    }

    #[tool(
        description = "Open SQL in a new SayQL tab on a saved connection, without running it. The user reviews it \
                          and clicks Run; SayQL asks before anything that changes data. Use this for statements \
                          run_query refuses. The results stay in SayQL and are not returned."
    )]
    async fn open_query(
        &self,
        Parameters(p): Parameters<OpenQueryParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, String> {
        let app = self.gate(&ctx, "open_query")?;
        let result = (|| {
            let draft = handoff::query_draft(
                &p.connection_id,
                p.database.as_deref(),
                &p.sql,
                p.title.as_deref(),
                p.question.as_deref(),
            )?;
            let conn = self.query_connection(&draft.connection_id, draft.database.as_deref())?;
            self.hand_off(Action::OpenQuery(draft), &ctx)?;
            Ok(format!(
                "Opened a new SayQL tab on {:?} with the SQL. Nothing ran: the user reviews it and clicks Run.",
                conn.name
            ))
        })();
        // The SQL an app put in front of the user (it may change data) is in the log too.
        self.log_call(&app, "open_query", &p.connection_id, &p.sql, &result);
        result
    }
}

/// The New connection fields from `add_connection`'s arguments; refuses passwords.
fn connection_draft(p: AddConnectionParams) -> Result<handoff::ConnectionDraft, String> {
    if p.extra.keys().any(|k| handoff::is_secret_field(k)) {
        return Err(handoff::secret_refused());
    }
    if let Some(k) = p.extra.keys().next() {
        return Err(format!("Unknown field `{k}`."));
    }
    let get = |k: &str| match k {
        "type" => Some(p.kind.clone()),
        "name" => p.name.clone(),
        "host" => p.host.clone(),
        "database" => p.database.clone(),
        "user" => p.user.clone(),
        "file" => p.file.clone(),
        _ => None,
    };
    let flag = |k: &str| {
        Ok(if k == "ssl" {
            p.ssl
        } else {
            p.trust_server_certificate
        })
    };
    handoff::connection_draft(get, p.port, flag)
}

/// The AI app's name for the activity log and the block list, from the name it reported when it
/// connected. Nothing verifies it: an app can report any name, so blocking by name is a
/// convenience, not a security boundary. The display `title` an app sends is ignored.
fn client_label(info: &Implementation) -> Option<String> {
    known_client(info)
        .map(str::to_string)
        .or_else(|| handoff::source(Some(&info.name)))
}

fn known_client(info: &Implementation) -> Option<&'static str> {
    let name = info.name.to_ascii_lowercase();
    if name.contains("claude-code") {
        Some("Claude Code")
    } else if name.starts_with("claude") {
        Some("Claude")
    } else if name.contains("cursor") {
        Some("Cursor")
    } else if name == "opencode" {
        // OpenCode's MCP client name (packages/opencode/src/mcp/index.ts), also SayQL's built-in
        // audit runner.
        Some("OpenCode")
    } else {
        None
    }
}

/// Who a hand-off in SayQL says it is from. A name SayQL doesn't recognise is shown as the app
/// reported it, marked unverified, so an app can't present itself as "SayQL Support".
fn handoff_label(info: &Implementation) -> Option<String> {
    match known_client(info) {
        Some(k) => handoff::source(Some(k)),
        None => {
            let name: String = info
                .name
                .chars()
                .filter(|c| c.is_alphanumeric() || " -_.".contains(*c))
                .take(24)
                .collect();
            let name = name.trim();
            let name = if name.is_empty() { "an AI app" } else { name };
            handoff::source(Some(&format!("{name} (unverified)")))
        }
    }
}

impl SayqlServer {
    fn connections(&self) -> Result<String, String> {
        let layout = self.layout()?;
        let mut list = Vec::new();
        for c in self.state.store.connections().map_err(|e| e.to_string())? {
            let Some(limit) = layout.reach(&self.scope, &c.id) else {
                continue;
            };
            let place = if c.kind == Kind::Sqlite {
                c.file.clone()
            } else {
                c.host.clone()
            };
            let may_see = self.access(&c.id)?.apps;
            // Never a default database this connection can't use here.
            let default_database = if limit.is_open() || limit.allows(&c.database) {
                c.database.clone()
            } else {
                limit.first().unwrap_or_default().to_string()
            };
            let mut item = serde_json::Map::new();
            item.insert("id".into(), json!(c.id));
            item.insert("name".into(), json!(c.name));
            item.insert("kind".into(), json!(c.kind));
            item.insert("dialect".into(), json!(c.kind.dialect()));
            item.insert("location".into(), json!(place));
            item.insert("default_database".into(), json!(default_database));
            item.insert("ai_may_see".into(), json!(may_see));
            if self.scope.workspace != crate::store::layout::PERSONAL
                && let Some(w) = layout
                    .workspaces
                    .iter()
                    .find(|w| w.id == self.scope.workspace)
            {
                item.insert("workspace".into(), json!(w.name));
            }
            if let Some(group) = layout.group_name_of(&c.id) {
                item.insert("group".into(), json!(group));
            }
            list.push(item);
        }
        Ok(json!(list).to_string())
    }

    /// The saved scripts, with as much of their SQL as the connection lets AI apps see: all of
    /// it at `all`, with string literals masked at `counts` and `masked` (they can hold values),
    /// and none at `off` (names only).
    fn scripts(&self, connection_id: Option<&str>) -> Result<String, String> {
        let connections = self.state.store.connections().map_err(|e| e.to_string())?;
        let mut list = Vec::new();
        for s in self.state.store.scripts().map_err(|e| e.to_string())? {
            if connection_id.is_some_and(|id| s.connection_id != id) || !self.script_in_reach(&s)? {
                continue;
            }
            let saved = connections.iter().find(|c| c.id == s.connection_id);
            let connection = saved.map(|c| c.name.clone());
            let level = self.access(&s.connection_id)?.apps;
            let mut item = json!({ "id": s.id, "name": s.name, "connection_id": s.connection_id,
                                   "connection": connection, "database": s.database });
            match level {
                AppResults::All => {
                    item["question"] = json!(s.question);
                    item["sql"] = json!(s.sql);
                }
                AppResults::Counts | AppResults::Masked => {
                    item["sql"] = json!(mask_literals(&s.sql, saved.map(|c| c.kind)));
                    item["note"] = json!(
                        "String literals and comments are masked: they can hold values from the data."
                    );
                }
                AppResults::Off => {
                    item["note"] = json!(
                        "The user lets AI apps see only row counts and column names on this connection, so the SQL isn't shown. open_script opens it for the user."
                    );
                }
            }
            list.push(item);
        }
        Ok(json!(list).to_string())
    }

    /// The connection `open_query` puts SQL on, when it's saved and in reach.
    fn query_connection(
        &self,
        connection_id: &str,
        database: Option<&str>,
    ) -> Result<db::ConnConfig, String> {
        let unknown = || {
            format!("No saved connection has id {connection_id:?}; use one from list_connections.")
        };
        let conn = self
            .state
            .store
            .connection(connection_id)
            .map_err(|_| unknown())?;
        let limit = self.limit_for(connection_id).map_err(|e| {
            if e == SETTINGS_UNREADABLE {
                e
            } else {
                unknown()
            }
        })?;
        if let Some(database) = database
            && conn.kind != Kind::Sqlite
            && !limit.allows(database)
        {
            return Err(scope::no_database(database));
        }
        Ok(conn)
    }

    /// Whether this server may show a saved script: its connection is saved and in reach, and
    /// its database isn't outside the limit. A script whose connection was deleted has no
    /// workspace to be in, so it isn't shown.
    fn script_in_reach(&self, s: &crate::store::Script) -> Result<bool, String> {
        match self.limit_for(&s.connection_id) {
            Ok(limit) => Ok(s.database.as_deref().is_none_or(|d| limit.allows(d))),
            Err(e) if e == SETTINGS_UNREADABLE => Err(e),
            Err(_) => Ok(false),
        }
    }

    async fn databases(&self, p: ConnectionParams) -> Result<String, String> {
        let limit = self.limit_for(&p.connection_id)?;
        let pool = self
            .state
            .target_pool(&Target::default_of(&p.connection_id))
            .await
            .map_err(|e| e.to_string())?;
        match db::list_databases(&pool).await.map_err(|e| e.to_string())? {
            Some(mut list) => {
                list.databases.retain(|d| limit.allows(d));
                if list.current.as_deref().is_some_and(|c| !limit.allows(c)) {
                    list.current = list.databases.first().cloned();
                }
                Ok(json!(list).to_string())
            }
            None => Ok("This is a SQLite file: it is a single database; omit `database`.".into()),
        }
    }

    async fn describe(&self, mut p: SchemaParams) -> Result<String, String> {
        p.database = self
            .target_in_reach(&p.connection_id, p.database)
            .await?
            .database;
        // With their short ids, so an app can suggest replacing or removing one.
        let notes = self.state.store.notes_text_with_refs(
            &p.connection_id,
            self.notes_database(&p.connection_id, p.database.as_deref())
                .as_deref(),
        );
        let target = Target::new(p.connection_id.clone(), p.database.clone());
        let never = never_send::entries(&self.access(&p.connection_id)?.never_send);
        let schema = self.schema(&p.connection_id, p.database).await?;
        let mut schema = never_send::schema_for_ai(&schema, &never).into_owned();
        mark_never_send(&mut schema, &never);
        // Counts and dates from the database's statistics; none when it keeps none or they can't be read.
        let usage = match self.state.target_pool(&target).await {
            Ok(pool) => db::usage::read(&pool).await,
            Err(_) => db::usage::Stats::None,
        };
        Ok(describe_with(&schema, p.search.as_deref(), &usage) + &notes)
    }

    /// The calling AI app's name, refused when the user blocked it or, when they only let in
    /// apps they approved, hasn't approved it (and the attempt logged).
    fn gate(&self, ctx: &RequestContext<RoleServer>, tool: &str) -> Result<String, String> {
        let app = ctx
            .peer
            .peer_info()
            .and_then(|i| client_label(&i.client_info))
            .unwrap_or_else(|| UNKNOWN_APP.into());
        self.gate_app(app, tool)
    }

    fn gate_app(&self, app: String, tool: &str) -> Result<String, String> {
        let refuse = |detail: &str| {
            self.write_log(Activity {
                status: "refused".into(),
                detail: Some(detail.into()),
                ..Activity::new(&app, tool)
            });
        };
        // Settings that can't be read fail closed: nothing runs until the app repairs them.
        let gate = self
            .state
            .store
            .agent_access_readable()
            .and_then(|_| self.state.store.app_gate(&app));
        match gate {
            Err(_) => {
                refuse("settings unreadable");
                Err(SETTINGS_UNREADABLE.into())
            }
            Ok(AppGate::Blocked) => {
                refuse("app blocked");
                Err(format!(
                    "The user has blocked {app} from using SayQL. They can allow it again in SayQL under Settings → AI apps → Access."
                ))
            }
            Ok(AppGate::NotApproved) => {
                refuse(NOT_APPROVED);
                Err(format!(
                    "SayQL only lets in AI apps the user has approved. Ask them to approve {app} in SayQL under Settings → AI apps → Access."
                ))
            }
            Ok(AppGate::Allowed) => Ok(app),
        }
    }

    /// Logs a call that isn't a query (reads, hand-offs): the tool and outcome, no data.
    fn log_call(
        &self,
        app: &str,
        tool: &str,
        connection_id: &str,
        sql: &str,
        result: &Result<String, String>,
    ) {
        let mut log = self.activity(app, tool, connection_id, sql);
        if result.is_err() {
            log.status = "error".into();
        }
        self.write_log(log);
    }

    /// A log entry for a query or export on `connection_id`.
    fn activity(&self, app: &str, tool: &str, connection_id: &str, sql: &str) -> Activity {
        let connection = self
            .state
            .store
            .connection(connection_id)
            .map(|c| c.name)
            .unwrap_or_default();
        Activity {
            connection_id: connection_id.to_string(),
            connection,
            sql: sql.to_string(),
            ..Activity::new(app, tool)
        }
    }

    /// Writes the entry with the outcome. A log that can't be written doesn't fail the call.
    /// Query errors arrive with `detail` already set to their safe form (see `failed`); other
    /// errors (connecting, SayQL's own) hold no row values.
    fn record(&self, mut log: Activity, result: &Result<String, String>) {
        if let Err(e) = result
            && log.status == "ok"
        {
            log.status = "error".into();
            if log.detail.is_none() {
                log.detail = Some(e.chars().take(160).collect());
            }
        }
        self.write_log(log);
    }

    /// The connection's agent and AI app settings, or the defaults when none are saved (AI apps
    /// then see counts and summaries). Settings that can't be read are an error, never the
    /// defaults.
    fn access(&self, connection_id: &str) -> Result<AgentAccess, String> {
        self.state
            .store
            .agent_access(connection_id)
            .map_err(|_| SETTINGS_UNREADABLE.to_string())
    }

    /// Opens `action` in the SayQL app through a one-time `sayql://` link.
    fn hand_off(&self, action: Action, ctx: &RequestContext<RoleServer>) -> Result<(), String> {
        let source = ctx
            .peer
            .peer_info()
            .and_then(|i| handoff_label(&i.client_info));
        let token = handoff::issue_token(self.state.store.dir())?;
        let link = handoff::to_url(&Handoff { source, action }, &token)?;
        handoff::open_in_app(&link)
    }

    /// Read fresh on every call: the server lives as long as the AI app's session, and a schema
    /// cached at the first call would miss tables and columns added since.
    async fn schema(
        &self,
        connection_id: &str,
        database: Option<String>,
    ) -> Result<Schema, String> {
        let target = Target::new(connection_id.to_string(), database);
        self.state
            .load_schema(&target)
            .await
            .map_err(|e| e.to_string())
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for SayqlServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_prompts()
                .build(),
        )
        .with_server_info(Implementation::new("sayql", env!("CARGO_PKG_VERSION")))
        .with_instructions(INSTRUCTIONS)
    }

    /// One prompt, `audit` (`/mcp__sayql__audit` in Claude Code). Listing it is open to any
    /// app, like the tool list; fetching it goes through the gate.
    async fn list_prompts(
        &self,
        _request: Option<PaginatedRequestParams>,
        _ctx: RequestContext<RoleServer>,
    ) -> Result<ListPromptsResult, ErrorData> {
        Ok(ListPromptsResult::with_all_items(vec![audit_prompt_info()]))
    }

    async fn get_prompt(
        &self,
        request: GetPromptRequestParams,
        ctx: RequestContext<RoleServer>,
    ) -> Result<GetPromptResponse, ErrorData> {
        if request.name != audit::PROMPT {
            return Err(ErrorData::invalid_params(
                format!(
                    "SayQL has no prompt called {:?}; its prompt is \"audit\".",
                    request.name
                ),
                None,
            ));
        }
        let app = self
            .gate(&ctx, audit::PROMPT)
            .map_err(|e| ErrorData::invalid_request(e, None))?;
        let text = self
            .audit_prompt(&app, request.arguments.as_ref())
            .map_err(|e| ErrorData::invalid_params(e, None))?;
        Ok(
            GetPromptResult::new(vec![PromptMessage::new_text(Role::User, text)])
                .with_description(audit::DESCRIPTION)
                .into(),
        )
    }
}

/// Where the SayQL app keeps its files; `SAYQL_CONFIG_DIR` overrides (for tests).
fn app_dirs() -> Result<(PathBuf, PathBuf), String> {
    if let Ok(dir) = std::env::var("SAYQL_CONFIG_DIR") {
        let dir = PathBuf::from(dir);
        return Ok((dir.clone(), dir));
    }
    let id = app_id(std::env::var(APP_ID_VAR).ok());
    let config = dirs::config_dir()
        .ok_or("Could not find the user's config directory")?
        .join(&id);
    let data = dirs::data_dir()
        .ok_or("Could not find the user's data directory")?
        .join(&id);
    Ok((config, data))
}

/// The identifier of the app this server serves: `from_env` ([`APP_ID_VAR`]) when it's a plain
/// identifier (it names folders), else the app's.
fn app_id(from_env: Option<String>) -> String {
    from_env
        .filter(|id| {
            !id.is_empty()
                && !id.starts_with('.')
                && id
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
        })
        .unwrap_or_else(|| APP_ID.to_string())
}

/// The variable that tells a server started by the app which workspace to serve.
pub(crate) const WORKSPACE_VAR: &str = scope::WORKSPACE_VAR;

/// What a process that starts this server passes it: this app's identifier ([`APP_ID_VAR`]).
pub(crate) fn launch_env() -> (&'static str, &'static str) {
    (APP_ID_VAR, crate::store::secrets::service())
}

/// Where the `sayql-mcp` binary is: next to the app's own executable.
pub(crate) fn binary_path() -> crate::error::AppResult<PathBuf> {
    let exe = std::env::current_exe()?;
    let name = if cfg!(windows) {
        "sayql-mcp.exe"
    } else {
        "sayql-mcp"
    };
    Ok(exe.with_file_name(name))
}

/// Runs the server on stdin/stdout until the client disconnects.
pub async fn serve_stdio() -> Result<(), Box<dyn std::error::Error>> {
    let (config_dir, data_dir) = app_dirs()?;
    // The same app's keychain item as its files.
    crate::store::secrets::set_service(&app_id(std::env::var(APP_ID_VAR).ok()));
    let mut state = AppState::new(config_dir, data_dir)?;
    // The app owns its files: this process never moves or replaces one it can't read.
    state.store = state.store.without_repair();
    // Which workspace is a choice made once, at setup, so a mistake stops here with what to do.
    let args: Vec<String> = std::env::args().skip(1).collect();
    let scope = scope::scope_from(&args, |k| std::env::var(k).ok())?;
    scope::check_scope(&state.store.layout()?, &scope)?;
    let server = SayqlServer::new(state).with_scope(scope);
    server
        .serve(rmcp::transport::stdio())
        .await?
        .waiting()
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests;
