//! The AI query gate: the one place SQL is checked before it runs for an AI (the MCP server,
//! agent mode and note checks), and run with a time limit and a redacted error when it fails.
//!
//! `prepare` refuses what may not run (more than one statement, a statement that could change
//! data, never-send columns and functions that read server files); `run` runs it, stopping it
//! on the server after the time limit, and keeps both forms of a failure (the database's own
//! message and `safety::redact::safe_error`'s). Each caller picks what the AI gets.
//!
//! The callers differ on purpose, and the differences are the `Options` each one passes:
//!
//! | Caller        | Writes on a team server's pool              | More than one statement |
//! |---------------|---------------------------------------------|-------------------------|
//! | `MCP`         | refused here, like on every other pool      | left to the read-only check |
//! | `AGENT`       | sent on: the team server refuses and audits | refused                 |
//! | `NOTE_CHECK`  | sent on: the team server refuses and audits | left to the read-only check |
//!
//! Agent mode also puts real values back in place of its tokens (‹email-1›) before calling
//! `prepare`, since that is what runs; the gate checks the SQL exactly as it will run.

use crate::db::{self, DbPool, QueryResult};
use crate::safety::never_send::{self, Guard};
use crate::safety::readonly::{self, QueryFailure};
use std::time::Duration;
use tokio_util::sync::CancellationToken;

/// What a caller's gate refuses beyond the never-send check, which every caller gets.
#[derive(Debug, Clone, Copy)]
pub struct Options {
    /// Refuse a statement that could change data even on a team server's pool. When false, a
    /// team pool gets the statement, refuses it itself and records the attempt in its audit log.
    pub refuse_writes_on_team: bool,
    /// Refuse SQL with more than one statement before any other check.
    pub one_statement: bool,
}

/// `sayql-mcp`'s `run_query` and `export_csv`.
pub const MCP: Options = Options {
    refuse_writes_on_team: true,
    one_statement: false,
};

/// Agent mode's query, show, explain and the SQL it checks for the person.
pub const AGENT: Options = Options {
    refuse_writes_on_team: false,
    one_statement: true,
};

/// A database note's check (`notes::run_check`).
pub const NOTE_CHECK: Options = Options {
    refuse_writes_on_team: false,
    one_statement: false,
};

/// Why `prepare` refused SQL. The caller words the first two for its AI.
#[derive(Debug, Clone)]
pub enum Refusal {
    /// More than one statement, with `Options::one_statement`.
    Statements,
    /// The statement could change data.
    Write,
    /// The never-send or forbidden-function check refused it (`full == safe`), or the database
    /// rejected its plan (an error in the SQL; show `safe`).
    NeverSend(QueryFailure),
}

/// Checks `sql`, exactly as it will run, for an AI. The `Guard` says which result columns to
/// withhold and whether errors must be shown only in their safe form.
pub async fn prepare(
    pool: &DbPool,
    sql: &str,
    never_send: &[String],
    opts: Options,
) -> Result<Guard, Refusal> {
    if opts.one_statement && crate::ai::agent::checks::statements(sql) > 1 {
        return Err(Refusal::Statements);
    }
    let team = matches!(pool, DbPool::Team(_));
    if (opts.refuse_writes_on_team || !team) && !db::is_read_only(sql, pool.kind()) {
        return Err(Refusal::Write);
    }
    // Columns the user marked never to send to an AI are refused in any form (whole rows,
    // renames, views on Postgres), and so are functions that read server files: see never_send.rs.
    never_send::check(pool, sql, never_send)
        .await
        .map_err(Refusal::NeverSend)
}

/// Runs SQL that `prepare` passed (or that SayQL wrote itself), read-only, with at most `cap`
/// rows, stopped on the server after `limit` or when `stop` fires.
pub async fn run(
    pool: &DbPool,
    sql: &str,
    cap: usize,
    limit: Duration,
    stop: &CancellationToken,
) -> Result<QueryResult, QueryFailure> {
    readonly::run_limited_checked(pool, sql, cap, limit, stop).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::tenant_fixture;

    #[tokio::test]
    async fn each_caller_refuses_what_its_options_say() {
        let dir = std::env::temp_dir().join(format!("sayql-gate-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let pool = tenant_fixture::sqlite(&dir.join("tenant.sqlite")).await;
        let none: Vec<String> = vec![];
        let two = "SELECT 1; SELECT 2";
        // Only agent mode refuses more than one statement before the read-only check.
        assert!(matches!(
            prepare(&pool, two, &none, AGENT).await,
            Err(Refusal::Statements)
        ));
        for opts in [MCP, NOTE_CHECK] {
            assert!(!matches!(
                prepare(&pool, two, &none, opts).await,
                Err(Refusal::Statements)
            ));
        }
        // Writes are refused on a pool that isn't a team server's, for every caller.
        for opts in [MCP, AGENT, NOTE_CHECK] {
            assert!(matches!(
                prepare(&pool, "DELETE FROM users", &none, opts).await,
                Err(Refusal::Write)
            ));
        }
        let never = vec!["users.email".to_string()];
        for opts in [MCP, AGENT, NOTE_CHECK] {
            assert!(matches!(
                prepare(&pool, "SELECT email FROM users", &never, opts).await,
                Err(Refusal::NeverSend(_))
            ));
            assert!(
                prepare(&pool, "SELECT count(*) FROM users", &never, opts)
                    .await
                    .is_ok()
            );
        }
        let r = run(
            &pool,
            "SELECT count(*) FROM users",
            1,
            Duration::from_secs(5),
            &CancellationToken::new(),
        )
        .await
        .unwrap();
        assert_eq!(r.rows.len(), 1);
        let _ = std::fs::remove_dir_all(dir);
    }
}
