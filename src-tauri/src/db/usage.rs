//! How much each table is used, from the database's own statistics, so an AI app reading
//! `describe_schema` can tell a live table from an abandoned one.
//!
//! One query per describe call, with a short timeout. Numbers and dates only, never row
//! values. Counters reset when the server restarts or fails over and differ per replica, so
//! the period they cover is always stated. When the statistics can't be read (no permission,
//! performance_schema off, a managed database's limits) the describe says so in one line.
//! SQLite keeps no such statistics, and team connections go through the team server, so
//! neither says anything.

use super::{DbPool, Kind, QueryResult};
use crate::schema::Table;
use serde_json::Value;
use std::collections::HashMap;
use std::time::Duration;

/// How long the statistics query may take before the describe goes on without it.
const TIMEOUT: Duration = Duration::from_secs(5);
/// Most tables read from the statistics.
const MAX_TABLES: usize = 20_000;

pub const UNAVAILABLE: &str =
    "Usage statistics unavailable (the database user can't read them, or they are turned off).";

/// Reads, writes and live rows on PostgreSQL. The period: when the database's statistics
/// were last reset, else when the server started.
const PG_SQL: &str = "SELECT i.stats_reset, i.started, x.schema_name, x.table_name, x.reads, x.writes, x.live_rows, x.analyzed \
FROM (SELECT (SELECT stats_reset::text FROM pg_stat_database WHERE datname = current_database()) AS stats_reset, \
             pg_postmaster_start_time()::text AS started) i \
LEFT JOIN (SELECT schemaname::text AS schema_name, relname::text AS table_name, \
                  (seq_scan + COALESCE(idx_scan, 0))::bigint AS reads, \
                  (n_tup_ins + n_tup_upd + n_tup_del)::bigint AS writes, \
                  n_live_tup::bigint AS live_rows, \
                  GREATEST(last_autoanalyze, last_analyze)::text AS analyzed \
           FROM pg_stat_user_tables) x ON true";

/// Index usage on SQL Server since it started. A table missing from the view wasn't used.
const MSSQL_SQL: &str = "SELECT i.started, x.schema_name, x.table_name, x.reads, x.writes, x.last_read, x.last_write \
FROM (SELECT CONVERT(varchar(33), sqlserver_start_time, 126) AS started FROM sys.dm_os_sys_info) i \
LEFT JOIN (SELECT s.name AS schema_name, o.name AS table_name, \
                  SUM(u.user_seeks + u.user_scans + u.user_lookups) AS reads, SUM(u.user_updates) AS writes, \
                  CONVERT(varchar(33), MAX(r.last_read), 126) AS last_read, \
                  CONVERT(varchar(33), MAX(u.last_user_update), 126) AS last_write \
           FROM sys.dm_db_index_usage_stats u \
           CROSS APPLY (SELECT MAX(v) AS last_read FROM (VALUES (u.last_user_seek), (u.last_user_scan), (u.last_user_lookup)) t(v)) r \
           JOIN sys.objects o ON o.object_id = u.object_id \
           JOIN sys.schemas s ON s.schema_id = o.schema_id \
           WHERE u.database_id = DB_ID() AND o.type = 'U' \
           GROUP BY s.name, o.name) x ON 1 = 1";

/// Row reads and writes from performance_schema since the server started, and the last
/// write time information_schema keeps.
const MYSQL_SQL: &str = "SELECT (SELECT CAST(VARIABLE_VALUE AS CHAR) FROM performance_schema.global_status WHERE VARIABLE_NAME = 'Uptime') AS uptime, \
       CAST(t.TABLE_SCHEMA AS CHAR) AS schema_name, CAST(t.TABLE_NAME AS CHAR) AS table_name, \
       w.COUNT_READ AS reads, w.COUNT_WRITE AS writes, CAST(t.UPDATE_TIME AS CHAR) AS last_write \
FROM information_schema.TABLES t \
LEFT JOIN performance_schema.table_io_waits_summary_by_table w ON w.OBJECT_SCHEMA = t.TABLE_SCHEMA AND w.OBJECT_NAME = t.TABLE_NAME \
WHERE t.TABLE_SCHEMA = DATABASE() AND t.TABLE_TYPE = 'BASE TABLE'";

/// What one table's statistics say.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Counts {
    pub reads: Option<i64>,
    pub writes: Option<i64>,
    pub live_rows: Option<i64>,
    /// Dates as `YYYY-MM-DD`.
    pub last_read: Option<String>,
    pub last_write: Option<String>,
    pub analyzed: Option<String>,
}

