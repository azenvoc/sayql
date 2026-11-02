use super::decode::{lite_cell, ms_cell, my_cell, pg_cell};
use super::mssql_pool::MssqlClient;
use super::{DbPool, MAX_ROWS, MssqlPool, QueryResult};
use crate::error::AppResult;
use futures_util::TryStreamExt;
use serde_json::Value;
use sqlx::{AssertSqlSafe, Column as _, Either, Executor, Row, SqlSafeStr, Statement};
use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use tiberius::Command;
use tokio_util::sync::CancellationToken;

/// The error of a query stopped with its `CancellationToken`. The UI gets it as
/// `QUERY_CANCELLED` from the generated src/api/bindings.ts.
pub const CANCELLED: &str = "Query cancelled.";

/// How long a cancelled query gets to stop before its connection is closed anyway.
const STOP_GRACE: Duration = Duration::from_secs(5);

pub async fn run_query(pool: &DbPool, sql: &str, read_only: bool) -> AppResult<QueryResult> {
    run_query_capped(pool, sql, read_only, MAX_ROWS).await
}

/// `run_query`, collecting up to `cap` rows instead of MAX_ROWS (for exports).
pub async fn run_query_capped(
    pool: &DbPool,
    sql: &str,
    read_only: bool,
    cap: usize,
) -> AppResult<QueryResult> {
    run_query_with(pool, sql, read_only, cap, &CancellationToken::new()).await
}

