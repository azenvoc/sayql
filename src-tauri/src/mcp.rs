//! An MCP server over stdio (`sayql-mcp`), so Claude (Claude Code, Claude Desktop) can use
//! the person's saved SayQL connections. Read-only: anything that could change data is
//! refused and has to be run in the SayQL app, where the person reviews it first.
//! Three tools hand work to the app instead (see handoff.rs): a prefilled New connection
//! dialog, where the person types the password, and SQL or a saved script in a tab, which
//! they run. `export_csv` writes a read-only query's rows to a new file in Downloads.

use crate::ai::agent::mask::Masker;
use crate::audit;
use crate::db::{self, Kind, QueryResult};
use crate::handoff::{self, Action, Handoff, ScriptRef};
use crate::never_send::{self, Guard};
use crate::readonly::{DEFAULT_ROWS, MAX_ROWS, QueryFailure, WRITE_REFUSED, describe_with};
use crate::safety::gate::{self, Refusal};
use crate::schema::Schema;
use crate::state::{AppState, Target};
use crate::store::agent::{AgentAccess, AppResults};
use crate::store::apps::{Activity, AppGate, NOT_APPROVED, UNKNOWN_APP};
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

/// The Tauri app identifier: where the app keeps its config and data.
const APP_ID: &str = "com.sayql.app";
/// Most rows `export_csv` writes.
const EXPORT_ROWS: usize = 100_000;
/// How long a query may run before SayQL stops it. An AI app has no way to cancel one, and
/// a forgotten query would keep the database busy.
const QUERY_TIMEOUT: Duration = Duration::from_secs(120);
const EXPORT_TIMEOUT: Duration = Duration::from_secs(600);

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
export_csv works only where the user allows `all`.";

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
const EXPORT_OFF: &str = "The user hasn't allowed AI apps to see all rows on this connection, so export_csv is off \
here. Use open_query to put the query in a SayQL tab: the user can run it and save the rows from there.";

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
    tool_router: ToolRouter<Self>,
}