/// The statistics of one database.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Usage {
    /// The period the counters cover, e.g. "since 2026-01-01" or "since the server started on 2026-10-01".
    pub since: Option<String>,
    /// By `schema.table`, lowercased.
    tables: HashMap<String, Counts>,
    /// By table name alone, lowercased; `None` when two schemas have a table of that name.
    by_name: HashMap<String, Option<Counts>>,
    /// A table the statistics don't list wasn't used in the period (SQL Server).
    absent_means_unused: bool,
}

/// What a describe can say about usage.
#[derive(Debug, Clone, Default, PartialEq)]
pub enum Stats {
    /// The database keeps none (SQLite), or SayQL doesn't ask (team connections).
    #[default]
    None,
    /// It keeps them but they couldn't be read.
    Unavailable,
    Known(Usage),
}

impl Usage {
    fn insert(&mut self, schema: Option<&str>, name: &str, counts: Counts) {
        let name = name.to_lowercase();
        if let Some(s) = schema {
            self.tables
                .insert(format!("{}.{name}", s.to_lowercase()), counts.clone());
        }
        self.by_name
            .entry(name)
            .and_modify(|c| *c = None)
            .or_insert(Some(counts));
    }

    fn get(&self, t: &Table) -> Option<&Counts> {
        if let Some(c) = t.schema.as_ref().and_then(|s| {
            self.tables
                .get(&format!("{}.{}", s.to_lowercase(), t.name.to_lowercase()))
        }) {
            return Some(c);
        }
        self.by_name
            .get(&t.name.to_lowercase())
            .and_then(Option::as_ref)
    }
}

impl Stats {
    /// One line saying what the usage figures are and the period they cover.
    pub fn header(&self) -> Option<String> {
        match self {
            Stats::None => None,
            Stats::Unavailable => Some(UNAVAILABLE.to_string()),
            Stats::Known(u) => Some(match &u.since {
                Some(since) => format!(
                    "`usage:` is from the database's own statistics, {since} (counters reset on restart or failover and \
                     differ per replica): reads and writes are counts of scans and changed rows, never row values."
                ),
                None => "`usage:` is from the database's own statistics (reset when the server restarts): \
                         the last time each table was written, never row values."
                    .to_string(),
            }),
        }
    }

    /// The table's usage, e.g. `12.3k reads, 0 writes, last write 2026-03-12`, or nothing.
    pub fn line(&self, t: &Table) -> Option<String> {
        let Stats::Known(u) = self else { return None };
        if t.kind != "table" {
            return None;
        }
        match u.get(t) {
            Some(c) => format_counts(c, t.row_estimate),
            None if u.absent_means_unused => Some("no reads or writes".into()),
            None => None,
        }
    }
}

/// Compact counts: 950, 12.3k, 4.1M, 2.0B.
pub fn compact(n: i64) -> String {
    let a = n.unsigned_abs() as f64;
    let sign = if n < 0 { "-" } else { "" };
    let (v, unit) = if a >= 1e9 {
        (a / 1e9, "B")
    } else if a >= 1e6 {
        (a / 1e6, "M")
    } else if a >= 1e4 {
        (a / 1e3, "k")
    } else {
        return n.to_string();
    };
    format!(
        "{sign}{}{unit}",
        if v >= 100.0 {
            format!("{v:.0}")
        } else {
            format!("{v:.1}")
        }
    )
}

fn format_counts(c: &Counts, row_estimate: Option<i64>) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(r) = c.reads {
        parts.push(format!(
            "{} read{}",
            compact(r),
            if r == 1 { "" } else { "s" }
        ));
    }
    if let Some(w) = c.writes {
        parts.push(format!(
            "{} write{}",
            compact(w),
            if w == 1 { "" } else { "s" }
        ));
    }
    // Live rows only where the schema has no row estimate of its own.
    if let Some(n) = c.live_rows.filter(|_| row_estimate.is_none_or(|e| e < 0)) {
        parts.push(format!("~{} live rows", compact(n)));
    }
    if let Some(d) = &c.last_read {
        parts.push(format!("last read {d}"));
    }
    if let Some(d) = &c.last_write {
        parts.push(format!("last write {d}"));
    }
    if let Some(d) = &c.analyzed {
        parts.push(format!("analyzed {d}"));
    }
    (!parts.is_empty()).then(|| parts.join(", "))
}

fn num(v: Option<&Value>) -> Option<i64> {
    match v? {
        Value::Number(n) => n.as_i64().or_else(|| n.as_f64().map(|f| f as i64)),
        Value::String(s) => s.trim().parse::<f64>().ok().map(|f| f as i64),
        _ => None,
    }
}