/// `run_query_capped` that can be stopped: when `cancel` fires, the database is asked to stop
/// the statement (it would otherwise run on after SayQL stops waiting), the connection is
/// closed, and the result is the `CANCELLED` error.
pub async fn run_query_with(
    pool: &DbPool,
    sql: &str,
    read_only: bool,
    cap: usize,
    cancel: &CancellationToken,
) -> AppResult<QueryResult> {
    let sql = sql.trim().trim_end_matches(';').trim();
    if sql.is_empty() {
        return Err("Nothing to run.".into());
    }
    // A team server checks every statement itself and records refused ones in its audit log,
    // so a write goes there to be refused rather than being stopped here.
    if let DbPool::Team(t) = pool {
        return t.query(sql, cap, cancel).await;
    }
    if read_only && !super::is_read_only(sql, pool.kind()) {
        return Err("This statement can change data, so it can't run in read-only mode.".into());
    }
    let start = Instant::now();
    // A confirmed statement can leave session state behind: a SET, USE, LOCK TABLES, temp
    // table or session lock. Its connection is closed afterwards instead of going back to the
    // pool, so none of that carries over to later queries (a held LOCK TABLES would make
    // them wait forever).
    let mut res = match pool {
        DbPool::Pg(p) => {
            let mut conn = p.acquire().await?;
            // Every statement runs inside a transaction, which a pooler (Supabase on port 6543, PgBouncer)
            // keeps on one server connection, starting with DEALLOCATE ALL so a statement name another
            // client left on that server can't collide. A confirmed write commits at the end, except the
            // few statements Postgres refuses inside a transaction (VACUUM and the like), run on their own.
            let own = !read_only && pg_needs_autocommit(sql);
            // The server process's id, to cancel from another connection; in the same round trip.
            let begin = match (read_only, own) {
                (true, _) => "BEGIN READ ONLY; DEALLOCATE ALL; SELECT pg_backend_pid()",
                (false, false) => "BEGIN; DEALLOCATE ALL; SELECT pg_backend_pid()",
                (false, true) => "DEALLOCATE ALL; SELECT pg_backend_pid()",
            };
            let pid: i32 = sqlx::raw_sql(begin)
                .fetch_all(&mut *conn)
                .await?
                .first()
                .map(|r| r.try_get(0))
                .transpose()?
                .unwrap_or(0);
            let out = {
                let mut query = Box::pin(collect_pg(&mut conn, sql, cap));
                match race(&mut query, cancel).await {
                    Some(out) => Some(out),
                    None => {
                        // One plain-text message, so it works behind a pooler too; pid is a number from the server.
                        let _ = sqlx::raw_sql(AssertSqlSafe(format!(
                            "SELECT pg_cancel_backend({pid})"
                        )))
                        .execute(p)
                        .await;
                        let _ = tokio::time::timeout(STOP_GRACE, &mut query).await;
                        None
                    }
                }
            };
            let Some(out) = out else {
                let _ = conn.close().await;
                return Err(CANCELLED.into());
            };
            if read_only {
                if conn.execute("ROLLBACK").await.is_err() {
                    // Never hand a connection with an open transaction back to the pool.
                    let _ = conn.close().await;
                }
                out?
            } else {
                // A COMMIT can still fail (a deferred constraint); that error is the result then.
                // Behind a pooler the server connection outlives ours, so session state a confirmed
                // statement set (SET, advisory locks) is undone before committing.
                let end = if own {
                    Ok(())
                } else if out.is_ok() {
                    sqlx::raw_sql("RESET ALL; SELECT pg_advisory_unlock_all(); COMMIT")
                        .execute(&mut *conn)
                        .await
                        .map(|_| ())
                } else {
                    conn.execute("ROLLBACK").await.map(|_| ())
                };
                let _ = conn.close().await;
                let out = out?;
                end?;
                out
            }
        }
        DbPool::My(p) => {
            let mut conn = p.acquire().await?;
            if read_only {
                conn.execute("START TRANSACTION READ ONLY").await?;
            }
            // The connection's id, to stop the query from another connection with KILL QUERY.
            let id: u64 = sqlx::query_scalar("SELECT CONNECTION_ID()")
                .fetch_one(&mut *conn)
                .await?;
            let out = {
                let mut query = Box::pin(collect_my(&mut conn, sql, cap));
                match race(&mut query, cancel).await {
                    Some(out) => Some(out),
                    None => {
                        // KILL can't be a prepared statement; `id` is the server's own number.
                        let kill = format!("KILL QUERY {id}");
                        let _ = sqlx::raw_sql(AssertSqlSafe(kill)).execute(p).await;
                        let _ = tokio::time::timeout(STOP_GRACE, &mut query).await;
                        None
                    }
                }
            };
            let Some(out) = out else {
                let _ = conn.close().await;
                return Err(CANCELLED.into());
            };
            if !read_only || conn.execute("ROLLBACK").await.is_err() {
                let _ = conn.close().await;
            }
            out?
        }
        DbPool::Lite(p) => {
            let mut conn = if read_only {
                p.ro.acquire()
            } else {
                p.rw.acquire()
            }
            .await?;
            // SQLite runs in-process: a progress handler checked every few thousand steps
            // interrupts the statement once `stop` is set.
            let stop = Arc::new(AtomicBool::new(false));
            let flag = stop.clone();
            conn.lock_handle()
                .await?
                .set_progress_handler(4000, move || !flag.load(Ordering::Relaxed));
            let out = {
                let mut query = Box::pin(collect_lite(&mut conn, sql, cap));
                match race(&mut query, cancel).await {
                    Some(out) => Some(out),
                    None => {
                        stop.store(true, Ordering::Relaxed);
                        let _ = tokio::time::timeout(STOP_GRACE, &mut query).await;
                        None
                    }
                }
            };
            let Some(out) = out else {
                let _ = conn.close().await;
                return Err(CANCELLED.into());
            };
            if !read_only {
                let _ = conn.close().await;
            }
            out?
        }
        DbPool::Ms(p) => Box::pin(run_mssql(p, sql, read_only, cap, cancel)).await?,
        DbPool::Team(t) => return t.query(sql, cap, cancel).await,
    };
    res.elapsed_ms = start.elapsed().as_millis();
    Ok(res)
}

/// Statements Postgres refuses inside a transaction block, so a confirmed one runs on its own.
fn pg_needs_autocommit(sql: &str) -> bool {
    let words: Vec<String> = sql
        .split_whitespace()
        .take(4)
        .map(|w| w.to_ascii_uppercase())
        .collect();
    let w = |i: usize| words.get(i).map(String::as_str).unwrap_or("");
    match w(0) {
        "VACUUM" | "CLUSTER" => true,
        "CREATE" | "DROP" => {
            matches!(w(1), "DATABASE" | "TABLESPACE" | "SUBSCRIPTION")
                || words.iter().any(|x| x == "CONCURRENTLY")
        }
        "REINDEX" => {
            w(1) == "SYSTEM" || w(1) == "DATABASE" || words.iter().any(|x| x == "CONCURRENTLY")
        }
        "ALTER" => {
            (w(1) == "SYSTEM")
                || (w(1) == "TYPE" && words.iter().any(|x| x == "ADD"))
                || w(1) == "SUBSCRIPTION"
        }
        _ => false,
    }
}

