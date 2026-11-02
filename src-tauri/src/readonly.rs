//! Read-only access for AI clients: the pieces `sayql-mcp` and agent mode share, so both
//! keep the same safety rules. Schema search, the write refusal, and queries that SayQL
//! stops on the server when they run too long.

use crate::db::usage::Stats;
use crate::db::{self, DbPool, QueryResult};
use crate::schema::{self, Schema, Table};
use std::time::Duration;
use tokio_util::sync::CancellationToken;

/// Rows a query tool returns when the caller doesn't say.
pub const DEFAULT_ROWS: usize = 100;
/// Most rows a query tool returns at once.
pub const MAX_ROWS: usize = 1000;
/// A schema bigger than this is listed by table name unless a search narrows it.
const FULL_SCHEMA_CHARS: usize = 60_000;

pub const WRITE_REFUSED: &str = "SayQL's MCP access is read-only, and this statement could change data. \
Use open_query to put it in a SayQL tab, where the user reviews and runs it.";

/// The same refusal for agent mode, which has no tab to hand the statement to.
pub const AGENT_WRITE_REFUSED: &str = "SayQL's agent is read-only, and this statement could change data, so it was not run. \
If the answer is a change to the data, put the statement in finish as proposed_sql for the person to review.";

/// What a query stopped by its caller (agent mode's Stop) returns.
pub const STOPPED: &str = "Stopped.";

/// Runs a read-only query, stopping it on the server if it runs longer than `limit`.
pub async fn run_limited(
    pool: &DbPool,
    sql: &str,
    cap: usize,
    limit: Duration,
) -> Result<QueryResult, String> {
    run_limited_with(pool, sql, cap, limit, &CancellationToken::new()).await
}

/// `run_limited` that also stops, on the server too, when `stop` fires.
pub async fn run_limited_with(
    pool: &DbPool,
    sql: &str,
    cap: usize,
    limit: Duration,
    stop: &CancellationToken,
) -> Result<QueryResult, String> {
    run_limited_checked(pool, sql, cap, limit, stop)
        .await
        .map_err(|f| f.full)
}

/// Why a query failed: the database's full message, and the form an AI that may not see
/// row values gets (see `ai::redact::safe_error`).
#[derive(Debug, Clone)]
pub struct QueryFailure {
    pub full: String,
    pub safe: String,
}

impl QueryFailure {
    /// A message SayQL wrote itself, the same in both forms.
    pub fn own(message: impl Into<String>) -> Self {
        let m = message.into();
        QueryFailure {
            full: m.clone(),
            safe: m,
        }
    }
}

/// `run_limited_with`, keeping the safe form of the error too.
pub async fn run_limited_checked(
    pool: &DbPool,
    sql: &str,
    cap: usize,
    limit: Duration,
    stop: &CancellationToken,
) -> Result<QueryResult, QueryFailure> {
    let cancel = stop.child_token();
    let timer = tokio::spawn({
        let cancel = cancel.clone();
        async move {
            tokio::time::sleep(limit).await;
            cancel.cancel();
        }
    });
    let out = db::run_query_with(pool, sql, true, cap, &cancel).await;
    timer.abort();
    out.map_err(|e| match e.to_string() {
        m if m == db::CANCELLED && stop.is_cancelled() => QueryFailure::own(STOPPED),
        m if m == db::CANCELLED => QueryFailure::own(format!(
            "The query was still running after {} seconds, so SayQL stopped it. Narrow it (a WHERE clause, \
             fewer joins, LIMIT/TOP) and try again, or run it in the SayQL app.",
            limit.as_secs()
        )),
        m => QueryFailure {
            safe: crate::ai::redact::safe_error(&e, sql),
            full: m,
        },
    })
}

/// The schema as compact text: everything when small, else the tables matching `search`,
/// else just the table names with a hint to search.
pub fn describe(schema: &Schema, search: Option<&str>) -> String {
    describe_with(schema, search, &Stats::None)
}