/// The `YYYY-MM-DD` a timestamp starts with.
fn date(v: Option<&Value>) -> Option<String> {
    let s = v?.as_str()?.trim();
    let d = s.get(..10)?;
    chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d")
        .ok()
        .map(|_| d.to_string())
}

fn text(v: Option<&Value>) -> Option<String> {
    v?.as_str().map(str::to_string)
}

/// The statistics from the query's result, by column name.
pub fn parse(kind: Kind, r: &QueryResult) -> Stats {
    let col = |name: &str| r.columns.iter().position(|c| c.eq_ignore_ascii_case(name));
    let at = |row: &[Value], name: &str| col(name).and_then(|i| row.get(i).cloned());
    let mut u = Usage {
        absent_means_unused: kind == Kind::Mssql,
        ..Default::default()
    };
    let Some(first) = r.rows.first() else {
        return Stats::Unavailable;
    };
    u.since = match kind {
        Kind::Postgres => match (
            date(at(first, "stats_reset").as_ref()),
            date(at(first, "started").as_ref()),
        ) {
            (Some(reset), _) => Some(format!("since {reset}")),
            (None, Some(started)) => Some(format!("since the server started on {started}")),
            (None, None) => None,
        },
        Kind::Mssql => {
            date(at(first, "started").as_ref()).map(|d| format!("since the server started on {d}"))
        }
        Kind::Mysql => num(at(first, "uptime").as_ref()).map(|secs| {
            let started = chrono::Utc::now() - chrono::Duration::seconds(secs);
            format!("since the server started on {}", started.format("%Y-%m-%d"))
        }),
        Kind::Sqlite => return Stats::None,
    };
    for row in &r.rows {
        let Some(name) = text(at(row, "table_name").as_ref()) else {
            continue;
        };
        let schema = text(at(row, "schema_name").as_ref());
        let mut counts = Counts {
            reads: num(at(row, "reads").as_ref()),
            writes: num(at(row, "writes").as_ref()),
            live_rows: num(at(row, "live_rows").as_ref()),
            last_read: date(at(row, "last_read").as_ref()),
            last_write: date(at(row, "last_write").as_ref()),
            analyzed: date(at(row, "analyzed").as_ref()),
        };
        // Counts without the period they cover would mislead; keep only the dates then.
        if u.since.is_none() {
            (counts.reads, counts.writes, counts.live_rows) = (None, None, None);
        }
        u.insert(schema.as_deref(), &name, counts);
    }
    // SQL Server without the period: "not listed" can't be read as "unused".
    if u.since.is_none() {
        u.absent_means_unused = false;
    }
    Stats::Known(u)
}