/// Runs one statement, collecting up to `cap` rows and the affected-row count.
/// A macro rather than a generic fn: generic sqlx executors make the future
/// non-`Send` across the Tauri command boundary.
macro_rules! collect_fn {
    ($name:ident, $conn:ty, $cell:ident, $first_result_only:expr) => {
        async fn $name(conn: &mut $conn, sql: &str, cap: usize) -> AppResult<QueryResult> {
            // Running the user's own SQL is the point; read-only mode is enforced around this call.
            let sql = AssertSqlSafe(sql).into_sql_str();
            let mut columns: Vec<String> = Vec::new();
            let mut rows: Vec<Vec<Value>> = Vec::new();
            let mut affected: Option<u64> = None;
            let mut truncated = false;
            {
                #[allow(
                    deprecated,
                    reason = "fetch_many is sqlx's one call that returns rows and affected counts together"
                )]
                let mut stream = sqlx::query(sql.clone()).fetch_many(&mut *conn);
                while let Some(item) = stream.try_next().await? {
                    match item {
                        Either::Left(r) => {
                            affected = Some(affected.unwrap_or(0) + r.rows_affected());
                            // SQLite runs every statement of a script in one call; show the first
                            // result set only, as on the other databases. (Postgres and MySQL
                            // prepare one statement, and stopping early here would leave MySQL's
                            // last packet unread.)
                            if $first_result_only && !columns.is_empty() {
                                break;
                            }
                        }
                        Either::Right(row) => {
                            if columns.is_empty() {
                                columns =
                                    row.columns().iter().map(|c| c.name().to_string()).collect();
                            }
                            if rows.len() >= cap {
                                truncated = true;
                                break;
                            }
                            rows.push((0..row.columns().len()).map(|i| $cell(&row, i)).collect());
                        }
                    }
                }
            }
            if columns.is_empty() && rows.is_empty() {
                // No rows came back; still show the column headers if the statement has any.
                if let Ok(stmt) = (&mut *conn).prepare(sql).await {
                    columns = stmt
                        .columns()
                        .iter()
                        .map(|c| c.name().to_string())
                        .collect();
                }
            }
            let rows_affected = if columns.is_empty() { affected } else { None };
            Ok(QueryResult {
                columns,
                rows,
                rows_affected,
                truncated,
                elapsed_ms: 0,
                withheld: vec![],
            })
        }
    };
}

collect_fn!(collect_pg, sqlx::PgConnection, pg_cell, false);
collect_fn!(collect_my, sqlx::MySqlConnection, my_cell, false);
collect_fn!(collect_lite, sqlx::SqliteConnection, lite_cell, true);

/// SQL Server has no read-only transaction mode. Reads run inside a transaction that is
/// always rolled back, on top of the keyword check in `classify` that sends anything
/// containing a write keyword to the confirmation path instead.
async fn run_mssql(
    pool: &MssqlPool,
    sql: &str,
    read_only: bool,
    cap: usize,
    cancel: &CancellationToken,
) -> AppResult<QueryResult> {
    let mut conn = pool.acquire().await?;
    if !read_only {
        // sp_executesql reports both the rows a statement returns (INSERT ... OUTPUT,
        // DECLARE ...; SELECT) and how many rows it changed. It reads every row, which is fine
        // for statements the person confirmed one by one.
        let mut cmd = Command::new("sp_executesql");
        cmd.bind_param("@stmt", sql.to_string());
        let out = {
            let mut query = Box::pin(async {
                match cmd.exec(conn.client()).await {
                    Ok(stream) => stream.into_command_result().await,
                    Err(e) => Err(e),
                }
            });
            race(&mut query, cancel).await
        };
        let Some(out) = out else {
            stop_mssql(conn).await;
            return Err(CANCELLED.into());
        };
        // Like the other databases: a fresh connection afterwards, so no session state carries over.
        conn.discard();
        let out = out?;
        let affected: u64 = out.rows_affected().iter().sum();
        let first = out.into_iter().next().unwrap_or_default();
        let columns: Vec<String> = first
            .first()
            .map(|r| r.columns().iter().map(|c| c.name().to_string()).collect())
            .unwrap_or_default();
        let truncated = first.len() > cap;
        let kept = &first[..first.len().min(cap)];
        let rows = kept
            .iter()
            .map(|row| (0..row.len()).map(|i| ms_cell(row, i)).collect())
            .collect();
        let udts = first
            .first()
            .map(|r| udt_columns(r.columns()))
            .unwrap_or_default();
        let udts = udts
            .into_iter()
            .map(|c| (c, kept.iter().map(|row| udt_bytes(row, c)).collect()))
            .collect();
        let rows_affected = if columns.is_empty() {
            Some(affected)
        } else {
            None
        };
        let mut res = QueryResult {
            columns,
            rows,
            rows_affected,
            truncated,
            elapsed_ms: 0,
            withheld: vec![],
        };
        show_udts(pool, sql, &mut res, udts).await;
        return Ok(res);
    }

    conn.client()
        .simple_query("BEGIN TRANSACTION")
        .await?
        .into_results()
        .await?;
    let out = {
        let mut query = Box::pin(collect_mssql(conn.client(), sql, cap));
        race(&mut query, cancel).await
    };
    let Some(out) = out else {
        stop_mssql(conn).await;
        return Err(CANCELLED.into());
    };
    // A cut-off result leaves unread rows on the wire, and a failed rollback leaves the
    // transaction open: either way, close the connection (the server rolls back).
    let cut_off = out.as_ref().map(|(r, _)| r.truncated).unwrap_or(false);
    let rolled_back = !cut_off && rollback(conn.client()).await.is_ok();
    if rolled_back {
        drop(conn);
    } else {
        conn.discard();
    }
    let (mut res, udts) = out?;
    show_udts(pool, sql, &mut res, udts).await;
    Ok(res)
}