#[tool_router]
impl SayqlServer {
    pub fn new(state: AppState) -> Self {
        SayqlServer {
            state: Arc::new(state),
            tool_router: Self::tool_router(),
        }
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
                .find(|s| s.id == script_id)
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
            let conn = self
                .state
                .store
                .connection(&draft.connection_id)
                .map_err(|_| {
                    format!(
                        "No saved connection has id {:?}; use one from list_connections.",
                        draft.connection_id
                    )
                })?;
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

/// One page of a query's rows.
struct Page {
    rows: Vec<Vec<Value>>,
    offset: usize,
    /// Where the next page starts, when there are more rows SayQL read.
    next_offset: Option<usize>,
    /// More rows exist, read or not.
    more: bool,
    /// The page reached the end of what SayQL reads, and the query has more.
    past_cap: bool,
}

/// A query's result as the connection's "AI apps may see" setting allows: row counts, column
/// summaries, a page of masked rows, or a page of rows with never-send columns withheld.
/// `withheld`: result positions the never-send check found to hold never-send values.
fn shape(
    level: AppResults,
    never: &mut Masker,
    sql: &str,
    withheld: &[usize],
    mut r: QueryResult,
    offset: usize,
    limit: usize,
) -> Value {
    // Which result columns are renamed personal or never-send ones, and what the SQL lets
    // SayQL trace: for counts (min/max) and masked rows alike.
    never.learn_aliases(sql);
    never.withhold_positions(withheld);
    let rows = match level {
        AppResults::Off => {
            return json!({ "columns": r.columns, "row_count": r.rows.len(), "truncated": r.truncated,
                           "elapsed_ms": r.elapsed_ms, "note": OFF_NOTE });
        }
        AppResults::Counts => {
            return json!({ "summary": never.summarize(&r), "elapsed_ms": r.elapsed_ms, "note": COUNTS_NOTE });
        }
        AppResults::Masked | AppResults::All => std::mem::take(&mut r.rows),
    };
    let elapsed_ms = r.elapsed_ms;
    let mut page = page(rows, r.truncated, offset, limit);
    let mut columns = r.columns.clone();
    let mut notes = Vec::new();
    if level == AppResults::Masked {
        let masked = never.mask(
            &QueryResult {
                rows: std::mem::take(&mut page.rows),
                ..r
            },
            usize::MAX,
        );
        (columns, page.rows) = (masked.columns, masked.rows);
        if !masked.masked_columns.is_empty() {
            notes.push(MASKED_NOTE);
        }
    } else {
        withhold(never, &columns, &mut page.rows);
    }
    if page.past_cap {
        notes.push(
            "SayQL reads at most 5,000 rows per query. For later rows, add a WHERE or LIMIT/OFFSET to the SQL; \
             to save them all, use export_csv.",
        );
    }
    let mut out = json!({
        "columns": columns,
        "row_count": page.rows.len(),
        "rows": page.rows,
        "offset": page.offset,
        "next_offset": page.next_offset,
        "truncated": page.more,
        "elapsed_ms": elapsed_ms,
    });
    if !notes.is_empty() {
        out["note"] = json!(notes.join(" "));
    }
    out
}

/// A failed query's error as the AI app may see it: the database's message only at `all` on a
/// query that reads no table with never-send columns; otherwise a fixed description of the
/// error (see ai/redact.rs), since messages can quote values.
fn shown_error(level: AppResults, guard: &Guard, e: QueryFailure) -> String {
    if level == AppResults::All && !guard.protected {
        e.full
    } else {
        e.safe
    }
}

/// A query that failed in the database: logged in its safe form (never values), and shown as
/// `shown_error` says.
fn failed(log: &mut Activity, level: AppResults, guard: &Guard, e: QueryFailure) -> String {
    log.status = "error".into();
    log.detail = Some(e.safe.chars().take(160).collect());
    shown_error(level, guard, e)
}

/// `sql` with the text of every string literal and comment replaced by `…`, read as `kind`'s
/// database reads it (every engine's way when the connection is gone).
fn mask_literals(sql: &str, kind: Option<crate::db::Kind>) -> String {
    use crate::sql_lex::Dialect;
    match kind {
        Some(k) => crate::sql_lex::mask_values(sql, &[never_send::dialect(k)]),
        None => crate::sql_lex::mask_values(
            sql,
            &[
                Dialect::Postgres,
                Dialect::MySql,
                Dialect::Sqlite,
                Dialect::Mssql,
            ],
        ),
    }
}

/// Marks never-send columns in the schema text the AI app reads: `ssn text [never sent to AI]`.
fn mark_never_send(schema: &mut Schema, never: &[never_send::Entry]) {
    for t in &mut schema.tables {
        let table = t.name.to_lowercase();
        for c in &mut t.columns {
            let column = c.name.to_lowercase();
            if never
                .iter()
                .any(|e| e.column == column && e.table.as_ref().is_none_or(|x| *x == table))
            {
                c.data_type.push_str(" [never sent to AI]");
            }
        }
    }
}

/// Marks the log entry refused, with a short reason, and returns the message for the AI app.
fn refused(log: &mut Activity, why: &str, message: String) -> String {
    log.status = "refused".into();
    log.detail = Some(why.to_string());
    message
}

/// Replaces the values of never-send columns (by name, or by position from the never-send
/// check). Call after `learn_aliases`, so a `table.column` entry applies only where its table is read.
fn withhold(never: &Masker, columns: &[String], rows: &mut [Vec<Value>]) {
    let hidden: Vec<usize> = columns
        .iter()
        .enumerate()
        .filter(|(i, c)| never.is_withheld(*i, c))
        .map(|(i, _)| i)
        .collect();
    for row in rows.iter_mut() {
        for &i in &hidden {
            if let Some(v) = row.get_mut(i).filter(|v| !v.is_null()) {
                *v = json!("(withheld: never sent to AI)");
            }
        }
    }
}

fn page(rows: Vec<Vec<Value>>, cut_off: bool, offset: usize, limit: usize) -> Page {
    let total = rows.len();
    let rows: Vec<_> = rows.into_iter().skip(offset).take(limit).collect();
    let end = offset.min(total) + rows.len();
    Page {
        offset,
        next_offset: (end < total).then_some(end),
        more: end < total || cut_off,
        past_cap: end >= total && cut_off,
        rows,
    }
}

/// RFC 4180 CSV, like "Copy as CSV" in the app. NULL becomes an empty field.
fn to_csv(columns: &[String], rows: &[Vec<Value>]) -> String {
    fn field(s: &str) -> String {
        if s.contains(['"', ',', '\n', '\r']) {
            format!("\"{}\"", s.replace('"', "\"\""))
        } else {
            s.to_string()
        }
    }
    let cell = |v: &Value| match v {
        Value::Null => String::new(),
        Value::String(s) => field(s),
        other => field(&other.to_string()),
    };
    let mut out = columns
        .iter()
        .map(|c| field(c))
        .collect::<Vec<_>>()
        .join(",");
    for row in rows {
        out.push('\n');
        out.push_str(&row.iter().map(cell).collect::<Vec<_>>().join(","));
    }
    out.push('\n');
    out
}

/// A new file in `dir` named after `name` (letters, digits, spaces, `-`, `_`, `.`), never an existing one.
fn new_export_file(dir: &Path, name: Option<&str>) -> Result<(PathBuf, std::fs::File), String> {
    let clean: String = name
        .unwrap_or("")
        .chars()
        .filter(|c| c.is_alphanumeric() || " -_.".contains(*c))
        .take(80)
        .collect();
    let stem = clean
        .trim()
        .trim_end_matches(".csv")
        .trim_matches('.')
        .trim();
    let stem = if stem.is_empty() {
        "sayql-export"
    } else {
        stem
    };
    for n in 1..1000 {
        let path = dir.join(if n == 1 {
            format!("{stem}.csv")
        } else {
            format!("{stem} ({n}).csv")
        });
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(file) => return Ok((path, file)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("Could not create {}: {e}", path.display())),
        }
    }
    Err(format!("Too many files named {stem} in {}.", dir.display()))
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
        let mut list = Vec::new();
        for c in self.state.store.connections().map_err(|e| e.to_string())? {
            let place = if c.kind == Kind::Sqlite {
                c.file.clone()
            } else {
                c.host.clone()
            };
            let may_see = self.access(&c.id)?.apps;
            list.push(json!({ "id": c.id, "name": c.name, "kind": c.kind, "dialect": c.kind.dialect(), "location": place,
                    "default_database": c.database, "ai_may_see": may_see }));
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
            if connection_id.is_some_and(|id| s.connection_id != id) {
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

    async fn databases(&self, p: ConnectionParams) -> Result<String, String> {
        let pool = self
            .state
            .target_pool(&Target::default_of(&p.connection_id))
            .await
            .map_err(|e| e.to_string())?;
        match db::list_databases(&pool).await.map_err(|e| e.to_string())? {
            Some(list) => Ok(json!(list).to_string()),
            None => Ok("This is a SQLite file: it is a single database; omit `database`.".into()),
        }
    }

    async fn describe(&self, p: SchemaParams) -> Result<String, String> {
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

    /// The database notes are kept under: the one named, else the connection's default, as the
    /// app saves them (its tabs name the default database), so both see the same notes.
    fn notes_database(&self, connection_id: &str, database: Option<&str>) -> Option<String> {
        audit::notes_database(&self.state.store, connection_id, database)
    }

    /// Stores a suggested note for the user to accept or dismiss, and logs it.
    fn suggest(&self, app: &str, p: SuggestNoteParams) -> Result<String, String> {
        let conn = match self.state.store.connection(&p.connection_id) {
            Ok(c) => c,
            Err(_) => {
                let e = format!(
                    "No saved connection has id {:?}; use one from list_connections.",
                    p.connection_id
                );
                let _ = self.state.store.log_app_activity(Activity {
                    connection_id: p.connection_id.clone(),
                    status: "refused".into(),
                    detail: Some("unknown connection".into()),
                    ..Activity::new(app, "suggest_note")
                });
                return Err(e);
            }
        };
        let mut log = Activity {
            connection_id: conn.id.clone(),
            connection: conn.name.clone(),
            ..Activity::new(app, "suggest_note")
        };
        let remove = p.remove.unwrap_or(false);
        let target = p
            .replaces
            .as_deref()
            .map(str::trim)
            .filter(|r| !r.is_empty());
        let change = match (target, remove) {
            (None, false) => Ok(Change::Add),
            (Some(r), false) => Ok(Change::Replace(r)),
            (Some(r), true) => Ok(Change::Remove(r)),
            (None, true) => Err(
                "To suggest removing a note, set `replaces` to its id from describe_schema's notes.".to_string(),
            ),
        };
        let database = self.notes_database(&conn.id, p.database.as_deref());
        let result = change.and_then(|change| {
            self.state
                .store
                .suggest_checked_note_change(
                    &conn.id,
                    database.as_deref(),
                    &p.text,
                    &p.evidence,
                    app,
                    change,
                    p.check.as_deref(),
                )
                .map(|_| {
                    let what = match change {
                        Change::Add => "the note",
                        Change::Replace(_) => "replacing the note",
                        Change::Remove(_) => "removing the note",
                    };
                    format!(
                        "Suggested {what} for {:?}. It waits in SayQL until the user accepts it; describe_schema \
                         shows the notes as they are until then.",
                        conn.name
                    )
                })
                .map_err(|e| e.to_string())
        });
        // The note itself waits in SayQL for the user; the log says only what was suggested.
        match &result {
            Ok(_) => {
                log.detail = Some(
                    match (target.is_some(), remove) {
                        (false, _) => "suggested a note",
                        (true, false) => "suggested replacing a note",
                        (true, true) => "suggested removing a note",
                    }
                    .into(),
                )
            }
            Err(e) => {
                log.status = "refused".into();
                log.detail = Some(e.chars().take(160).collect());
            }
        }
        let _ = self.state.store.log_app_activity(log);
        result
    }

    /// The audit instructions for an app the gate let in, for the connection `connection`
    /// names (id or name; the only one when there's one and none is named). The audit is
    /// recorded for "Last audit" in Settings → AI apps, and logged.
    fn audit_prompt(&self, app: &str, args: Option<&Map<String, Value>>) -> Result<String, String> {
        let arg = |k: &str| {
            args.and_then(|a| a.get(k))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
        };
        let mut log = Activity::new(app, audit::PROMPT);
        let found = self
            .state
            .store
            .connections()
            .map_err(|e| e.to_string())
            .and_then(|list| {
                Ok(match arg("connection") {
                    Some(wanted) => Some(audit::find_connection(&list, &wanted)?.clone()),
                    None if list.len() == 1 => list.into_iter().next(),
                    None => None,
                })
            });
        let conn = match found {
            Ok(c) => c,
            Err(e) => {
                log.status = "error".into();
                log.detail = Some(e.chars().take(160).collect());
                let _ = self.state.store.log_app_activity(log);
                return Err(e);
            }
        };
        let Some(conn) = conn else {
            // The app picks the connection; nothing to record until it names one.
            log.detail = Some("audit prompt, connection not named".into());
            let _ = self.state.store.log_app_activity(log);
            return Ok(audit::instructions(None, false));
        };
        // A SQLite file is one database: its notes are kept without one.
        let database = if conn.kind == Kind::Sqlite {
            None
        } else {
            self.notes_database(&conn.id, arg("database").as_deref())
        };
        // A record that can't be written doesn't stop the audit.
        let _ = self
            .state
            .store
            .record_audit(&conn.id, database.as_deref(), app);
        log.connection_id = conn.id.clone();
        log.connection = conn.name.clone();
        log.detail = Some("audit prompt".into());
        let _ = self.state.store.log_app_activity(log);
        Ok(audit::instructions(
            Some(&audit::Target {
                connection_id: &conn.id,
                name: &conn.name,
                database: database.as_deref(),
            }),
            false,
        ))
    }

    /// Runs a read-only query for an AI app and returns what the connection lets it see.
    async fn query(&self, p: QueryParams, log: &mut Activity) -> Result<String, String> {
        let target = Target::new(p.connection_id, p.database);
        let access = self.access(&target.connection_id)?;
        log.may_see = Some(access.apps);
        let pool = self
            .state
            .target_pool(&target)
            .await
            .map_err(|e| e.to_string())?;
        // Columns the user marked never to send to an AI are refused in any form, and withheld
        // when a query returns them anyway (SELECT *); see never_send.rs.
        let guard = self.checked(&pool, &p.sql, &access.never_send, log).await?;
        let mut never = Masker::new(&access.never_send);
        let r = gate::run(
            &pool,
            &p.sql,
            db::MAX_ROWS,
            QUERY_TIMEOUT,
            &CancellationToken::new(),
        )
        .await
        .map_err(|e| failed(log, access.apps, &guard, e))?;
        log.rows = Some(r.rows.len());
        let limit = p.max_rows.unwrap_or(DEFAULT_ROWS).clamp(1, MAX_ROWS);
        Ok(shape(
            access.apps,
            &mut never,
            &p.sql,
            &guard.withheld,
            r,
            p.offset.unwrap_or(0),
            limit,
        )
        .to_string())
    }

    /// The AI query gate with MCP's options (see safety/gate.rs): the write refusal, on every
    /// pool, then the never-send and forbidden-function check, logged when it refuses.
    async fn checked(
        &self,
        pool: &db::DbPool,
        sql: &str,
        never_send: &[String],
        log: &mut Activity,
    ) -> Result<Guard, String> {
        gate::prepare(pool, sql, never_send, gate::MCP)
            .await
            .map_err(|r| match r {
                Refusal::Write => refused(log, "could change data", WRITE_REFUSED.into()),
                // MCP's options leave this to the read-only check, so it doesn't happen.
                Refusal::Statements => refused(
                    log,
                    "more than one statement",
                    "Run one statement at a time.".into(),
                ),
                Refusal::NeverSend(f) => {
                    // SayQL's own refusal, or the database rejecting the plan (an error in the SQL).
                    log.status = if f.full == f.safe { "refused" } else { "error" }.into();
                    log.detail = Some(f.safe.chars().take(160).collect());
                    f.safe
                }
            })
    }

    /// Saves a read-only query's rows to a CSV in Downloads, where the connection allows all rows.
    async fn export(&self, p: ExportParams, log: &mut Activity) -> Result<String, String> {
        let target = Target::new(p.connection_id, p.database);
        // The AI app could read the file back, so exporting needs the same permission as seeing rows.
        let access = self.access(&target.connection_id)?;
        log.may_see = Some(access.apps);
        if access.apps != AppResults::All {
            return Err(refused(log, "export needs all rows", EXPORT_OFF.into()));
        }
        let pool = self
            .state
            .target_pool(&target)
            .await
            .map_err(|e| e.to_string())?;
        // The same checks as run_query: the AI app could read the file back.
        let guard = self.checked(&pool, &p.sql, &access.never_send, log).await?;
        let mut never = Masker::new(&access.never_send);
        let dir = dirs::download_dir()
            .or_else(dirs::home_dir)
            .ok_or("Could not find the Downloads folder.")?;
        let mut r = gate::run(
            &pool,
            &p.sql,
            EXPORT_ROWS,
            EXPORT_TIMEOUT,
            &CancellationToken::new(),
        )
        .await
        .map_err(|e| failed(log, access.apps, &guard, e))?;
        never.learn_aliases(&p.sql);
        never.withhold_positions(&guard.withheld);
        withhold(&never, &r.columns, &mut r.rows);
        log.rows = Some(r.rows.len());
        let (path, mut file) = new_export_file(&dir, p.file_name.as_deref())?;
        file.write_all(to_csv(&r.columns, &r.rows).as_bytes())
            .map_err(|e| format!("Could not write {}: {e}", path.display()))?;
        Ok(json!({
            "path": path,
            "columns": r.columns,
            "row_count": r.rows.len(),
            "truncated": r.truncated,
            "note": if r.truncated { "Stopped at 100,000 rows; narrow the query for the rest." } else { "All rows saved." },
        })
        .to_string())
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
            let _ = self.state.store.log_app_activity(Activity {
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
        let _ = self.state.store.log_app_activity(log);
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
        let _ = self.state.store.log_app_activity(log);
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

/// The `audit` prompt as prompts/list shows it.
fn audit_prompt_info() -> Prompt {
    Prompt::new(
        audit::PROMPT,
        Some(audit::DESCRIPTION),
        Some(vec![
            PromptArgument::new("connection")
                .with_description(
                    "The SayQL connection to audit: its name or id. Optional when only one is saved; otherwise the app picks it with list_connections.",
                )
                .with_required(false),
            PromptArgument::new("database")
                .with_description(
                    "A database on that connection's server. Omit for the connection's default (and for SQLite).",
                )
                .with_required(false),
        ]),
    )
    .with_title("Audit: teach SayQL your database")
}

/// Where the SayQL app keeps its files; `SAYQL_CONFIG_DIR` overrides (for tests).
fn app_dirs() -> Result<(PathBuf, PathBuf), String> {
    if let Ok(dir) = std::env::var("SAYQL_CONFIG_DIR") {
        let dir = PathBuf::from(dir);
        return Ok((dir.clone(), dir));
    }
    let config = dirs::config_dir()
        .ok_or("Could not find the user's config directory")?
        .join(APP_ID);
    let data = dirs::data_dir()
        .ok_or("Could not find the user's data directory")?
        .join(APP_ID);
    Ok((config, data))
}

/// Runs the server on stdin/stdout until the client disconnects.
pub async fn serve_stdio() -> Result<(), Box<dyn std::error::Error>> {
    let (config_dir, data_dir) = app_dirs()?;
    let mut state = AppState::new(config_dir, data_dir)?;
    // The app owns its files: this process never moves or replaces one it can't read.
    state.store = state.store.without_repair();
    let server = SayqlServer::new(state);
    server
        .serve(rmcp::transport::stdio())
        .await?
        .waiting()
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio_util::sync::CancellationToken;

    /// The audit run ("Run audit") allows or denies every tool this server offers by name, so a
    /// new tool can't slip into the run unlisted.
    #[test]
    fn the_audit_allows_or_denies_every_tool() {
        use crate::audit_runner::{CLAUDE_DENIED, SAYQL_TOOLS};
        let offered: Vec<String> = SayqlServer::tool_router()
            .list_all()
            .into_iter()
            .map(|t| format!("mcp__sayql__{}", t.name))
            .collect();
        assert!(offered.len() >= 10, "{offered:?}");
        let allowed: Vec<String> = SAYQL_TOOLS
            .iter()
            .map(|t| format!("mcp__sayql__{t}"))
            .collect();
        for tool in &offered {
            let a = allowed.contains(tool);
            let d = CLAUDE_DENIED.contains(&tool.as_str());
            assert!(a ^ d, "{tool}: allowed {a}, denied {d}");
        }
        for listed in allowed.iter().map(String::as_str).chain(
            CLAUDE_DENIED
                .iter()
                .copied()
                .filter(|t| t.starts_with("mcp__")),
        ) {
            assert!(offered.iter().any(|t| t == listed), "{listed} isn't a tool");
        }
    }

    fn add_params(args: Value) -> Result<AddConnectionParams, serde_json::Error> {
        serde_json::from_value(args)
    }

    #[test]
    fn add_connection_refuses_passwords() {
        let p =
            add_params(json!({ "type": "postgres", "host": "db", "password": "hunter2" })).unwrap();
        assert_eq!(connection_draft(p).unwrap_err(), handoff::secret_refused());
        let p = add_params(json!({ "type": "mysql", "apiKey": "x" })).unwrap();
        assert_eq!(connection_draft(p).unwrap_err(), handoff::secret_refused());
        let p = add_params(json!({ "type": "mysql", "colour": "red" })).unwrap();
        assert!(connection_draft(p).unwrap_err().contains("colour"));
    }

    #[test]
    fn add_connection_validates_fields() {
        let p = add_params(json!({ "type": "sqlserver", "host": "sql.local", "port": 1433, "ssl": true, "trust_server_certificate": true })).unwrap();
        let d = connection_draft(p).unwrap();
        assert_eq!(
            (d.kind, d.port, d.ssl, d.trust_cert),
            (Kind::Mssql, Some(1433), Some(true), Some(true))
        );
        assert!(
            connection_draft(add_params(json!({ "type": "postgres", "port": 0 })).unwrap())
                .is_err()
        );
        assert!(connection_draft(add_params(json!({ "type": "sqlite" })).unwrap()).is_err());
        assert!(
            add_params(json!({ "host": "db" })).is_err(),
            "type is required"
        );
        assert!(add_params(json!({ "type": "postgres", "port": "5432" })).is_err());
    }

    #[test]
    fn withholds_never_send_columns() {
        let mut never = Masker::new(["patients.notes"]);
        never.learn_aliases("SELECT * FROM patients");
        let mut rows = vec![
            vec![json!(1), json!("private"), json!("x")],
            vec![json!(2), Value::Null, json!("y")],
        ];
        withhold(
            &never,
            &["id".into(), "NOTES".into(), "name".into()],
            &mut rows,
        );
        assert_eq!(
            rows[0],
            vec![json!(1), json!("(withheld: never sent to AI)"), json!("x")]
        );
        assert_eq!(rows[1][1], Value::Null);
        // `patients.notes` is about patients: a `notes` column of another table is sent.
        never.learn_aliases("SELECT * FROM visits");
        let mut rows = vec![vec![json!(1), json!("follow up"), json!("x")]];
        withhold(
            &never,
            &["id".into(), "notes".into(), "name".into()],
            &mut rows,
        );
        assert_eq!(rows[0][1], json!("follow up"));
        // A position from the Postgres plan is withheld whatever its name.
        never.learn_aliases("SELECT t.c6 FROM (SELECT * FROM patients) t(c1, c2, c3, c4, c5, c6)");
        never.withhold_positions(&[0]);
        let mut rows = vec![vec![json!("123-45-6789")]];
        withhold(&never, &["c6".into()], &mut rows);
        assert_eq!(rows[0][0], json!("(withheld: never sent to AI)"));
    }

    fn result(columns: &[&str], rows: Vec<Vec<Value>>) -> QueryResult {
        QueryResult {
            columns: columns.iter().map(|c| c.to_string()).collect(),
            rows,
            rows_affected: None,
            truncated: false,
            elapsed_ms: 3,
            withheld: vec![],
        }
    }

    #[test]
    fn results_follow_what_ai_apps_may_see() {
        let rows = || {
            result(
                &["id", "email", "notes", "total"],
                vec![
                    vec![
                        json!(1),
                        json!("ana@example.com"),
                        json!("allergic to penicillin"),
                        json!(40),
                    ],
                    vec![json!(2), json!("raj@example.com"), json!("vip"), json!(60)],
                ],
            )
        };
        let sql = "SELECT id, email, notes, total FROM customers";
        let never = || Masker::new(["notes".to_string()]);
        let text = |v: &Value| v.to_string();

        // Off: the row count and column names, no values at all.
        let off = shape(AppResults::Off, &mut never(), sql, &[], rows(), 0, 100);
        assert_eq!(
            (off["row_count"].clone(), off["columns"].clone()),
            (json!(2), json!(["id", "email", "notes", "total"]))
        );
        assert!(
            off.get("rows").is_none()
                && !text(&off).contains("ana@")
                && !text(&off).contains("vip")
        );

        // Counts: per-column summaries; no row values or labels.
        let counts = shape(AppResults::Counts, &mut never(), sql, &[], rows(), 0, 100);
        assert_eq!(counts["summary"]["rowCount"], json!(2));
        assert_eq!(counts["summary"]["columns"][3]["max"], json!(60));
        for value in ["ana@", "raj@", "penicillin", "vip"] {
            assert!(!text(&counts).contains(value), "{value} in {counts}");
        }

        // Masked: rows, with personal data and never-send columns as tokens.
        let masked = shape(AppResults::Masked, &mut never(), sql, &[], rows(), 0, 100);
        assert_eq!(masked["rows"][0][0], json!(1));
        assert_eq!(masked["rows"][1][3], json!(60));
        for value in ["ana@", "raj@", "penicillin", "vip"] {
            assert!(!text(&masked).contains(value), "{value} in {masked}");
        }
        assert!(masked["note"].as_str().unwrap().contains("tokens"));

        // All: rows as they are, never-send columns still withheld, paged.
        let all = shape(AppResults::All, &mut never(), sql, &[], rows(), 1, 100);
        assert_eq!(
            all["rows"],
            json!([[2, "raj@example.com", "(withheld: never sent to AI)", 60]])
        );
        assert_eq!(
            (all["offset"].clone(), all["next_offset"].clone()),
            (json!(1), Value::Null)
        );
    }

    #[test]
    fn errors_show_only_their_safe_form_below_all_or_near_never_send_columns() {
        let e = || QueryFailure {
            full: "unrecognized configuration parameter \"chandra@example.in\"".into(),
            safe: "Something the query names does not exist.".into(),
        };
        let open = Guard::default();
        let protected = Guard {
            protected: true,
            withheld: vec![],
        };
        for level in [AppResults::Off, AppResults::Counts, AppResults::Masked] {
            assert!(
                !shown_error(level, &open, e()).contains("chandra"),
                "{level:?}"
            );
        }
        assert!(shown_error(AppResults::All, &open, e()).contains("chandra"));
        assert!(!shown_error(AppResults::All, &protected, e()).contains("chandra"));
        // The log keeps the safe form, at every level.
        let mut log = Activity::new("x", "run_query");
        let shown = failed(&mut log, AppResults::All, &open, e());
        assert!(shown.contains("chandra") && !log.detail.unwrap().contains("chandra"));
        assert_eq!(log.status, "error");
    }

    /// H1 and M1 of the governance test (2026-10-28): renamed personal columns get no min/max
    /// at Counts, and neither do computed numbers (which can carry text).
    #[test]
    fn counts_give_min_and_max_only_for_plain_columns_and_aggregates() {
        let r = |cols: &[&str], v: Vec<Value>| result(cols, vec![v.clone(), v]);
        let summary = |sql: &str, res: QueryResult| {
            shape(
                AppResults::Counts,
                &mut Masker::new(Vec::<String>::new()),
                sql,
                &[],
                res,
                0,
                100,
            )
        };
        for sql in [
            "SELECT date_of_birth AS d FROM customers",
            "SELECT date_of_birth + 0 AS d FROM customers",
            "SELECT extract(year FROM date_of_birth) AS d FROM customers",
            "SELECT ('x'||encode(substr(email,1,7)::bytea,'hex'))::bit(56)::bigint AS d FROM customers WHERE id=1",
            "SELECT ascii(substr(email, 1, 1)) AS d FROM customers",
            "SELECT d FROM (SELECT ascii(substr(email, 1, 1)) AS d FROM customers) t",
            "SELECT sum(ascii(substr(email, 1, 1))) AS d FROM customers",
            "SELECT 1 AS d UNION ALL SELECT ascii(email) FROM customers",
        ] {
            let s = summary(sql, r(&["d"], vec![json!(97)]));
            assert!(
                s["summary"]["columns"][0].get("max").is_none(),
                "{sql}: {s}"
            );
        }
        for sql in [
            "SELECT count(*) AS d FROM customers WHERE email LIKE 'a%'",
            "SELECT max(total) AS d FROM orders",
            "SELECT o.total AS d FROM orders o",
            "SELECT round(avg(total), 2) AS d FROM orders",
            "SELECT count(DISTINCT email) d FROM customers",
        ] {
            let s = summary(sql, r(&["d"], vec![json!(97)]));
            assert_eq!(s["summary"]["columns"][0]["max"], json!(97), "{sql}: {s}");
        }
        let s = summary(
            "SELECT * FROM orders",
            r(&["id", "total"], vec![json!(1), json!(5)]),
        );
        assert_eq!(s["summary"]["columns"][1]["max"], json!(5), "{s}");
    }

    /// H2 of the governance test (2026-10-28): at Masked, names that reach the result without
    /// their column's name are masked too.
    #[test]
    fn masked_rows_hide_names_that_arrive_under_other_names() {
        for sql in [
            "SELECT t.a FROM (SELECT name FROM customers) t(a)",
            "SELECT a FROM customers c, LATERAL (VALUES (c.name)) v(a)",
            "SELECT (c).name AS q FROM customers c",
            "SELECT x FROM unnest(ARRAY(SELECT name FROM customers)) x",
            "SELECT to_jsonb(c)->>'name' AS v FROM customers c",
            "SELECT 'x' AS a UNION ALL SELECT name FROM customers",
            "SELECT 1 AS id, 'x' AS label UNION ALL SELECT id, name FROM customers",
            "WITH x(a) AS (SELECT name FROM customers) SELECT a FROM x",
            "SELECT U&\"n\\0061me\" AS v FROM customers",
        ] {
            let res = result(
                &["v"],
                vec![vec![json!("Alice Smith")], vec![json!("Bob Jones")]],
            );
            let out = shape(
                AppResults::Masked,
                &mut Masker::new(Vec::<String>::new()),
                sql,
                &[],
                res,
                0,
                100,
            );
            assert!(!out.to_string().contains("Alice"), "{sql}: {out}");
        }
        // Plain queries keep ordinary values.
        let res = result(&["status"], vec![vec![json!("paid")]]);
        let out = shape(
            AppResults::Masked,
            &mut Masker::new(Vec::<String>::new()),
            "SELECT o.status FROM orders o JOIN customers c ON c.id = o.customer_id",
            &[],
            res,
            0,
            100,
        );
        assert_eq!(out["rows"][0][0], json!("paid"), "{out}");
    }

    #[test]
    fn scripts_show_as_much_sql_as_the_level_allows() {
        assert_eq!(
            mask_literals(
                "SELECT * FROM c WHERE email = 'alice@x.com' AND n = 'it''s'",
                None
            ),
            "SELECT * FROM c WHERE email = '…' AND n = '…'"
        );
        // The 2026-10-29 audit: MySQL "…" strings, backslash escapes and comments leaked.
        let masked = mask_literals(
            "SELECT * FROM c WHERE email = \"bob@x.com\" AND note = 'a\\' carol' -- eve@x.com",
            Some(crate::db::Kind::Mysql),
        );
        for value in ["bob", "carol", "eve"] {
            assert!(!masked.contains(value), "{value} in {masked}");
        }
    }

    #[test]
    fn describe_marks_never_send_columns() {
        use crate::schema::{Column, Table};
        let col = |n: &str| Column {
            name: n.into(),
            data_type: "text".into(),
            nullable: true,
            primary_key: false,
            ..Default::default()
        };
        let table = |n: &str| Table {
            schema: None,
            name: n.into(),
            kind: "table".into(),
            columns: vec![col("email"), col("ssn")],
            foreign_keys: vec![],
            row_estimate: None,
            ..Default::default()
        };
        let mut s = Schema {
            dialect: "PostgreSQL".into(),
            tables: vec![table("customers"), table("staff")],
        };
        mark_never_send(&mut s, &never_send::entries(&["customers.email", "ssn"]));
        let types: Vec<Vec<&str>> = s
            .tables
            .iter()
            .map(|t| t.columns.iter().map(|c| c.data_type.as_str()).collect())
            .collect();
        assert_eq!(
            types,
            [
                ["text [never sent to AI]", "text [never sent to AI]"],
                ["text", "text [never sent to AI]"]
            ]
        );
    }

    #[test]
    fn unreadable_settings_refuse_everything_and_stay_put() {
        let dir = std::env::temp_dir().join(format!("sayql-mcp-corrupt-{}", uuid::Uuid::new_v4()));
        let mut state = AppState::new(dir.clone(), dir.clone()).unwrap();
        state.store = state.store.without_repair();
        let bad = r#"{"pg-off": {"apps": "off", "neverSend": ["ssn"]}"#;
        std::fs::write(dir.join("agent.json"), bad).unwrap();
        let server = SayqlServer::new(state);
        assert_eq!(server.access("pg-off").unwrap_err(), SETTINGS_UNREADABLE);
        assert!(server.state.store.agent_access_readable().is_err());
        assert_eq!(
            std::fs::read_to_string(dir.join("agent.json")).unwrap(),
            bad
        );
        let others: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains("corrupt"))
            .collect();
        assert!(others.is_empty(), "nothing moved aside");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_gate_lets_in_only_what_the_user_allows() {
        let dir = std::env::temp_dir().join(format!("sayql-mcp-gate-{}", uuid::Uuid::new_v4()));
        let mut state = AppState::new(dir.clone(), dir.clone()).unwrap();
        state.store = state.store.without_repair();
        let server = SayqlServer::new(state);
        let store = &server.state.store;
        let gate = |app: &str| server.gate_app(app.to_string(), "run_query");

        // Off (the default): anything not blocked.
        assert_eq!(gate("Zed").unwrap(), "Zed");
        store
            .log_app_activity(Activity::new("Zed", "run_query"))
            .unwrap();
        store.set_app_blocked("Cursor", true).unwrap();
        assert!(gate("Cursor").unwrap_err().contains("blocked Cursor"));

        // On: Zed was seen, so it's approved; a new app waits.
        store.set_only_approved_apps(true).unwrap();
        assert_eq!(gate("Zed").unwrap(), "Zed");
        assert_eq!(
            gate("Windsurf").unwrap_err(),
            "SayQL only lets in AI apps the user has approved. Ask them to approve Windsurf in SayQL under Settings → AI apps → Access."
        );
        let last = &store.app_activity(1).unwrap()[0];
        assert_eq!(
            (
                last.app.as_str(),
                last.status.as_str(),
                last.detail.as_deref()
            ),
            ("Windsurf", "refused", Some("not approved"))
        );
        let list = store.app_summaries().unwrap();
        let w = list.apps.iter().find(|a| a.name == "Windsurf").unwrap();
        assert!(list.only_approved && !w.approved && !w.blocked);
        store.approve_app("Windsurf").unwrap();
        assert_eq!(gate("Windsurf").unwrap(), "Windsurf");

        // Blocked wins over approved.
        store.approve_app("Cursor").unwrap();
        assert!(gate("Cursor").unwrap_err().contains("blocked Cursor"));

        // A corrupt apps file still refuses everything, approved or not, and stays put.
        std::fs::write(dir.join("mcp-apps.json"), r#"{"only_approved": tr"#).unwrap();
        assert_eq!(gate("Zed").unwrap_err(), SETTINGS_UNREADABLE);
        assert_eq!(
            std::fs::read_to_string(dir.join("mcp-apps.json")).unwrap(),
            r#"{"only_approved": tr"#
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn ai_apps_see_counts_until_the_user_chooses() {
        assert_eq!(AgentAccess::default().apps, AppResults::Counts);
        let saved: AgentAccess =
            serde_json::from_value(json!({ "enabled": true, "results": "rows" })).unwrap();
        assert_eq!(
            saved.apps,
            AppResults::Counts,
            "settings saved before this setting existed"
        );
        assert_eq!(
            serde_json::to_value(AppResults::Masked).unwrap(),
            json!("masked")
        );
    }

    #[test]
    fn pages_through_rows() {
        let rows = |n: usize| (0..n).map(|i| vec![json!(i)]).collect::<Vec<_>>();
        let p = page(rows(250), false, 0, 100);
        assert_eq!(
            (p.rows.len(), p.next_offset, p.more, p.past_cap),
            (100, Some(100), true, false)
        );
        let p = page(rows(250), false, 200, 100);
        assert_eq!(
            (p.rows[0][0].clone(), p.rows.len(), p.next_offset, p.more),
            (json!(200), 50, None, false)
        );
        let p = page(rows(250), false, 900, 100);
        assert_eq!((p.rows.len(), p.next_offset, p.more), (0, None, false));
        let p = page(rows(5000), true, 4950, 100);
        assert_eq!(
            (p.rows.len(), p.next_offset, p.more, p.past_cap),
            (50, None, true, true)
        );
    }

    #[test]
    fn writes_csv_like_the_app() {
        let csv = to_csv(
            &["id".into(), "note".into()],
            &[
                vec![json!(1), json!("a, \"b\"")],
                vec![json!(2), Value::Null],
            ],
        );
        assert_eq!(csv, "id,note\n1,\"a, \"\"b\"\"\"\n2,\n");
    }

    #[test]
    fn export_files_never_overwrite() {
        let dir = std::env::temp_dir().join(format!("sayql-export-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let (a, _) = new_export_file(&dir, Some("../../etc/orders.csv")).unwrap();
        let (b, _) = new_export_file(&dir, Some("....etcorders")).unwrap();
        let (c, _) = new_export_file(&dir, None).unwrap();
        assert_eq!(a, dir.join("etcorders.csv"));
        assert_eq!(b, dir.join("etcorders (2).csv"));
        assert_eq!(c, dir.join("sayql-export.csv"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn labels_known_clients() {
        let info = |name: &str| Implementation::new(name, "1");
        assert_eq!(client_label(&info("claude-ai")).as_deref(), Some("Claude"));
        assert_eq!(
            client_label(&info("claude-code")).as_deref(),
            Some("Claude Code")
        );
        assert_eq!(
            client_label(&info("cursor-vscode")).as_deref(),
            Some("Cursor")
        );
        assert_eq!(client_label(&info("Zed")).as_deref(), Some("Zed"));
        assert_eq!(
            client_label(&info("opencode")).as_deref(),
            Some(crate::audit_opencode::APP)
        );
        // The title an app sends is not used, and an unknown name is marked unverified.
        let mut spoof = info("x-agent");
        spoof.title = Some("SayQL Support".into());
        assert_eq!(client_label(&spoof).as_deref(), Some("x-agent"));
        assert_eq!(
            handoff_label(&spoof).as_deref(),
            Some("x-agent (unverified)")
        );
        assert_eq!(
            handoff_label(&info("claude-code")).as_deref(),
            Some("Claude Code")
        );
    }

    #[test]
    fn suggested_notes_wait_for_the_user_and_are_logged() {
        let dir = std::env::temp_dir().join(format!("sayql-mcp-suggest-{}", uuid::Uuid::new_v4()));
        let state = AppState::new(dir.clone(), dir.clone()).unwrap();
        let cfg = db::ConnConfig {
            id: "c1".into(),
            name: "Shop".into(),
            kind: Kind::Sqlite,
            host: String::new(),
            port: None,
            database: String::new(),
            user: String::new(),
            file: "/tmp/none.sqlite".into(),
            ssl: false,
            trust_cert: false,
            production: false,
        };
        let pg = db::ConnConfig {
            id: "pg".into(),
            kind: Kind::Postgres,
            database: "shop".into(),
            file: String::new(),
            ..cfg.clone()
        };
        state.store.save_connections(&[cfg, pg]).unwrap();
        let server = SayqlServer::new(state);
        // Without `database`, notes go under the connection's default database, where the app keeps them.
        let on_pg = SuggestNoteParams {
            connection_id: "pg".into(),
            database: None,
            text: "status 3 = cancelled".into(),
            evidence: String::new(),
            replaces: None,
            remove: None,
            check: None,
        };
        server.suggest("Cursor", on_pg).unwrap();
        let s = server.state.store.note_suggestions().unwrap().remove(0);
        assert_eq!(s.database.as_deref(), Some("shop"));
        server.state.store.dismiss_note_suggestion(&s.id).unwrap();
        server.state.store.clear_app_activity().unwrap();
        let p = |text: &str, evidence: &str| SuggestNoteParams {
            connection_id: "c1".into(),
            database: None,
            text: text.into(),
            evidence: evidence.into(),
            replaces: None,
            remove: None,
            check: None,
        };
        let ok = server
            .suggest(
                "Claude Code",
                p(
                    "orders_legacy is unused since March; use orders_v2",
                    "0 writes since 2026-01-01",
                ),
            )
            .unwrap();
        assert!(ok.contains("until the user accepts"), "{ok}");
        let store = &server.state.store;
        assert!(store.notes_text("c1", None).is_empty(), "not a note yet");
        let waiting = store.note_suggestions().unwrap();
        assert_eq!((waiting.len(), waiting[0].app.as_str()), (1, "Claude Code"));

        assert!(
            server
                .suggest(
                    "Claude Code",
                    p("orders_legacy is unused since March; use orders_v2", "")
                )
                .unwrap_err()
                .contains("already suggested")
        );
        assert!(
            server
                .suggest("Cursor", p("the owner is dana@example.com", ""))
                .is_err()
        );
        assert!(
            server
                .suggest(
                    "Cursor",
                    SuggestNoteParams {
                        connection_id: "nope".into(),
                        ..p("x", "")
                    }
                )
                .unwrap_err()
                .contains("list_connections")
        );

        let log = store.app_activity(10).unwrap();
        assert_eq!(log.len(), 4, "an unknown connection is logged too");
        assert_eq!(
            (
                log[3].tool.as_str(),
                log[3].status.as_str(),
                log[3].connection.as_str()
            ),
            ("suggest_note", "ok", "Shop")
        );
        assert_eq!(
            log[3].detail.as_deref(),
            Some("suggested a note"),
            "the note text stays out of the log"
        );
        assert_eq!(log[0].status, "refused");

        store.accept_note_suggestion(&waiting[0].id).unwrap();
        assert!(
            store
                .notes_text("c1", None)
                .contains("orders_legacy is unused"),
            "accepted notes reach describe_schema"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    fn sqlite_conn(id: &str, name: &str) -> db::ConnConfig {
        db::ConnConfig {
            id: id.into(),
            name: name.into(),
            kind: Kind::Sqlite,
            host: String::new(),
            port: None,
            database: String::new(),
            user: String::new(),
            file: "/tmp/none.sqlite".into(),
            ssl: false,
            trust_cert: false,
            production: false,
        }
    }

    #[test]
    fn the_audit_prompt_names_the_connection_and_is_recorded() {
        let dir = std::env::temp_dir().join(format!("sayql-mcp-audit-{}", uuid::Uuid::new_v4()));
        let state = AppState::new(dir.clone(), dir.clone()).unwrap();
        let pg = db::ConnConfig {
            kind: Kind::Postgres,
            database: "shop".into(),
            file: String::new(),
            ..sqlite_conn("pg", "Warehouse")
        };
        state
            .store
            .save_connections(&[sqlite_conn("c1", "Shop"), pg])
            .unwrap();
        let server = SayqlServer::new(state);
        let store = &server.state.store;
        let args = |v: Value| v.as_object().cloned();

        // By name, any case; a SQLite file has no database, whatever the app passes.
        let text = server
            .audit_prompt(
                "Claude Code",
                args(json!({ "connection": "shop", "database": "x" })).as_ref(),
            )
            .unwrap();
        assert!(
            text.contains("connection_id \"c1\"") && !text.contains("\"x\""),
            "{text}"
        );
        // By id, on the server's default database.
        let text = server
            .audit_prompt("Cursor", args(json!({ "connection": "pg" })).as_ref())
            .unwrap();
        assert!(text.contains("database \"shop\""), "{text}");
        let mut audits = store.audits().unwrap();
        audits.sort_by(|a, b| a.connection_id.cmp(&b.connection_id));
        assert_eq!(
            audits
                .iter()
                .map(|a| (
                    a.connection_id.as_str(),
                    a.database.as_deref(),
                    a.app.as_str()
                ))
                .collect::<Vec<_>>(),
            [("c1", None, "Claude Code"), ("pg", Some("shop"), "Cursor")]
        );
        // No connection named with two saved: the app picks one; nothing recorded.
        let open = server.audit_prompt("Cursor", None).unwrap();
        assert!(open.contains("call list_connections"), "{open}");
        assert_eq!(store.audits().unwrap().len(), 2);
        let e = server
            .audit_prompt("Cursor", args(json!({ "connection": "crm" })).as_ref())
            .unwrap_err();
        assert!(e.contains("\"Warehouse\""), "{e}");

        let log = store.app_activity(10).unwrap();
        assert_eq!(
            log.iter()
                .map(|l| (l.tool.as_str(), l.status.as_str(), l.connection.as_str()))
                .collect::<Vec<_>>(),
            [
                ("audit", "error", ""),
                ("audit", "ok", ""),
                ("audit", "ok", "Warehouse"),
                ("audit", "ok", "Shop"),
            ]
        );
        // The gate applies: a blocked app gets nothing.
        store.set_app_blocked("Cursor", true).unwrap();
        assert!(
            server
                .gate_app("Cursor".into(), audit::PROMPT)
                .unwrap_err()
                .contains("blocked Cursor")
        );
        assert_eq!(audit_prompt_info().name, "audit");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn suggest_note_can_replace_or_remove_a_saved_note() {
        let dir = std::env::temp_dir().join(format!("sayql-mcp-replace-{}", uuid::Uuid::new_v4()));
        let state = AppState::new(dir.clone(), dir.clone()).unwrap();
        state
            .store
            .save_connections(&[sqlite_conn("c1", "Shop")])
            .unwrap();
        let server = SayqlServer::new(state);
        let store = &server.state.store;
        let note = store
            .save_note(crate::store::agent::Note {
                id: "1A2B3C4D-0000-4000-8000-000000000000".into(),
                connection_id: "c1".into(),
                database: None,
                text: "orders_v1 is the live orders table".into(),
                updated_at: 0,
                ..Default::default()
            })
            .unwrap();
        assert!(
            store
                .notes_text_with_refs("c1", None)
                .contains("[n_1a2b3c4d] orders_v1 is the live orders table")
        );
        let p = |text: &str, replaces: Option<&str>, remove: Option<bool>| SuggestNoteParams {
            connection_id: "c1".into(),
            database: None,
            text: text.into(),
            evidence: "api/orders.ts reads orders_v2 only".into(),
            replaces: replaces.map(str::to_string),
            remove,
            check: None,
        };
        let ok = server
            .suggest(
                "Claude Code",
                p(
                    "orders_v2 is the live orders table",
                    Some("n_1a2b3c4d"),
                    None,
                ),
            )
            .unwrap();
        assert!(ok.contains("Suggested replacing the note"), "{ok}");
        assert!(
            server
                .suggest("Claude Code", p("gone", None, Some(true)))
                .unwrap_err()
                .contains("set `replaces`")
        );
        assert!(
            server
                .suggest("Claude Code", p("gone", Some("n_ffffffff"), Some(true)))
                .unwrap_err()
                .contains("No saved note")
        );
        server
            .suggest(
                "Claude Code",
                p("orders_v1 was dropped", Some("[n_1a2b3c4d]"), Some(true)),
            )
            .unwrap();
        let waiting = store.note_suggestions().unwrap();
        assert_eq!(
            waiting
                .iter()
                .map(|s| (s.replaces.as_deref(), s.remove))
                .collect::<Vec<_>>(),
            [
                (Some(note.id.as_str()), false),
                (Some(note.id.as_str()), true)
            ]
        );
        let details: Vec<_> = store
            .app_activity(10)
            .unwrap()
            .into_iter()
            .filter_map(|l| l.detail)
            .collect();
        assert!(details.contains(&"suggested replacing a note".to_string()));
        assert!(details.contains(&"suggested removing a note".to_string()));
        let _ = std::fs::remove_dir_all(dir);
    }

    /// 1000 rows, 0..999, in SQL every supported server runs.
    const THOUSAND: &str = "SELECT a.d * 100 + b.d * 10 + c.d AS v FROM \
        (SELECT 0 AS d UNION ALL SELECT 1 UNION ALL SELECT 2 UNION ALL SELECT 3 UNION ALL SELECT 4 UNION ALL SELECT 5 UNION ALL SELECT 6 UNION ALL SELECT 7 UNION ALL SELECT 8 UNION ALL SELECT 9) a CROSS JOIN \
        (SELECT 0 AS d UNION ALL SELECT 1 UNION ALL SELECT 2 UNION ALL SELECT 3 UNION ALL SELECT 4 UNION ALL SELECT 5 UNION ALL SELECT 6 UNION ALL SELECT 7 UNION ALL SELECT 8 UNION ALL SELECT 9) b CROSS JOIN \
        (SELECT 0 AS d UNION ALL SELECT 1 UNION ALL SELECT 2 UNION ALL SELECT 3 UNION ALL SELECT 4 UNION ALL SELECT 5 UNION ALL SELECT 6 UNION ALL SELECT 7 UNION ALL SELECT 8 UNION ALL SELECT 9) c \
        ORDER BY v";

    /// The tools against the demo SQLite store, and against real servers when the
    /// SAYQL_TEST_*_URL variables are set (see db/integration_tests.rs). The connection is saved
    /// without a password and its pool opened directly, so the test never touches the keychain.
    #[tokio::test]
    async fn tools_work_against_real_databases() {
        let demo_dir =
            std::env::temp_dir().join(format!("sayql-mcp-demo-{}", uuid::Uuid::new_v4()));
        let demo = demo_dir.join("demo.sqlite");
        db::ensure_demo(&demo).await.unwrap();
        let sqlite = db::ConnConfig {
            id: "demo".into(),
            name: "Demo".into(),
            kind: Kind::Sqlite,
            host: String::new(),
            port: None,
            database: String::new(),
            user: String::new(),
            file: demo.to_string_lossy().into_owned(),
            ssl: false,
            trust_cert: false,
            production: false,
        };
        let servers = [
            ("SAYQL_TEST_PG_URL", Kind::Postgres),
            ("SAYQL_TEST_MYSQL_URL", Kind::Mysql),
            ("SAYQL_TEST_MSSQL_URL", Kind::Mssql),
        ];
        let targets = std::iter::once(Some((sqlite, None)))
            .chain(servers.map(|(var, kind)| db::env_config(var, kind)));
        for (cfg, password) in targets.flatten() {
            let kind = cfg.kind;
            let dir = std::env::temp_dir().join(format!("sayql-mcp-{}", uuid::Uuid::new_v4()));
            let state = AppState::new(dir.clone(), dir.clone()).unwrap();
            state
                .store
                .save_connections(std::slice::from_ref(&cfg))
                .unwrap();
            state.set_pool(
                Target::default_of(&cfg.id),
                db::connect(&cfg, password).await.unwrap(),
            );
            let server = SayqlServer::new(state);
            let json = |s: String| serde_json::from_str::<Value>(&s).unwrap();
            let conn = || ConnectionParams {
                connection_id: cfg.id.clone(),
            };

            let list = json(server.connections().unwrap());
            assert_eq!(list[0]["dialect"], json!(kind.dialect()), "{kind:?}");
            assert_eq!(
                list[0]["ai_may_see"],
                json!("counts"),
                "{kind:?}: counts until the user chooses"
            );
            let dbs = server.databases(conn()).await.unwrap();
            if kind == Kind::Sqlite {
                assert!(dbs.contains("single database"), "{dbs}");
            } else {
                assert!(json(dbs.clone())["databases"].is_array(), "{kind:?}: {dbs}");
            }
            let schema = server
                .describe(SchemaParams {
                    connection_id: cfg.id.clone(),
                    database: None,
                    search: None,
                })
                .await
                .unwrap();
            assert!(
                schema.starts_with(&format!("Dialect: {}.", kind.dialect())),
                "{schema}"
            );
            // Usage from the database's statistics: nothing for SQLite; for servers, the period or why not.
            if kind == Kind::Sqlite {
                assert!(
                    !schema.contains("usage") && !schema.contains("Usage"),
                    "{schema}"
                );
            } else {
                assert!(schema.contains("statistics"), "{kind:?}: {schema}");
            }
            if kind == Kind::Sqlite {
                // A table added while the server runs shows up in the next describe_schema.
                let pool = server
                    .state
                    .target_pool(&Target::default_of(&cfg.id))
                    .await
                    .unwrap();
                db::run_query(
                    &pool,
                    "CREATE TABLE added_later (id INTEGER PRIMARY KEY)",
                    false,
                )
                .await
                .unwrap();
                let again = server
                    .describe(SchemaParams {
                        connection_id: cfg.id.clone(),
                        database: None,
                        search: None,
                    })
                    .await
                    .unwrap();
                assert!(
                    !schema.contains("added_later") && again.contains("added_later"),
                    "{again}"
                );
            }

            let run = |sql: &str, offset: Option<usize>| {
                let p = QueryParams {
                    connection_id: cfg.id.clone(),
                    database: None,
                    sql: sql.into(),
                    max_rows: Some(300),
                    offset,
                };
                async {
                    server
                        .query(p, &mut Activity::new("test", "run_query"))
                        .await
                }
            };
            // By default the AI app gets counts and summaries, not rows, and can't export.
            let counts = json(run(THOUSAND, None).await.unwrap());
            assert_eq!(
                counts["summary"]["rowCount"],
                json!(1000),
                "{kind:?}: {counts}"
            );
            assert!(counts.get("rows").is_none(), "{kind:?}: {counts}");
            let export = |sql: &str| {
                let p = ExportParams {
                    connection_id: cfg.id.clone(),
                    database: None,
                    sql: sql.into(),
                    file_name: None,
                };
                async {
                    server
                        .export(p, &mut Activity::new("test", "export_csv"))
                        .await
                }
            };
            assert_eq!(export(THOUSAND).await.unwrap_err(), EXPORT_OFF, "{kind:?}");
            // Rows once the user allows them.
            let all = AgentAccess {
                apps: AppResults::All,
                ..Default::default()
            };
            server.state.store.save_agent_access(&cfg.id, all).unwrap();

            // Pages through the rows, and says where the next page starts.
            let first = json(run(THOUSAND, None).await.unwrap());
            assert_eq!(
                (first["row_count"].clone(), first["next_offset"].clone()),
                (json!(300), json!(300)),
                "{kind:?}"
            );
            assert_eq!(first["rows"][0], json!([0]));
            let last = json(run(THOUSAND, Some(900)).await.unwrap());
            assert_eq!(
                (last["row_count"].clone(), last["next_offset"].clone()),
                (json!(100), Value::Null),
                "{kind:?}"
            );
            assert_eq!(last["rows"][99], json!([999]));
            assert_eq!(last["truncated"], json!(false));
            // Anything that could change data is refused, however it is dressed up.
            for sql in [
                "DELETE FROM t",
                "SELECT 1; DROP TABLE t",
                "WITH x AS (SELECT 1 AS a) UPDATE t SET a = 1",
            ] {
                assert_eq!(
                    run(sql, None).await.unwrap_err(),
                    WRITE_REFUSED,
                    "{kind:?}: {sql}"
                );
            }
            // What an AI app ran is in the activity log, under its name, with why it was refused.
            for sql in ["SELECT 1 AS n", "DELETE FROM t"] {
                let mut log = server.activity("Claude Code", "run_query", &cfg.id, sql);
                let p = QueryParams {
                    connection_id: cfg.id.clone(),
                    database: None,
                    sql: sql.into(),
                    max_rows: None,
                    offset: None,
                };
                let result = server.query(p, &mut log).await;
                server.record(log, &result);
            }
            let log = server.state.store.app_activity(2).unwrap();
            assert_eq!(
                (log[0].status.as_str(), log[0].detail.as_deref()),
                ("refused", Some("could change data")),
                "{kind:?}"
            );
            assert_eq!(
                (log[1].app.as_str(), log[1].status.as_str(), log[1].rows),
                ("Claude Code", "ok", Some(1)),
                "{kind:?}"
            );
            assert_eq!(log[1].connection, cfg.name);

            // export_csv writes to Downloads; check what it writes without writing there.
            let pool = server
                .state
                .target_pool(&Target::default_of(&cfg.id))
                .await
                .unwrap();
            let r = db::run_query_with(
                &pool,
                THOUSAND,
                true,
                EXPORT_ROWS,
                &CancellationToken::new(),
            )
            .await
            .unwrap();
            let csv = to_csv(&r.columns, &r.rows);
            assert_eq!(csv.lines().count(), 1001, "{kind:?}");
            assert!(
                csv.starts_with("v\n0\n1\n") && csv.ends_with("999\n"),
                "{kind:?}"
            );
            let _ = std::fs::remove_dir_all(dir);
        }
        let _ = std::fs::remove_dir_all(demo_dir);
    }

    /// Every repro from the governance test (2026-10-28) against a real PostgreSQL, when
    /// SAYQL_TEST_PG_URL is set: refused, or answered without the values.
    #[tokio::test]
    async fn governance_repros_on_postgres() {
        let Some((cfg, password)) = db::env_config("SAYQL_TEST_PG_URL", Kind::Postgres) else {
            return;
        };
        let dir = std::env::temp_dir().join(format!("sayql-mcp-gov-{}", uuid::Uuid::new_v4()));
        let state = AppState::new(dir.clone(), dir.clone()).unwrap();
        state
            .store
            .save_connections(std::slice::from_ref(&cfg))
            .unwrap();
        let pool = db::connect(&cfg, password).await.unwrap();
        for sql in [
            "DROP VIEW IF EXISTS gov_contacts",
            "DROP TABLE IF EXISTS gov_customers",
            "CREATE TABLE gov_customers (id int PRIMARY KEY, name text, email text, ssn text, salary int, \
             date_of_birth date, secret_code text)",
            "INSERT INTO gov_customers VALUES (1, 'Alice Smith', 'alice@example.com', '123-45-6789', 91000, '1985-03-14', 'zebra'), \
             (2, 'Bob Jones', 'bob@example.com', '987-65-4321', 72000, '1990-07-01', 'tiger'), \
             (3, 'Chandra Rao', 'chandra@example.in', '111-22-3333', 55000, '1979-12-25', 'koala')",
            "CREATE VIEW gov_contacts AS SELECT id, ssn AS ref, salary AS pay FROM gov_customers",
        ] {
            db::run_query(&pool, sql, false).await.unwrap();
        }
        state.set_pool(Target::default_of(&cfg.id), pool.clone());
        let server = SayqlServer::new(state);
        let set = |apps: AppResults| {
            let a = AgentAccess {
                apps,
                never_send: vec![
                    "ssn".into(),
                    "gov_customers.salary".into(),
                    "secret_code".into(),
                ],
                ..Default::default()
            };
            server.state.store.save_agent_access(&cfg.id, a).unwrap();
        };
        let run = |sql: &str| {
            let p = QueryParams {
                connection_id: cfg.id.clone(),
                database: None,
                sql: sql.into(),
                max_rows: None,
                offset: None,
            };
            let server = &server;
            async move {
                let mut log = Activity::new("test", "run_query");
                let r = server.query(p, &mut log).await;
                let text = match &r {
                    Ok(t) | Err(t) => t.clone(),
                };
                (r.is_ok(), text, serde_json::to_string(&log).unwrap())
            }
        };
        let never_values = [
            "123-45-6789",
            "987-65",
            "91000",
            "72000",
            "zebra",
            "tiger",
            "koala",
        ];

        // C2: never-send values in any form, at every level, and in export_csv.
        for level in [AppResults::All, AppResults::Masked, AppResults::Counts] {
            set(level);
            for sql in [
                "SELECT row_to_json(c) AS r FROM gov_customers c WHERE id=1",
                "SELECT c::text AS r FROM gov_customers c WHERE id=1",
                "SELECT json_agg(c) AS j FROM gov_customers c",
                "SELECT to_jsonb(c)->>('sec'||'ret_code') AS v FROM gov_customers c",
                r#"SELECT U&"\0073sn" AS v FROM gov_customers"#,
                "SELECT t.c4 FROM (SELECT * FROM gov_customers) AS t(c1,c2,c3,c4,c5,c6,c7)",
                "SELECT sal FROM gov_customers c, LATERAL (SELECT c.*) x(i,n,e,s,sal,dob,sc)",
                "SELECT key, value FROM gov_customers c, jsonb_each_text(to_jsonb(c)) WHERE c.id=1",
                "SELECT id FROM gov_customers c WHERE c::text LIKE '%123-45%'",
                "SELECT (to_jsonb(c)->>('sal'||'ary'))::int AS v FROM gov_customers c",
                "SELECT max(pay) AS v FROM gov_contacts",
                "SELECT id FROM gov_contacts WHERE ref LIKE '1%'",
                "WITH x AS (SELECT * FROM gov_contacts) SELECT * FROM x",
                "SELECT 'x' AS a UNION ALL SELECT ref FROM gov_contacts",
            ] {
                let (_, text, log) = run(sql).await;
                for v in never_values {
                    assert!(
                        !text.contains(v) && !log.contains(v),
                        "{level:?} {sql}: {text}"
                    );
                }
            }
            // A never-send column passed straight through a view is withheld, not shown.
            let (ok, text, _) = run("SELECT id, ref, pay FROM gov_contacts").await;
            assert!(ok, "{level:?}: {text}");
            for v in never_values {
                assert!(!text.contains(v), "{level:?}: {text}");
            }
            let (ok, text, _) = run("SELECT * FROM gov_customers").await;
            assert!(ok, "{level:?}: {text}");
            for v in never_values {
                assert!(!text.contains(v), "{level:?}: {text}");
            }
        }
        set(AppResults::All);
        let p = ExportParams {
            connection_id: cfg.id.clone(),
            database: None,
            sql: "SELECT row_to_json(c) AS r FROM gov_customers c".into(),
            file_name: None,
        };
        assert!(
            server
                .export(p, &mut Activity::new("test", "export_csv"))
                .await
                .is_err()
        );
        // L7: server files, whatever the login.
        assert!(!run("SELECT pg_read_file('/etc/hosts')").await.0);
        assert!(!run("SELECT * FROM pg_ls_dir('.') d").await.0);
        // Ordinary queries still run.
        for sql in [
            "SELECT count(*) FROM gov_customers",
            "SELECT c.id, c.name, c.email FROM gov_customers c WHERE c.id > 1 ORDER BY c.name",
            "SELECT count(*) FROM gov_customers WHERE id IN (SELECT id FROM gov_customers WHERE name LIKE 'A%')",
        ] {
            let (ok, text, _) = run(sql).await;
            assert!(ok, "{sql}: {text}");
        }

        // C1: errors don't carry values below all (nor at all near never-send columns), nor does the log.
        for level in [
            AppResults::Off,
            AppResults::Counts,
            AppResults::Masked,
            AppResults::All,
        ] {
            set(level);
            for sql in [
                "SELECT current_setting(email) FROM gov_customers WHERE id=3",
                "SELECT convert_from(name::bytea, name) FROM gov_customers",
                "SELECT xmlparse(document name) FROM gov_customers",
                "SELECT email::regclass FROM gov_customers",
                "SELECT name::regtype FROM gov_customers",
                "SELECT email::json FROM gov_customers",
                "SELECT email::int FROM gov_customers",
            ] {
                let (ok, text, log) = run(sql).await;
                assert!(!ok, "{sql}");
                for v in ["alice", "Alice", "Smith", "chandra", "Chandra"] {
                    assert!(!log.contains(v), "{level:?} {sql}: log {log}");
                    if level != AppResults::All {
                        assert!(!text.contains(v), "{level:?} {sql}: {text}");
                    }
                }
            }
            let (_, text, _) = run("SELECT nme FROM gov_customers").await;
            assert!(
                text.contains("nme") && text.contains("does not exist"),
                "{text}"
            );
        }

        // H1, M1: no min/max for renamed personal columns or computed numbers at counts.
        set(AppResults::Counts);
        for sql in [
            "SELECT date_of_birth AS d FROM gov_customers",
            "SELECT date_of_birth + 0 AS d FROM gov_customers",
            "SELECT extract(year FROM date_of_birth) AS d FROM gov_customers",
            "SELECT ('x'||encode(substr(email,1,7)::bytea,'hex'))::bit(56)::bigint AS d FROM gov_customers WHERE id=1",
        ] {
            let (ok, text, _) = run(sql).await;
            assert!(ok, "{sql}: {text}");
            let v: Value = serde_json::from_str(&text).unwrap();
            assert!(
                v["summary"]["columns"][0].get("max").is_none(),
                "{sql}: {text}"
            );
        }
        let (_, text, _) = run("SELECT max(id) AS top FROM gov_customers").await;
        let v: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v["summary"]["columns"][0]["max"], json!(3), "{text}");

        // H2: names at masked.
        set(AppResults::Masked);
        for sql in [
            "SELECT t.a FROM (SELECT name FROM gov_customers) t(a)",
            "SELECT a FROM gov_customers c, LATERAL (VALUES (c.name)) v(a)",
            "SELECT (c).name AS q FROM gov_customers c",
            "SELECT x FROM unnest(ARRAY(SELECT name FROM gov_customers)) x",
            "SELECT to_jsonb(c)->>'name' AS v FROM gov_customers c",
            "SELECT 'x' AS a UNION ALL SELECT name FROM gov_customers",
            "SELECT 1 AS id, 'x' AS label UNION ALL SELECT id, name FROM gov_customers",
        ] {
            let (_, text, _) = run(sql).await;
            for v in ["Alice", "Bob", "Chandra"] {
                assert!(!text.contains(v), "{sql}: {text}");
            }
        }

        for sql in ["DROP VIEW gov_contacts", "DROP TABLE gov_customers"] {
            db::run_query(&pool, sql, false).await.unwrap();
        }
        let _ = std::fs::remove_dir_all(dir);
    }
}