/// The usage statistics of the pool's database, in one query.
pub async fn read(pool: &DbPool) -> Stats {
    let sql = match pool {
        DbPool::Pg(_) => PG_SQL,
        DbPool::Ms(_) => MSSQL_SQL,
        DbPool::My(_) => MYSQL_SQL,
        DbPool::Lite(_) | DbPool::Team(_) => return Stats::None,
    };
    match crate::readonly::run_limited(pool, sql, MAX_TABLES, TIMEOUT).await {
        Ok(r) => parse(pool.kind(), &r),
        Err(_) => Stats::Unavailable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn table(schema: Option<&str>, name: &str, kind: &str) -> Table {
        Table {
            schema: schema.map(Into::into),
            name: name.into(),
            kind: kind.into(),
            columns: vec![],
            foreign_keys: vec![],
            row_estimate: Some(10),
            ..Default::default()
        }
    }

    fn result(columns: &[&str], rows: Vec<Vec<Value>>) -> QueryResult {
        QueryResult {
            columns: columns.iter().map(|c| c.to_string()).collect(),
            rows,
            rows_affected: None,
            truncated: false,
            elapsed_ms: 1,
            withheld: vec![],
        }
    }

    #[test]
    fn compacts_numbers() {
        assert_eq!(compact(0), "0");
        assert_eq!(compact(9_999), "9999");
        assert_eq!(compact(12_345), "12.3k");
        assert_eq!(compact(4_100_000), "4.1M");
        assert_eq!(compact(250_000_000), "250M");
        assert_eq!(compact(2_000_000_000), "2.0B");
    }

    #[test]
    fn postgres_usage_states_the_period() {
        let cols = [
            "stats_reset",
            "started",
            "schema_name",
            "table_name",
            "reads",
            "writes",
            "live_rows",
            "analyzed",
        ];
        let r = result(
            &cols,
            vec![
                vec![
                    Value::Null,
                    json!("2026-10-01 08:00:00+00"),
                    json!("public"),
                    json!("orders"),
                    json!(12345),
                    json!(40),
                    json!(1200),
                    json!("2026-10-20 03:00:00+00"),
                ],
                vec![
                    Value::Null,
                    json!("2026-10-01 08:00:00+00"),
                    json!("public"),
                    json!("orders_legacy"),
                    json!("3"),
                    json!(0),
                    json!(0),
                    Value::Null,
                ],
            ],
        );
        let s = parse(Kind::Postgres, &r);
        assert!(
            s.header()
                .unwrap()
                .contains("since the server started on 2026-10-01"),
            "{:?}",
            s.header()
        );
        assert_eq!(
            s.line(&table(Some("public"), "orders", "table")).unwrap(),
            "12.3k reads, 40 writes, analyzed 2026-10-20"
        );
        assert_eq!(
            s.line(&table(Some("public"), "orders_legacy", "table"))
                .unwrap(),
            "3 reads, 0 writes"
        );
        assert_eq!(
            s.line(&table(Some("public"), "orders", "view")),
            None,
            "views have no usage"
        );
        assert_eq!(s.line(&table(Some("public"), "missing", "table")), None);
        let no_estimate = Table {
            row_estimate: None,
            ..table(Some("public"), "orders", "table")
        };
        assert!(s.line(&no_estimate).unwrap().contains("~1200 live rows"));

        let reset = result(
            &cols,
            vec![vec![
                json!("2026-01-01 00:00:00+00"),
                json!("2026-10-01 08:00:00+00"),
                Value::Null,
                Value::Null,
                Value::Null,
                Value::Null,
                Value::Null,
                Value::Null,
            ]],
        );
        assert!(
            parse(Kind::Postgres, &reset)
                .header()
                .unwrap()
                .contains("since 2026-01-01 (")
        );
    }

    #[test]
    fn sql_server_lists_unused_tables() {
        let cols = [
            "started",
            "schema_name",
            "table_name",
            "reads",
            "writes",
            "last_read",
            "last_write",
        ];
        let r = result(
            &cols,
            vec![vec![
                json!("2026-09-30T22:01:02.123"),
                json!("dbo"),
                json!("Orders"),
                json!(7),
                json!(2),
                json!("2026-10-27T10:00:00"),
                Value::Null,
            ]],
        );
        let s = parse(Kind::Mssql, &r);
        assert_eq!(
            s.line(&table(Some("dbo"), "orders", "table")).unwrap(),
            "7 reads, 2 writes, last read 2026-10-27"
        );
        assert_eq!(
            s.line(&table(Some("dbo"), "OrdersOld", "table")).unwrap(),
            "no reads or writes"
        );
        assert!(
            s.header()
                .unwrap()
                .contains("since the server started on 2026-09-30")
        );
    }

    #[test]
    fn mysql_without_performance_schema_keeps_only_dates() {
        let cols = [
            "uptime",
            "schema_name",
            "table_name",
            "reads",
            "writes",
            "last_write",
        ];
        let r = result(
            &cols,
            vec![vec![
                Value::Null,
                json!("shop"),
                json!("orders"),
                Value::Null,
                Value::Null,
                json!("2026-10-02 11:00:00"),
            ]],
        );
        let s = parse(Kind::Mysql, &r);
        assert_eq!(
            s.line(&table(Some("shop"), "orders", "table")).unwrap(),
            "last write 2026-10-02"
        );
        assert!(
            s.header()
                .unwrap()
                .contains("last time each table was written")
        );
        let r = result(
            &cols,
            vec![vec![
                json!("86400"),
                json!("shop"),
                json!("orders"),
                json!(5),
                json!(1),
                Value::Null,
            ]],
        );
        let s = parse(Kind::Mysql, &r);
        assert_eq!(
            s.line(&table(None, "orders", "table")).unwrap(),
            "5 reads, 1 write",
            "found by name alone"
        );
        assert!(s.header().unwrap().contains("since the server started on"));
    }

    #[test]
    fn nothing_for_sqlite_and_one_line_when_unavailable() {
        assert_eq!(Stats::None.header(), None);
        assert_eq!(Stats::Unavailable.header().as_deref(), Some(UNAVAILABLE));
        assert_eq!(Stats::Unavailable.line(&table(None, "t", "table")), None);
        assert_eq!(
            parse(Kind::Postgres, &result(&["started"], vec![])),
            Stats::Unavailable
        );
    }
}