/// The raw values of a result's CLR-typed columns (geography, geometry, hierarchyid), by
/// column, one per row.
type Udts = Vec<(usize, Vec<Option<Vec<u8>>>)>;

fn udt_columns(columns: &[tiberius::Column]) -> Vec<usize> {
    columns
        .iter()
        .enumerate()
        .filter(|(_, c)| c.column_type() == tiberius::ColumnType::Udt)
        .map(|(i, _)| i)
        .collect()
}

fn udt_bytes(row: &tiberius::Row, i: usize) -> Option<Vec<u8>> {
    match row.cells().nth(i) {
        Some((_, tiberius::ColumnData::Binary(Some(b)))) => Some(b.to_vec()),
        _ => None,
    }
}

/// geography, geometry and hierarchyid values arrive as bytes, and the driver doesn't say
/// which type a column is. Asks SQL Server what each column is, then has it write the values
/// as text, the way its own tools show them (`STAsText()`, `ToString()`). Anything that fails
/// keeps what `ms_cell` showed.
async fn show_udts(pool: &MssqlPool, sql: &str, res: &mut QueryResult, udts: Udts) {
    if udts.is_empty() || res.rows.is_empty() {
        return;
    }
    let Ok(mut conn) = pool.acquire().await else {
        return;
    };
    if convert_udts(conn.client(), sql, res, udts).await.is_err() {
        // It may have stopped mid-result: start the next query on a fresh connection.
        conn.discard();
    }
}

/// Most bytes of values converted in one round trip; a bigger single value keeps its bytes.
const UDT_BATCH_BYTES: usize = 1 << 20;

async fn convert_udts(
    client: &mut MssqlClient,
    sql: &str,
    res: &mut QueryResult,
    udts: Udts,
) -> AppResult<()> {
    // Describes the statement without running it.
    let described = client
        .query("EXEC sp_describe_first_result_set @tsql = @P1", &[&sql])
        .await?
        .into_first_result()
        .await?;
    let type_of = |col: usize| {
        described.iter().find_map(|r| {
            let ordinal: i32 = r.try_get("column_ordinal").ok()??;
            let ty: &str = r.try_get("system_type_name").ok()??;
            (usize::try_from(ordinal - 1).ok()? == col).then(|| ty.to_ascii_lowercase())
        })
    };
    for (col, values) in udts {
        let to_text = match type_of(col).as_deref() {
            Some("geography") => "CAST(v.b AS geography).STAsText()",
            Some("geometry") => "CAST(v.b AS geometry).STAsText()",
            Some("hierarchyid") => "CAST(v.b AS hierarchyid).ToString()",
            _ => continue,
        };
        let present: Vec<(usize, &Vec<u8>)> = values
            .iter()
            .enumerate()
            .filter_map(|(i, v)| v.as_ref().map(|b| (i, b)))
            .filter(|(_, b)| b.len() <= UDT_BATCH_BYTES)
            .collect();
        // Batches of at most 500 values (VALUES takes 1,000 rows) and about 1 MB.
        let mut start = 0;
        while start < present.len() {
            let mut end = start;
            let mut size = 0;
            while end < present.len()
                && end - start < 500
                && (end == start || size + present[end].1.len() <= UDT_BATCH_BYTES)
            {
                size += present[end].1.len();
                end += 1;
            }
            let list: Vec<String> = present[start..end]
                .iter()
                .map(|(i, b)| {
                    format!(
                        "({i}, 0x{})",
                        b.iter().map(|x| format!("{x:02x}")).collect::<String>()
                    )
                })
                .collect();
            // Only row numbers and hex digits go into this SQL.
            let q = format!(
                "SELECT v.i, {to_text} FROM (VALUES {}) v(i, b)",
                list.join(", ")
            );
            for row in client.simple_query(q).await?.into_first_result().await? {
                let (Some(i), text) = (row.try_get::<i32, _>(0)?, row.try_get::<&str, _>(1)?)
                else {
                    continue;
                };
                if let Some(cell) = res.rows.get_mut(i as usize).and_then(|r| r.get_mut(col)) {
                    *cell = text.map_or(Value::Null, |t| Value::String(t.to_string()));
                }
            }
            start = end;
        }
    }
    Ok(())
}