/// `describe` with each table's usage from the database's statistics (see db/usage.rs)
/// after it, and one line saying what period the figures cover.
pub fn describe_with(schema: &Schema, search: Option<&str>, usage: &Stats) -> String {
    let words: Vec<String> = search
        .unwrap_or("")
        .split_whitespace()
        .map(str::to_lowercase)
        .collect();
    let matches = |t: &Table| {
        words.iter().any(|w| {
            t.qualified_name().to_lowercase().contains(w)
                || t.columns.iter().any(|c| c.name.to_lowercase().contains(w))
        })
    };
    let with_usage = |t: &Table, text: String| match usage.line(t) {
        Some(u) => format!("{text} usage: {u}"),
        None => text,
    };
    let render = |t: &Table| with_usage(t, schema::render_table(t, &schema.dialect));
    let header = format!(
        "Dialect: {}. {} tables and views.",
        schema.dialect,
        schema.tables.len()
    );
    // What the usage figures are and the period they cover, before the tables.
    let about = usage.header().map(|h| format!("{h}\n")).unwrap_or_default();
    if !words.is_empty() {
        let found: Vec<String> = schema
            .tables
            .iter()
            .filter(|t| matches(t))
            .map(render)
            .collect();
        return if found.is_empty() {
            format!(
                "{header}\nNo table or column matches {:?}. Try other words, or omit `search` for the table list.",
                words.join(" ")
            )
        } else {
            format!(
                "{header} Showing {} matching:\n{about}{}",
                found.len(),
                found.join("\n")
            )
        };
    }
    // Whether it fits is judged without the usage, so usage never hides columns.
    if schema::render_full(schema).len() <= FULL_SCHEMA_CHARS {
        let full: Vec<String> = schema.tables.iter().map(render).collect();
        return format!("{header}\n{about}{}", full.join("\n"));
    }
    let names: Vec<String> = schema
        .tables
        .iter()
        .map(|t| {
            let name = match t.row_estimate {
                Some(n) => format!("{} (~{n} rows)", schema::render_name(t, &schema.dialect)),
                None => schema::render_name(t, &schema.dialect),
            };
            with_usage(t, name)
        })
        .collect();
    format!(
        "{header} Too large to describe in full; call again with `search` words for columns.\n{about}{}",
        names.join("\n")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::Column;

    fn table(name: &str, cols: &[&str]) -> Table {
        Table {
            schema: Some("dbo".into()),
            name: name.into(),
            kind: "table".into(),
            columns: cols
                .iter()
                .map(|c| Column {
                    name: (*c).into(),
                    data_type: "int".into(),
                    nullable: false,
                    primary_key: false,
                    ..Default::default()
                })
                .collect(),
            foreign_keys: vec![],
            row_estimate: Some(10),
            ..Default::default()
        }
    }

    #[test]
    fn describe_filters_by_search_words() {
        let s = Schema {
            dialect: "SQL Server".into(),
            tables: vec![
                table("Person", &["PERSON_EMAIL"]),
                table("Account", &["ID"]),
            ],
        };
        let out = describe(&s, Some("email"));
        assert!(
            out.contains("dbo.Person") && !out.contains("dbo.Account"),
            "{out}"
        );
        assert!(describe(&s, Some("nothing")).contains("No table or column matches"));
        let all = describe(&s, None);
        assert!(
            all.contains("dbo.Person") && all.contains("dbo.Account"),
            "{all}"
        );
    }

    #[test]
    fn describe_adds_usage_after_each_table() {
        use crate::db::usage::{Stats, UNAVAILABLE, parse};
        let s = Schema {
            dialect: "SQL Server".into(),
            tables: vec![table("Person", &["ID"]), table("Account", &["ID"])],
        };
        let r = QueryResult {
            columns: ["started", "schema_name", "table_name", "reads", "writes"]
                .map(String::from)
                .to_vec(),
            rows: vec![vec![
                serde_json::json!("2026-10-01T00:00:00"),
                serde_json::json!("dbo"),
                serde_json::json!("Person"),
                serde_json::json!(50_000),
                serde_json::json!(3),
            ]],
            rows_affected: None,
            truncated: false,
            elapsed_ms: 1,
            withheld: vec![],
        };
        let out = describe_with(&s, None, &parse(crate::db::Kind::Mssql, &r));
        assert!(
            out.contains("since the server started on 2026-10-01"),
            "{out}"
        );
        assert!(out.contains("usage: 50.0k reads, 3 writes"), "{out}");
        assert!(
            out.lines()
                .any(|l| l.starts_with("dbo.Account") && l.ends_with("usage: no reads or writes")),
            "{out}"
        );
        let found = describe_with(&s, Some("person"), &Stats::Unavailable);
        assert!(
            found.contains(UNAVAILABLE) && !found.contains("Account"),
            "{found}"
        );
        assert_eq!(describe_with(&s, None, &Stats::None), describe(&s, None));
    }

    #[test]
    fn describe_lists_names_for_huge_schemas() {
        let cols: Vec<String> = (0..60).map(|i| format!("column_{i}")).collect();
        let cols: Vec<&str> = cols.iter().map(String::as_str).collect();
        let tables = (0..200)
            .map(|i| table(&format!("table_{i}"), &cols))
            .collect();
        let out = describe(
            &Schema {
                dialect: "SQL Server".into(),
                tables,
            },
            None,
        );
        assert!(
            out.contains("Too large")
                && out.contains("dbo.table_199 (~10 rows)")
                && !out.contains("column_0"),
            "{}",
            &out[..200]
        );
    }
}
