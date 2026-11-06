//! `run_query`: a read-only query for an AI app, through the AI query gate (safety/gate.rs) with
//! MCP's options, shaped as the connection allows.

use super::shape::{failed, shape};
use super::*;

impl SayqlServer {
    /// Runs a read-only query for an AI app and returns what the connection lets it see.
    pub(super) async fn query(&self, p: QueryParams, log: &mut Activity) -> Result<String, String> {
        let target = self.target_in_reach(&p.connection_id, p.database).await?;
        log.database.clone_from(&target.database);
        let limit = self.limit_for(&target.connection_id)?;
        let access = self.access(&target.connection_id)?;
        log.may_see = Some(access.apps);
        let pool = self
            .state
            .target_pool(&target)
            .await
            .map_err(|e| e.to_string())?;
        // Columns the user marked never to send to an AI are refused in any form, and withheld
        // when a query returns them anyway (SELECT *); see never_send.rs.
        let guard = self
            .checked(&pool, &p.sql, &access.never_send, &limit, log)
            .await?;
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
    pub(super) async fn checked(
        &self,
        pool: &db::DbPool,
        sql: &str,
        never_send: &[String],
        limit: &crate::safety::databases::DbLimit,
        log: &mut Activity,
    ) -> Result<Guard, String> {
        gate::prepare(pool, sql, never_send, limit, gate::MCP)
            .await
            .map_err(|r| match r {
                Refusal::Write => refused(log, "could change data", WRITE_REFUSED.into()),
                // MCP's options leave this to the read-only check, so it doesn't happen.
                Refusal::Statements => refused(
                    log,
                    "more than one statement",
                    "Run one statement at a time.".into(),
                ),
                Refusal::Database(why) => refused(log, "database not available here", why.into()),
                Refusal::NeverSend(f) => {
                    // SayQL's own refusal, or the database rejecting the plan (an error in the SQL).
                    log.status = if f.full == f.safe { "refused" } else { "error" }.into();
                    log.detail = Some(f.safe.chars().take(160).collect());
                    f.safe
                }
            })
    }
}