/// Sends SQL Server an attention (the protocol's cancel) for the running batch, then closes
/// the connection; the server rolls back what the batch had done.
async fn stop_mssql(mut conn: super::mssql_pool::MssqlConn<'_>) {
    let _ = tokio::time::timeout(STOP_GRACE, conn.client().cancel_query()).await;
    conn.discard();
}

/// The query's output, or `None` if `cancel` fired first. The query stays usable, so the
/// caller can wait for it to stop. Queries are boxed: their futures are large, and on the
/// stack they overflow a 2 MB thread in debug builds.
async fn race<F: Future + Unpin>(query: &mut F, cancel: &CancellationToken) -> Option<F::Output> {
    tokio::select! {
        out = query => Some(out),
        _ = cancel.cancelled() => None,
    }
}

async fn rollback(client: &mut MssqlClient) -> AppResult<()> {
    client
        .simple_query("IF @@TRANCOUNT > 0 ROLLBACK")
        .await?
        .into_results()
        .await?;
    Ok(())
}

/// Collects the first result set, up to `cap` rows, and the raw values of its CLR-typed columns.
async fn collect_mssql(
    client: &mut MssqlClient,
    sql: &str,
    cap: usize,
) -> AppResult<(QueryResult, Udts)> {
    let mut stream = client.simple_query(sql).await?;
    let (columns, mut udts): (Vec<String>, Udts) = match stream.columns().await? {
        Some(cols) => (
            // An expression without an alias has no name; SQL Server's own tools show this.
            cols.iter()
                .map(|c| {
                    if c.name().is_empty() {
                        "(No column name)".into()
                    } else {
                        c.name().to_string()
                    }
                })
                .collect(),
            udt_columns(cols)
                .into_iter()
                .map(|c| (c, Vec::new()))
                .collect(),
        ),
        None => Default::default(),
    };
    let mut rows = Vec::new();
    let mut truncated = false;
    while let Some(item) = stream.try_next().await? {
        let Some(row) = item.into_row() else { continue };
        if row.result_index() > 0 {
            continue;
        }
        if rows.len() >= cap {
            truncated = true;
            break;
        }
        rows.push((0..row.len()).map(|i| ms_cell(&row, i)).collect());
        for (c, values) in &mut udts {
            values.push(udt_bytes(&row, *c));
        }
    }
    Ok((
        QueryResult {
            columns,
            rows,
            rows_affected: None,
            truncated,
            elapsed_ms: 0,
            withheld: vec![],
        },
        udts,
    ))
}

#[cfg(test)]
mod tests {
    use super::pg_needs_autocommit;

    #[test]
    fn statements_postgres_refuses_in_a_transaction_run_on_their_own() {
        for sql in [
            "VACUUM orders",
            "vacuum analyze",
            "CREATE INDEX CONCURRENTLY i ON t (a)",
            "DROP INDEX CONCURRENTLY i",
            "CREATE DATABASE x",
            "DROP DATABASE x",
            "REINDEX TABLE CONCURRENTLY t",
            "ALTER SYSTEM SET work_mem = '64MB'",
            "ALTER TYPE mood ADD VALUE 'meh'",
            "CLUSTER",
        ] {
            assert!(pg_needs_autocommit(sql), "{sql}");
        }
        for sql in [
            "UPDATE orders SET status = 'x' WHERE id = 1",
            "DELETE FROM t",
            "CREATE INDEX i ON t (a)",
            "CREATE TABLE t (a int)",
            "ALTER TABLE t ADD COLUMN b int",
            "INSERT INTO t VALUES (1)",
        ] {
            assert!(!pg_needs_autocommit(sql), "{sql}");
        }
    }
}
