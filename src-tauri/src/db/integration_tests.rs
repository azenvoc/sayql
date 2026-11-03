//! Tests against real PostgreSQL, MySQL and SQL Server servers.
//!
//! Each test is skipped unless its URL is set, so `cargo test` passes without servers:
//!
//!   SAYQL_TEST_PG_URL=postgres://postgres:<password>@localhost:55416/sayql
//!   SAYQL_TEST_MYSQL_URL=mysql://root:<password>@localhost:33806
//!   SAYQL_TEST_MSSQL_URL=mssql://sa:<password>@localhost:14334/master
//!
//! (README.md shows how to start throwaway containers for these.)
//!
//! The Postgres tests drop and recreate the schemas `sayql_it`, `sayql_it_inv`, `sayql_usage_it`
//! and `sayql_kn`; the MySQL and SQL Server tests drop and recreate the databases `sayql_it` and
//! `sayql_kn`.
//! Nothing else is touched.

use super::*;
use crate::schema::Table;
use serde_json::json;

pub(super) struct Target {
    host: String,
    port: Option<u16>,
    user: String,
    password: Option<String>,
    pub(super) database: String,
}

/// Parses `scheme://user[:password]@host[:port][/database]`.
fn parse_url(url: &str) -> Target {
    let rest = url.split_once("://").map(|(_, r)| r).unwrap_or(url);
    let (auth, rest) = rest.rsplit_once('@').unwrap_or(("", rest));
    let (user, password) = match auth.split_once(':') {
        Some((u, p)) => (u.to_string(), Some(p.to_string())),
        None => (auth.to_string(), None),
    };
    let (hostport, database) = rest.split_once('/').unwrap_or((rest, ""));
    let (host, port) = match hostport.rsplit_once(':') {
        Some((h, p)) => (h.to_string(), Some(p.parse().expect("port"))),
        None => (hostport.to_string(), None),
    };
    let database = database.split('?').next().unwrap_or("").to_string();
    Target {
        host,
        port,
        user,
        password,
        database,
    }
}

pub(super) fn env_target(var: &str) -> Option<Target> {
    match std::env::var(var) {
        Ok(url) if !url.trim().is_empty() => Some(parse_url(url.trim())),
        _ => {
            eprintln!("skipping: set {var} to run this test against a real server");
            None
        }
    }
}

/// The connection settings and password from `var`, for tests outside this module.
pub(crate) fn env_config(var: &str, kind: Kind) -> Option<(ConnConfig, Option<String>)> {
    let t = env_target(var)?;
    let db = if kind == Kind::Postgres && t.database.is_empty() {
        "postgres".to_string()
    } else {
        t.database.clone()
    };
    Some((config(kind, &t, &db), t.password))
}

pub(super) async fn open(kind: Kind, t: &Target, database: &str) -> DbPool {
    connect(&config(kind, t, database), t.password.clone())
        .await
        .expect("connect")
}

fn config(kind: Kind, t: &Target, database: &str) -> ConnConfig {
    ConnConfig {
        id: "it".into(),
        name: "it".into(),
        kind,
        host: t.host.clone(),
        port: t.port,
        database: database.into(),
        user: t.user.clone(),
        file: String::new(),
        ssl: false,
        // The SQL Server test container uses its auto-generated self-signed certificate.
        trust_cert: kind == Kind::Mssql,
        production: false,
    }
}

/// Runs a T-SQL batch outside read-only mode.
async fn mssql_batch(pool: &MssqlPool, sql: &str) -> Result<(), String> {
    let mut conn = pool.acquire().await.map_err(|e| e.to_string())?;
    let stream = conn
        .client()
        .simple_query(sql)
        .await
        .map_err(|e| e.to_string())?;
    stream
        .into_results()
        .await
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// Runs a multi-statement setup script outside read-only mode.
pub(super) async fn exec(pool: &DbPool, sql: &str) {
    let r = match pool {
        DbPool::Pg(p) => sqlx::raw_sql(sqlx::AssertSqlSafe(sql))
            .execute(p)
            .await
            .map(|_| ())
            .map_err(|e| e.to_string()),
        DbPool::My(p) => sqlx::raw_sql(sqlx::AssertSqlSafe(sql))
            .execute(p)
            .await
            .map(|_| ())
            .map_err(|e| e.to_string()),
        DbPool::Ms(p) => mssql_batch(p, sql).await,
        DbPool::Lite(_) | DbPool::Team(_) => unreachable!("covered by the unit tests"),
    };
    r.unwrap_or_else(|e| panic!("setup failed: {e}\n{sql}"));
}

async fn query(pool: &DbPool, sql: &str, read_only: bool) -> QueryResult {
    run_query(pool, sql, read_only)
        .await
        .unwrap_or_else(|e| panic!("{sql}: {e}"))
}

async fn scalar(pool: &DbPool, sql: &str) -> Value {
    query(pool, sql, true).await.rows.remove(0).remove(0)
}

/// Runs `sql` with a deadline, so a query stuck behind a leftover lock fails instead of hanging.
async fn query_within(pool: &DbPool, sql: &str, read_only: bool) -> QueryResult {
    tokio::time::timeout(
        std::time::Duration::from_secs(10),
        run_query(pool, sql, read_only),
    )
    .await
    .unwrap_or_else(|_| panic!("{sql}: still waiting after 10 s"))
    .unwrap_or_else(|e| panic!("{sql}: {e}"))
}

/// Usage statistics (db/usage.rs) read from real servers: on Postgres a table's writes and the
/// period show up; elsewhere the statistics are read or said to be unavailable, never an error.
#[tokio::test]
async fn usage_statistics_from_real_servers() {
    if let Some(t) = env_target("SAYQL_TEST_PG_URL") {
        let db = if t.database.is_empty() {
            "postgres".to_string()
        } else {
            t.database.clone()
        };
        let pool = open(Kind::Postgres, &t, &db).await;
        exec(&pool, "DROP SCHEMA IF EXISTS sayql_usage_it CASCADE; CREATE SCHEMA sayql_usage_it; \
                     CREATE TABLE sayql_usage_it.busy (id int PRIMARY KEY); CREATE TABLE sayql_usage_it.idle (id int); \
                     INSERT INTO sayql_usage_it.busy SELECT generate_series(1, 25);")
            .await;
        let schema = introspect(&pool, Kind::Postgres).await.expect("introspect");
        // Postgres publishes a session's counters shortly after its transaction ends.
        let mut out = String::new();
        for _ in 0..40 {
            out = crate::safety::readonly::describe_with(
                &schema,
                Some("sayql_usage_it"),
                &usage::read(&pool).await,
            );
            if out.contains("25 writes") {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        }
        assert!(out.contains("own statistics, since"), "{out}");
        assert!(
            out.lines()
                .any(|l| l.starts_with("sayql_usage_it.busy") && l.contains("25 writes")),
            "{out}"
        );
        assert!(
            out.lines()
                .any(|l| l.starts_with("sayql_usage_it.idle") && l.contains("0 writes")),
            "{out}"
        );
        exec(&pool, "DROP SCHEMA sayql_usage_it CASCADE").await;
    }
    for (var, kind) in [
        ("SAYQL_TEST_MYSQL_URL", Kind::Mysql),
        ("SAYQL_TEST_MSSQL_URL", Kind::Mssql),
    ] {
        let Some(t) = env_target(var) else { continue };
        let pool = open(kind, &t, &t.database).await;
        let stats = usage::read(&pool).await;
        assert!(
            matches!(stats, usage::Stats::Known(_) | usage::Stats::Unavailable),
            "{kind:?}: {stats:?}"
        );
        if let usage::Stats::Known(u) = &stats {
            assert!(u.since.is_some() || kind == Kind::Mysql, "{kind:?}: {u:?}");
        }
    }
}

/// The database's own comments and CHECK constraints reach the schema and its text, and changing
/// a comment moves the fingerprint.
#[tokio::test]
async fn comments_and_checks_from_real_servers() {
    fn expect(tables: &[Table], schema: Option<&str>, dialect: &str) {
        let orders = table(tables, schema, "orders");
        assert_eq!(orders.comment.as_deref(), Some("One row per order"));
        assert_eq!(
            column(orders, "status").comment.as_deref(),
            Some("p = pending, s = shipped")
        );
        assert_eq!(column(orders, "total").comment, None);
        assert!(
            orders
                .checks
                .iter()
                .any(|c| c.starts_with("CHECK") && c.contains("total") && c.contains('0')),
            "{:?}",
            orders.checks
        );
        assert_eq!(orders.checks.len(), 2, "{:?}", orders.checks);
        let plain = table(tables, schema, "plain");
        assert!(plain.comment.is_none() && plain.checks.is_empty());
        assert!(plain.columns.iter().all(|c| c.comment.is_none()));
        let text = crate::schema::render_table(orders, dialect);
        assert!(text.contains("/* One row per order */]"), "{text}");
        assert!(text.contains("/* p = pending, s = shipped */"), "{text}");
    }

    if let Some(t) = env_target("SAYQL_TEST_PG_URL") {
        let db = if t.database.is_empty() {
            "postgres".to_string()
        } else {
            t.database.clone()
        };
        let pool = open(Kind::Postgres, &t, &db).await;
        exec(&pool, "DROP SCHEMA IF EXISTS sayql_kn CASCADE; CREATE SCHEMA sayql_kn;
            CREATE TABLE sayql_kn.orders (id int PRIMARY KEY, status text NOT NULL CHECK (status IN ('p', 's')),
              total numeric NOT NULL, CONSTRAINT total_ok CHECK (total >= 0));
            CREATE TABLE sayql_kn.plain (id int);
            COMMENT ON TABLE sayql_kn.orders IS 'One row per order';
            COMMENT ON COLUMN sayql_kn.orders.status IS 'p = pending, s = shipped';").await;
        let before = schema_hash::read(&pool).await.expect("fingerprint");
        let schema = introspect(&pool, Kind::Postgres).await.expect("introspect");
        expect(&schema.tables, Some("sayql_kn"), "PostgreSQL");
        exec(&pool, "COMMENT ON COLUMN sayql_kn.orders.total IS 'In USD'").await;
        assert_ne!(schema_hash::read(&pool).await.unwrap(), before);
        exec(&pool, "DROP SCHEMA sayql_kn CASCADE").await;
    }

    if let Some(t) = env_target("SAYQL_TEST_MYSQL_URL") {
        let admin = open(Kind::Mysql, &t, &t.database).await;
        exec(
            &admin,
            "DROP DATABASE IF EXISTS sayql_kn; CREATE DATABASE sayql_kn",
        )
        .await;
        let pool = open(Kind::Mysql, &t, "sayql_kn").await;
        // CHECK constraints are kept from MySQL 8.0.16 and MariaDB 10.2; older servers parse
        // and drop them.
        exec(&pool, "CREATE TABLE orders (id int PRIMARY KEY,
              status varchar(1) NOT NULL COMMENT 'p = pending, s = shipped' CHECK (status IN ('p', 's')),
              total decimal(10,2) NOT NULL, CONSTRAINT total_ok CHECK (total >= 0)) COMMENT = 'One row per order';
            CREATE TABLE plain (id int)").await;
        let schema = introspect(&pool, Kind::Mysql).await.expect("introspect");
        let orders = table(&schema.tables, None, "orders");
        if orders.checks.is_empty() {
            assert_eq!(orders.comment.as_deref(), Some("One row per order"));
            assert_eq!(
                column(orders, "status").comment.as_deref(),
                Some("p = pending, s = shipped")
            );
        } else {
            expect(&schema.tables, None, "MySQL");
        }
        let before = schema_hash::read(&pool).await.expect("fingerprint");
        exec(&pool, "ALTER TABLE plain COMMENT = 'Nothing much'").await;
        assert_ne!(schema_hash::read(&pool).await.unwrap(), before);
        exec(&admin, "DROP DATABASE sayql_kn").await;
    }

    if let Some(t) = env_target("SAYQL_TEST_MSSQL_URL") {
        let admin = open(Kind::Mssql, &t, &t.database).await;
        exec(&admin, "IF DB_ID('sayql_kn') IS NOT NULL BEGIN ALTER DATABASE sayql_kn SET SINGLE_USER WITH ROLLBACK IMMEDIATE; DROP DATABASE sayql_kn; END").await;
        exec(&admin, "CREATE DATABASE sayql_kn").await;
        let pool = open(Kind::Mssql, &t, "sayql_kn").await;
        for sql in [
            "CREATE TABLE dbo.orders (id int PRIMARY KEY, status varchar(1) NOT NULL CHECK (status IN ('p', 's')),
               total decimal(10,2) NOT NULL, CONSTRAINT total_ok CHECK (total >= 0))",
            "CREATE TABLE dbo.plain (id int)",
            "EXEC sp_addextendedproperty 'MS_Description', 'One row per order', 'SCHEMA', 'dbo', 'TABLE', 'orders'",
            "EXEC sp_addextendedproperty 'MS_Description', 'p = pending, s = shipped', 'SCHEMA', 'dbo', 'TABLE', 'orders', 'COLUMN', 'status'",
        ] {
            exec(&pool, sql).await;
        }
        let before = schema_hash::read(&pool).await.expect("fingerprint");
        let schema = introspect(&pool, Kind::Mssql).await.expect("introspect");
        expect(&schema.tables, Some("dbo"), "SQL Server");
        exec(&pool, "EXEC sp_addextendedproperty 'MS_Description', 'In USD', 'SCHEMA', 'dbo', 'TABLE', 'orders', 'COLUMN', 'total'").await;
        assert_ne!(schema_hash::read(&pool).await.unwrap(), before);
        drop(pool);
        exec(&admin, "ALTER DATABASE sayql_kn SET SINGLE_USER WITH ROLLBACK IMMEDIATE; DROP DATABASE sayql_kn").await;
    }
}

/// A confirmed statement commits, and the session state it leaves (SET, USE, locks) doesn't
/// carry over to the queries after it.
#[tokio::test]
async fn confirmed_statements_commit_and_leave_no_session_state() {
    if let Some(t) = env_target("SAYQL_TEST_PG_URL") {
        let db = if t.database.is_empty() {
            "postgres".to_string()
        } else {
            t.database.clone()
        };
        let pool = open(Kind::Postgres, &t, &db).await;
        exec(&pool, "DROP SCHEMA IF EXISTS sayql_cw CASCADE; CREATE SCHEMA sayql_cw; CREATE TABLE sayql_cw.t (id int PRIMARY KEY, n int);
            INSERT INTO sayql_cw.t VALUES (1, 0);").await;
        let w = query_within(&pool, "UPDATE sayql_cw.t SET n = n + 1 RETURNING n", false).await;
        assert_eq!(w.rows, [[json!(1)]]);
        query_within(&pool, "SET search_path TO sayql_cw", false).await;
        query_within(&pool, "SELECT pg_advisory_lock(4343)", false).await;
        for _ in 0..8 {
            // Committed, and seen from every pooled connection.
            assert_eq!(
                query_within(&pool, "SELECT n FROM sayql_cw.t", true)
                    .await
                    .rows,
                [[json!(1)]]
            );
            let path = query_within(&pool, "SHOW search_path", true).await.rows[0][0].clone();
            assert_ne!(path, json!("sayql_cw"), "a confirmed SET carried over");
        }
        let locks = "SELECT count(*) FROM pg_locks WHERE locktype = 'advisory' AND objid = 4343";
        assert_eq!(
            query_within(&pool, locks, true).await.rows,
            [[json!(0)]],
            "the session lock outlived its statement"
        );
        exec(&pool, "DROP SCHEMA sayql_cw CASCADE").await;
    }
    if let Some(t) = env_target("SAYQL_TEST_MYSQL_URL") {
        let admin = open(Kind::Mysql, &t, &t.database).await;
        exec(&admin, "DROP DATABASE IF EXISTS sayql_cw; CREATE DATABASE sayql_cw; CREATE TABLE sayql_cw.t (id int PRIMARY KEY, n int);
            INSERT INTO sayql_cw.t VALUES (1, 0);").await;
        let pool = open(Kind::Mysql, &t, "sayql_cw").await;
        let w = query_within(&pool, "UPDATE t SET n = n + 1", false).await;
        assert_eq!(w.rows_affected, Some(1));
        // MySQL 8 can't prepare these two (an error, nothing held); MariaDB runs them.
        let _ = run_query(&pool, "LOCK TABLES t WRITE", false).await;
        let _ = run_query(&pool, "USE mysql", false).await;
        query_within(&pool, "SET SESSION sql_mode = 'ANSI_QUOTES'", false).await;
        query_within(&pool, "SELECT GET_LOCK('sayql_cw', 0)", false).await;
        for _ in 0..8 {
            assert_eq!(
                query_within(&pool, "SELECT n FROM t", true).await.rows,
                [[json!(1)]]
            );
            assert_eq!(
                query_within(&pool, "SELECT DATABASE()", true).await.rows,
                [[json!("sayql_cw")]]
            );
            let mode = query_within(&pool, "SELECT @@SESSION.sql_mode", true)
                .await
                .rows[0][0]
                .clone();
            assert!(
                !mode.as_str().unwrap_or("").contains("ANSI_QUOTES"),
                "a confirmed SET carried over: {mode}"
            );
        }
        assert_eq!(
            query_within(&pool, "SELECT IS_FREE_LOCK('sayql_cw')", true)
                .await
                .rows,
            [[json!(1)]]
        );
        exec(&admin, "DROP DATABASE sayql_cw").await;
    }
    if let Some(t) = env_target("SAYQL_TEST_MSSQL_URL") {
        let admin = open(Kind::Mssql, &t, &t.database).await;
        exec(&admin, "IF DB_ID('sayql_cw') IS NOT NULL BEGIN ALTER DATABASE sayql_cw SET SINGLE_USER WITH ROLLBACK IMMEDIATE; DROP DATABASE sayql_cw; END").await;
        exec(&admin, "CREATE DATABASE sayql_cw").await;
        let pool = open(Kind::Mssql, &t, "sayql_cw").await;
        exec(&pool, "CREATE TABLE dbo.t (id int IDENTITY PRIMARY KEY, n int); INSERT INTO dbo.t (n) VALUES (0), (0);").await;
        // Rows a confirmed statement returns are shown, not dropped.
        let w = query_within(
            &pool,
            "INSERT INTO dbo.t (n) OUTPUT inserted.id, inserted.n VALUES (5)",
            false,
        )
        .await;
        assert_eq!(
            (w.columns.clone(), w.rows.clone()),
            (
                vec!["id".to_string(), "n".to_string()],
                vec![vec![json!(3), json!(5)]]
            )
        );
        let w = query_within(&pool, "DECLARE @x int = 41; SELECT @x + 1 AS answer", false).await;
        assert_eq!(w.rows, [[json!(42)]]);
        // Statements without rows report how many rows they changed, across the whole batch.
        let w = query_within(
            &pool,
            "UPDATE dbo.t SET n = n + 1 WHERE n = 0; DELETE FROM dbo.t WHERE n = 5",
            false,
        )
        .await;
        assert_eq!((w.columns.len(), w.rows_affected), (0, Some(3)));
        assert_eq!(
            query_within(&pool, "SELECT COUNT(*) AS c FROM dbo.t", true)
                .await
                .rows,
            [[json!(2)]]
        );
        query_within(
            &pool,
            "EXEC sp_set_session_context 'sayql', 'leftover'",
            false,
        )
        .await;
        let ctx = query_within(
            &pool,
            "SELECT CAST(SESSION_CONTEXT(N'sayql') AS nvarchar(20)) AS v",
            true,
        )
        .await;
        assert_eq!(ctx.rows, [[Value::Null]], "session state carried over");
        drop(pool);
        exec(&admin, "ALTER DATABASE sayql_cw SET SINGLE_USER WITH ROLLBACK IMMEDIATE; DROP DATABASE sayql_cw").await;
    }
}

fn table<'a>(tables: &'a [Table], schema: Option<&str>, name: &str) -> &'a Table {
    tables
        .iter()
        .find(|t| t.schema.as_deref() == schema && t.name == name)
        .unwrap_or_else(|| panic!("table {schema:?}.{name} missing"))
}

fn pk_columns(t: &Table) -> Vec<&str> {
    t.columns
        .iter()
        .filter(|c| c.primary_key)
        .map(|c| c.name.as_str())
        .collect()
}

fn fks(t: &Table) -> Vec<(String, String, String)> {
    let mut v: Vec<_> = t
        .foreign_keys
        .iter()
        .map(|f| (f.column.clone(), f.ref_table.clone(), f.ref_column.clone()))
        .collect();
    v.sort();
    v
}

fn fk(c: &str, t: &str, rc: &str) -> (String, String, String) {
    (c.into(), t.into(), rc.into())
}

fn column<'a>(t: &'a Table, name: &str) -> &'a crate::schema::Column {
    t.columns
        .iter()
        .find(|c| c.name == name)
        .unwrap_or_else(|| panic!("column {}.{name} missing", t.name))
}

fn is_placeholder(v: &Value) -> bool {
    matches!(v, Value::String(s) if s.starts_with('<') && s.ends_with('>') && !s.contains("</"))
}

/// Compares one result row against `expected` (column name -> value) and reports every mismatch at once.
fn check_row(label: &str, r: &QueryResult, row: usize, expected: &Value) {
    let expected = expected.as_object().unwrap();
    let mut bad = Vec::new();
    for (i, name) in r.columns.iter().enumerate() {
        let got = &r.rows[row][i];
        match expected.get(name) {
            Some(want) if want != got => bad.push(format!("  {name}: got {got}, want {want}")),
            None => bad.push(format!("  {name}: got {got}, no expectation")),
            _ => {}
        }
    }
    for name in expected.keys() {
        if !r.columns.contains(name) {
            bad.push(format!("  {name}: column missing from result"));
        }
    }
    assert!(
        bad.is_empty(),
        "{label} row {row} mismatches:\n{}",
        bad.join("\n")
    );
}

/// Read-only mode must be enforced by the database, survive multi-statement tricks,
/// and leave pooled connections usable for normal writes afterwards.
async fn check_read_only(
    pool: &DbPool,
    count_sql: &str,
    writes: &[&str],
    show_ro: &str,
    approved_write: &str,
) {
    let before = scalar(pool, count_sql).await;
    for _ in 0..3 {
        for sql in writes {
            let e = run_query(pool, sql, true).await;
            assert!(e.is_err(), "read-only mode allowed: {sql}");
        }
    }
    assert_eq!(
        scalar(pool, count_sql).await,
        before,
        "a read-only statement changed data"
    );
    // Run more queries than the pool has connections so every pooled connection is checked.
    for _ in 0..8 {
        let r = query(pool, show_ro, false).await;
        let v = r.rows[0].last().unwrap().clone();
        assert!(
            v == json!("off") || v == json!("OFF") || v == json!(0),
            "connection left read-only: {v}"
        );
    }
    let w = query(pool, approved_write, false).await;
    assert_eq!(w.rows_affected, Some(1), "{approved_write}");
    assert!(w.columns.is_empty());
}

/// Effects a read-only transaction allows and its rollback doesn't undo are refused in
/// read-only mode, and leave nothing behind.
#[tokio::test]
async fn read_only_refuses_effects_a_rollback_keeps() {
    if let Some(t) = env_target("SAYQL_TEST_PG_URL") {
        let db = if t.database.is_empty() {
            "postgres".to_string()
        } else {
            t.database.clone()
        };
        let pool = open(Kind::Postgres, &t, &db).await;
        for sql in [
            "SELECT pg_create_physical_replication_slot('sayql_it_slot')",
            "SELECT pg_advisory_lock(4242)",
            "SELECT pg_stat_reset()",
        ] {
            assert!(
                run_query(&pool, sql, true).await.is_err(),
                "read-only mode allowed: {sql}"
            );
        }
        let slots = "SELECT count(*) FROM pg_replication_slots WHERE slot_name = 'sayql_it_slot'";
        assert_eq!(scalar(&pool, slots).await, json!(0));
        let locks = "SELECT count(*) FROM pg_locks WHERE locktype = 'advisory' AND objid = 4242";
        assert_eq!(scalar(&pool, locks).await, json!(0));
    }
    if let Some(t) = env_target("SAYQL_TEST_MYSQL_URL") {
        let admin = open(Kind::Mysql, &t, &t.database).await;
        exec(&admin, "DROP DATABASE IF EXISTS sayql_ro; CREATE DATABASE sayql_ro; CREATE TABLE sayql_ro.t (id int PRIMARY KEY)").await;
        let pool = open(Kind::Mysql, &t, "sayql_ro").await;
        assert!(
            run_query(&pool, "SELECT GET_LOCK('sayql_it', 0)", true)
                .await
                .is_err()
        );
        assert_eq!(
            scalar(&pool, "SELECT IS_FREE_LOCK('sayql_it')").await,
            json!(1)
        );
        // Showing a table's CREATE statement is a read.
        let r = query(&pool, "SHOW CREATE TABLE t", true).await;
        assert!(
            r.rows[0][1]
                .as_str()
                .is_some_and(|s| s.starts_with("CREATE TABLE `t`")),
            "{:?}",
            r.rows
        );
        exec(&admin, "DROP DATABASE sayql_ro").await;
    }
}

// ---------------------------------------------------------------- PostgreSQL

const PG_SEED: &str = r#"
DROP SCHEMA IF EXISTS sayql_it CASCADE;
DROP SCHEMA IF EXISTS sayql_it_inv CASCADE;
CREATE SCHEMA sayql_it;
CREATE SCHEMA sayql_it_inv;

CREATE TYPE sayql_it.order_status AS ENUM ('pending', 'paid', 'shipped', 'cancelled');
-- Same type name in another schema with different labels.
CREATE TYPE sayql_it_inv.order_status AS ENUM ('in_stock', 'backordered');
CREATE TYPE sayql_it.addr AS (city text, zip int);

CREATE TABLE sayql_it.customers (
  id serial PRIMARY KEY,
  email text NOT NULL UNIQUE,
  external_id uuid NOT NULL,
  tags text[] NOT NULL DEFAULT '{}',
  profile jsonb,
  signed_up_at timestamptz NOT NULL
);

CREATE TABLE sayql_it.orders (
  id bigserial PRIMARY KEY,
  customer_id int NOT NULL REFERENCES sayql_it.customers(id),
  status sayql_it.order_status NOT NULL,
  total numeric(12, 2) NOT NULL,
  ordered_at timestamptz NOT NULL
);

CREATE TABLE sayql_it_inv.warehouses (code text PRIMARY KEY, name text NOT NULL);

CREATE TABLE sayql_it_inv.stock (
  warehouse_code text NOT NULL REFERENCES sayql_it_inv.warehouses(code),
  product_id int NOT NULL,
  qty int NOT NULL,
  state sayql_it_inv.order_status NOT NULL,
  PRIMARY KEY (warehouse_code, product_id)
);

CREATE TABLE sayql_it_inv.movements (
  id int PRIMARY KEY,
  wh text NOT NULL,
  pid int NOT NULL,
  delta int NOT NULL,
  FOREIGN KEY (wh, pid) REFERENCES sayql_it_inv.stock (warehouse_code, product_id)
);

CREATE TABLE sayql_it.shipments (
  id int PRIMARY KEY,
  order_id bigint NOT NULL REFERENCES sayql_it.orders(id),
  warehouse_code text NOT NULL REFERENCES sayql_it_inv.warehouses(code)
);

CREATE VIEW sayql_it.paid_orders AS SELECT * FROM sayql_it.orders WHERE status = 'paid';

INSERT INTO sayql_it.customers (email, external_id, tags, profile, signed_up_at)
SELECT 'c' || i || '@example.com', gen_random_uuid(), ARRAY['t' || (i % 3)], jsonb_build_object('n', i),
       timestamptz '2024-01-01 00:00:00+00' + i * interval '1 day'
FROM generate_series(1, 200) i;

INSERT INTO sayql_it.orders (customer_id, status, total, ordered_at)
SELECT 1 + i % 200, (ARRAY['pending', 'paid', 'shipped', 'cancelled'])[1 + i % 4]::sayql_it.order_status,
       (i % 500) + 0.99, timestamptz '2024-01-01 00:00:00+00' + i * interval '1 hour'
FROM generate_series(1, 1000) i;

CREATE TABLE sayql_it.all_types (
  id int PRIMARY KEY,
  b bool, i2 smallint, i4 int, i8 bigint, f4 real, f8 double precision,
  n numeric(12, 2), n_big numeric, n_neg numeric, n_nan numeric,
  t text, vc varchar(20), c char(3), nm name, u uuid, j json, jb jsonb,
  ts timestamp(3), tstz timestamptz, ts_inf timestamptz, d date, tm time, tmtz timetz,
  iv interval, m money, by bytea, st sayql_it.order_status,
  ta text[], ia int[], ia_null int[], ia2 int[], ba bool[], na numeric[], ua uuid[],
  tsa timestamptz[], ea sayql_it.order_status[], ja jsonb[],
  ip inet, ip6 inet, cn cidr, mac macaddr, bits bit(4), vb varbit, x xml, o oid, ch "char",
  pt point, r int4range, dr daterange, comp sayql_it.addr
);

INSERT INTO sayql_it.all_types VALUES (
  1, true, -2, 40000, 9000000000, 1.1, 2.5,
  1234.5, 123456789012345678901234567890.123456789, -0.00012, 'NaN',
  'hello', 'world', 'ab', 'pg_name', 'a0eebc99-9c0b-4ef8-bb6d-6bb9bd380a11', '{"a": 1}', '{"b": [1, 2]}',
  '2024-03-01 12:34:56.789', '2024-03-01 12:34:56+00', 'infinity', '2024-03-01', '12:34:56', '12:34:56+02',
  '1 year 2 mons 3 days 04:05:06.5', 12.34, '\xdeadbeef', 'paid',
  '{a,b}', '{1,2}', '{1,NULL}', '{{1,2},{3,4}}', '{t,f}', '{1.50,2}', '{a0eebc99-9c0b-4ef8-bb6d-6bb9bd380a11}',
  '{"2024-03-01 12:34:56+00"}', '{paid,shipped}', ARRAY['{"k": 1}'::jsonb],
  '192.168.0.1', '::1', '10.0.0.0/8', '08:00:2b:01:02:03', B'1010', B'101', '<a>1</a>', 42, 'a',
  '(1,2.5)', '[1,10)', '[2024-01-01,2024-02-01)', ROW('Paris', 75001)
);
INSERT INTO sayql_it.all_types (id) VALUES (2);
"#;

fn pg_expected() -> Value {
    serde_json::from_str(
        r#"{
        "id": 1, "b": true, "i2": -2, "i4": 40000, "i8": 9000000000, "f4": 1.1, "f8": 2.5,
        "n": "1234.50", "n_big": "123456789012345678901234567890.123456789", "n_neg": "-0.00012", "n_nan": "NaN",
        "t": "hello", "vc": "world", "c": "ab ", "nm": "pg_name", "u": "a0eebc99-9c0b-4ef8-bb6d-6bb9bd380a11",
        "j": {"a": 1}, "jb": {"b": [1, 2]},
        "ts": "2024-03-01 12:34:56.789", "tstz": "2024-03-01 12:34:56 UTC", "ts_inf": "infinity",
        "d": "2024-03-01", "tm": "12:34:56", "tmtz": "12:34:56+02",
        "iv": "1 year 2 mons 3 days 04:05:06.5", "m": "12.34", "by": "0xdeadbeef", "st": "paid",
        "ta": ["a", "b"], "ia": [1, 2], "ia_null": [1, null], "ia2": [[1, 2], [3, 4]], "ba": [true, false],
        "na": ["1.50", "2"], "ua": ["a0eebc99-9c0b-4ef8-bb6d-6bb9bd380a11"], "tsa": ["2024-03-01 12:34:56 UTC"],
        "ea": ["paid", "shipped"], "ja": [{"k": 1}],
        "ip": "192.168.0.1", "ip6": "::1", "cn": "10.0.0.0/8", "mac": "08:00:2b:01:02:03",
        "bits": "1010", "vb": "101", "x": "<a>1</a>", "o": 42, "ch": "a",
        "pt": "(1,2.5)", "r": "[1,10)", "dr": "[2024-01-01,2024-02-01)", "comp": {"city": "Paris", "zip": 75001}
    }"#,
    )
    .unwrap()
}

#[tokio::test]
async fn postgres_integration() {
    let Some(t) = env_target("SAYQL_TEST_PG_URL") else {
        return;
    };
    let db = if t.database.is_empty() {
        "postgres".to_string()
    } else {
        t.database.clone()
    };
    let pool = open(Kind::Postgres, &t, &db).await;
    let list = list_databases(&pool)
        .await
        .expect("list")
        .expect("a server has databases");
    assert!(
        list.databases.contains(&db) && !list.databases.iter().any(|d| d.starts_with("template")),
        "{list:?}"
    );
    assert_eq!(list.current.as_deref(), Some(db.as_str()));
    exec(&pool, PG_SEED).await;

    // ---- schema reading
    let schema = introspect(&pool, Kind::Postgres).await.expect("introspect");
    let ours: Vec<String> = schema
        .tables
        .iter()
        .filter(|t| {
            t.schema
                .as_deref()
                .is_some_and(|s| s.starts_with("sayql_it"))
        })
        .map(|t| t.qualified_name())
        .collect();
    assert_eq!(
        ours,
        [
            "sayql_it.all_types",
            "sayql_it.customers",
            "sayql_it.orders",
            "sayql_it.paid_orders",
            "sayql_it.shipments",
            "sayql_it_inv.movements",
            "sayql_it_inv.stock",
            "sayql_it_inv.warehouses",
        ]
    );
    let tb = &schema.tables;
    let customers = table(tb, Some("sayql_it"), "customers");
    let orders = table(tb, Some("sayql_it"), "orders");
    let stock = table(tb, Some("sayql_it_inv"), "stock");
    let view = table(tb, Some("sayql_it"), "paid_orders");
    assert_eq!(customers.kind, "table");
    assert_eq!(view.kind, "view");
    assert_eq!(pk_columns(customers), ["id"]);
    assert_eq!(pk_columns(stock), ["warehouse_code", "product_id"]);
    assert!(pk_columns(view).is_empty());
    assert_eq!(fks(orders), [fk("customer_id", "sayql_it.customers", "id")]);
    assert_eq!(
        fks(table(tb, Some("sayql_it"), "shipments")),
        [
            fk("order_id", "sayql_it.orders", "id"),
            fk("warehouse_code", "sayql_it_inv.warehouses", "code")
        ]
    );
    assert_eq!(
        fks(table(tb, Some("sayql_it_inv"), "movements")),
        [
            fk("pid", "sayql_it_inv.stock", "product_id"),
            fk("wh", "sayql_it_inv.stock", "warehouse_code")
        ]
    );
    let status = column(orders, "status");
    // Non-public types are schema-qualified, as they must be written in a cast.
    assert_eq!(status.data_type, "sayql_it.order_status");
    assert_eq!(
        status.enum_values.as_deref(),
        Some(&["pending", "paid", "shipped", "cancelled"].map(String::from)[..])
    );
    assert_eq!(
        column(stock, "state").enum_values.as_deref(),
        Some(&["in_stock", "backordered"].map(String::from)[..]),
        "enum labels must come from the column's own schema"
    );
    assert_eq!(column(customers, "tags").data_type, "_text");
    assert_eq!(column(orders, "total").data_type, "numeric");
    assert!(!column(orders, "total").nullable);
    assert!(column(customers, "profile").nullable);
    // Never-analyzed tables have no statistics yet: report "unknown", not a negative count.
    for t in tb.iter().filter(|t| {
        t.schema
            .as_deref()
            .is_some_and(|s| s.starts_with("sayql_it"))
    }) {
        assert!(
            t.row_estimate.is_none_or(|n| n >= 0),
            "{}: row estimate {:?}",
            t.name,
            t.row_estimate
        );
    }
    exec(
        &pool,
        "ANALYZE sayql_it.customers; ANALYZE sayql_it.orders;",
    )
    .await;
    let schema = introspect(&pool, Kind::Postgres).await.expect("introspect");
    assert_eq!(
        table(&schema.tables, Some("sayql_it"), "customers").row_estimate,
        Some(200)
    );
    assert_eq!(
        table(&schema.tables, Some("sayql_it"), "orders").row_estimate,
        Some(1000)
    );
    assert_eq!(
        table(&schema.tables, Some("sayql_it"), "paid_orders").row_estimate,
        None
    );

    // ---- value decoding
    let r = query(&pool, "SELECT * FROM sayql_it.all_types ORDER BY id", true).await;
    check_row("pg read-only", &r, 0, &pg_expected());
    assert!(r.rows[1][1..].iter().all(Value::is_null), "{:?}", r.rows[1]);
    assert!(!r.rows[0].iter().any(is_placeholder));
    // Approved writes that return rows decode the same way.
    let w = query(
        &pool,
        "UPDATE sayql_it.all_types SET i4 = i4 WHERE id = 1 RETURNING *",
        false,
    )
    .await;
    check_row("pg RETURNING", &w, 0, &pg_expected());
    let r = query(
        &pool,
        "SELECT status, count(*) AS n, sum(total) AS s FROM sayql_it.orders GROUP BY 1 ORDER BY 1",
        true,
    )
    .await;
    assert_eq!(r.columns, ["status", "n", "s"]);
    assert_eq!(
        r.rows
            .iter()
            .map(|row| row[1].as_i64().unwrap())
            .sum::<i64>(),
        1000
    );
    assert_eq!(r.rows[0][0], json!("pending"));

    // ---- other read-only statement kinds still work
    assert_eq!(
        scalar(&pool, "SHOW transaction_read_only").await,
        json!("on")
    );
    assert_eq!(scalar(&pool, "VALUES (1)").await, json!(1));
    assert_eq!(
        query(&pool, "TABLE sayql_it_inv.warehouses", true)
            .await
            .columns,
        ["code", "name"]
    );
    assert!(
        !query(&pool, "EXPLAIN SELECT * FROM sayql_it.orders", true)
            .await
            .rows
            .is_empty()
    );
    assert_eq!(
        scalar(&pool, "WITH x AS (SELECT 41 AS v) SELECT v + 1 FROM x").await,
        json!(42)
    );

    // ---- empty results keep their headers
    for ro in [true, false] {
        let r = query(
            &pool,
            "SELECT id, email FROM sayql_it.customers WHERE false",
            ro,
        )
        .await;
        assert_eq!(r.columns, ["id", "email"]);
        assert!(r.rows.is_empty());
        assert_eq!(r.rows_affected, None);
    }

    // ---- read-only enforcement
    check_read_only(
        &pool,
        "SELECT count(*) FROM sayql_it.orders WHERE status = 'paid'",
        &[
            "UPDATE sayql_it.orders SET status = 'paid'",
            "DELETE FROM sayql_it.orders",
            "WITH d AS (DELETE FROM sayql_it.orders RETURNING id) SELECT count(*) FROM d",
            "SELECT 1; COMMIT; UPDATE sayql_it.orders SET status = 'paid'",
            "SELECT 1; ROLLBACK; BEGIN; UPDATE sayql_it.orders SET status = 'paid'; COMMIT",
            "SELECT nextval('sayql_it.orders_id_seq')",
        ],
        "SHOW transaction_read_only",
        "UPDATE sayql_it.orders SET status = 'paid' WHERE id = 1",
    )
    .await;

    exec(
        &pool,
        "DROP SCHEMA sayql_it CASCADE; DROP SCHEMA sayql_it_inv CASCADE;",
    )
    .await;
}

const PG_PARTS_SEED: &str = r#"
DROP SCHEMA IF EXISTS sayql_pt CASCADE;
DROP ROLE IF EXISTS sayql_pt_reader;
CREATE SCHEMA sayql_pt;
CREATE TABLE sayql_pt.events (id int, at date NOT NULL, note text, PRIMARY KEY (id, at)) PARTITION BY RANGE (at);
CREATE TABLE sayql_pt.events_2024 PARTITION OF sayql_pt.events FOR VALUES FROM ('2024-01-01') TO ('2025-01-01');
CREATE TABLE sayql_pt.events_2025 PARTITION OF sayql_pt.events FOR VALUES FROM ('2025-01-01') TO ('2026-01-01');
INSERT INTO sayql_pt.events SELECT i, date '2024-01-01' + (i % 700), 'n' FROM generate_series(1, 3000) i;
CREATE TABLE sayql_pt.event_tags (event_id int, event_at date, tag text,
  PRIMARY KEY (event_id, event_at, tag), FOREIGN KEY (event_id, event_at) REFERENCES sayql_pt.events (id, at));
CREATE MATERIALIZED VIEW sayql_pt.daily AS SELECT at, count(*) AS n FROM sayql_pt.events GROUP BY at;
ANALYZE sayql_pt.events, sayql_pt.events_2024, sayql_pt.events_2025, sayql_pt.daily;
"#;

/// Partitioned tables and materialized views, read by the owner and by a role that can
/// only SELECT (the setup the docs recommend).
#[tokio::test]
async fn postgres_partitions_matviews_and_read_only_roles() {
    let Some(t) = env_target("SAYQL_TEST_PG_URL") else {
        return;
    };
    let db = if t.database.is_empty() {
        "postgres".to_string()
    } else {
        t.database.clone()
    };
    let pool = open(Kind::Postgres, &t, &db).await;
    exec(&pool, PG_PARTS_SEED).await;
    let reader_pw = uuid::Uuid::new_v4().simple().to_string();
    exec(
        &pool,
        &format!(
            "CREATE ROLE sayql_pt_reader LOGIN PASSWORD '{reader_pw}'; GRANT USAGE ON SCHEMA sayql_pt TO sayql_pt_reader; \
             GRANT SELECT ON ALL TABLES IN SCHEMA sayql_pt TO sayql_pt_reader;"
        ),
    )
    .await;
    let reader_target = Target {
        user: "sayql_pt_reader".into(),
        password: Some(reader_pw),
        ..parse_url(&std::env::var("SAYQL_TEST_PG_URL").unwrap())
    };
    let reader = open(Kind::Postgres, &reader_target, &db).await;

    for (who, pool) in [("owner", &pool), ("reader", &reader)] {
        let schema = introspect(pool, Kind::Postgres).await.expect("introspect");
        let ours: Vec<String> = schema
            .tables
            .iter()
            .filter(|t| t.schema.as_deref() == Some("sayql_pt"))
            .map(|t| t.qualified_name())
            .collect();
        // Partitions are left out: queries go through the parent.
        assert_eq!(
            ours,
            ["sayql_pt.daily", "sayql_pt.event_tags", "sayql_pt.events"],
            "{who}"
        );
        let tb = &schema.tables;
        let events = table(tb, Some("sayql_pt"), "events");
        assert_eq!(events.kind, "table");
        // A role with only SELECT still sees the keys.
        assert_eq!(pk_columns(events), ["id", "at"], "{who}");
        assert_eq!(
            pk_columns(table(tb, Some("sayql_pt"), "event_tags")),
            ["event_id", "event_at", "tag"],
            "{who}"
        );
        // One foreign key to the parent, not one per partition.
        assert_eq!(
            fks(table(tb, Some("sayql_pt"), "event_tags")),
            [
                fk("event_at", "sayql_pt.events", "at"),
                fk("event_id", "sayql_pt.events", "id")
            ],
            "{who}"
        );
        // The parent's estimate covers all its partitions (on PG 13 its own estimate is 0).
        assert_eq!(events.row_estimate, Some(3000), "{who}");
        let daily = table(tb, Some("sayql_pt"), "daily");
        assert_eq!(daily.kind, "materialized view");
        assert_eq!(
            daily
                .columns
                .iter()
                .map(|c| c.name.as_str())
                .collect::<Vec<_>>(),
            ["at", "n"]
        );
        assert_eq!(column(daily, "at").data_type, "date");
        assert_eq!(column(daily, "n").data_type, "bigint");
        assert_eq!(daily.row_estimate, Some(700), "{who}");
        let text = crate::schema::render_full(&schema);
        assert!(
            text.contains("sayql_pt.daily [materialized view, ~700 rows](at date, n bigint)"),
            "{text}"
        );
    }
    drop(reader);
    exec(
        &pool,
        "DROP SCHEMA sayql_pt CASCADE; DROP ROLE sayql_pt_reader;",
    )
    .await;
}

// ---------------------------------------------------------------- MySQL

const MY_SEED: &str = r#"
DROP DATABASE IF EXISTS sayql_it;
CREATE DATABASE sayql_it;
USE sayql_it;

CREATE TABLE customers (
  id INT UNSIGNED AUTO_INCREMENT PRIMARY KEY,
  email VARCHAR(200) NOT NULL UNIQUE,
  tier ENUM('free', 'pro', 'enterprise') NOT NULL DEFAULT 'free',
  signed_up_at DATETIME NOT NULL
);

CREATE TABLE orders (
  id BIGINT UNSIGNED AUTO_INCREMENT PRIMARY KEY,
  customer_id INT UNSIGNED NOT NULL,
  status ENUM('pending', 'paid', 'shipped', 'it''s late') NOT NULL,
  total DECIMAL(12, 2) NOT NULL,
  meta JSON NULL,
  ordered_at TIMESTAMP NOT NULL,
  FOREIGN KEY (customer_id) REFERENCES customers (id)
);

CREATE TABLE order_items (
  order_id BIGINT UNSIGNED NOT NULL,
  line SMALLINT UNSIGNED NOT NULL,
  qty INT NOT NULL,
  PRIMARY KEY (order_id, line),
  FOREIGN KEY (order_id) REFERENCES orders (id)
);

CREATE VIEW paid_orders AS SELECT * FROM orders WHERE status = 'paid';

-- 1..1000 without a recursive CTE, which MySQL 5.7 lacks. Temporary, so not part of the schema.
CREATE TEMPORARY TABLE n (i INT PRIMARY KEY);
INSERT INTO n
SELECT a.d * 100 + b.d * 10 + c.d + 1
FROM (SELECT 0 d UNION ALL SELECT 1 UNION ALL SELECT 2 UNION ALL SELECT 3 UNION ALL SELECT 4
      UNION ALL SELECT 5 UNION ALL SELECT 6 UNION ALL SELECT 7 UNION ALL SELECT 8 UNION ALL SELECT 9) a,
     (SELECT 0 d UNION ALL SELECT 1 UNION ALL SELECT 2 UNION ALL SELECT 3 UNION ALL SELECT 4
      UNION ALL SELECT 5 UNION ALL SELECT 6 UNION ALL SELECT 7 UNION ALL SELECT 8 UNION ALL SELECT 9) b,
     (SELECT 0 d UNION ALL SELECT 1 UNION ALL SELECT 2 UNION ALL SELECT 3 UNION ALL SELECT 4
      UNION ALL SELECT 5 UNION ALL SELECT 6 UNION ALL SELECT 7 UNION ALL SELECT 8 UNION ALL SELECT 9) c;

INSERT INTO customers (email, tier, signed_up_at)
SELECT CONCAT('c', i, '@example.com'), ELT(1 + i % 3, 'free', 'pro', 'enterprise'), '2024-01-01' + INTERVAL i DAY
FROM n WHERE i <= 200 ORDER BY i;

INSERT INTO orders (customer_id, status, total, meta, ordered_at)
SELECT 1 + i % 200, ELT(1 + i % 4, 'pending', 'paid', 'shipped', 'it''s late'), (i % 500) + 0.99,
       JSON_OBJECT('n', i), '2024-01-01' + INTERVAL i HOUR FROM n ORDER BY i;
DROP TEMPORARY TABLE n;

CREATE TABLE all_types (
  id INT PRIMARY KEY,
  b BOOL, ti TINYINT, tiu TINYINT UNSIGNED, si SMALLINT, siu SMALLINT UNSIGNED, mi MEDIUMINT,
  i INT, iu INT UNSIGNED, bi BIGINT, biu BIGINT UNSIGNED, f FLOAT, d DOUBLE,
  dc DECIMAL(12, 2), dc_big DECIMAL(65, 30),
  dt DATETIME, dt6 DATETIME(6), ts TIMESTAMP NULL, da DATE, tm TIME, tm_neg TIME, tm_big TIME, yr YEAR,
  ch CHAR(3), vc VARCHAR(20), tx TEXT, js JSON, en ENUM('a', 'b'), st SET('x', 'y', 'z'),
  bin BINARY(4), vb VARBINARY(8), bl BLOB, bt BIT(4)
);

INSERT INTO all_types VALUES (
  1, TRUE, -5, 250, -300, 60000, -8000000,
  -2000000000, 4000000000, -9000000000, 18446744073709551615, 1.1, 2.5,
  1234.5, 12345678901234567890.123456789,
  '2024-03-01 12:34:56', '2024-03-01 12:34:56.123456', '2024-03-01 12:34:56', '2024-03-01', '12:34:56', '-01:30:00', '100:00:00', 2024,
  'ab', 'world', 'hello', '{"a": 1}', 'b', 'x,z',
  'ab', x'DEADBEEF', 'blob text', b'1010'
);
INSERT INTO all_types (id) VALUES (2);
"#;

/// Values the drivers don't decode on their own: MySQL zero dates (which sqlx reads as NULL)
/// and geometry, and SQL Server columns without a name.
#[tokio::test]
async fn zero_dates_geometry_and_unnamed_columns() {
    if let Some(t) = env_target("SAYQL_TEST_MYSQL_URL") {
        let admin = open(Kind::Mysql, &t, &t.database).await;
        exec(&admin, "DROP DATABASE IF EXISTS sayql_val; CREATE DATABASE sayql_val;
            CREATE TABLE sayql_val.zd (id int PRIMARY KEY, d date, dt datetime(6), ts timestamp NULL);
            CREATE TABLE sayql_val.geo (id int PRIMARY KEY, p point, g geometry, g4326 geometry);").await;
        // Zero dates need a non-strict sql_mode, on the same connection as the insert.
        exec(
            &admin,
            "SET SESSION sql_mode = ''; INSERT INTO sayql_val.zd VALUES
            (1, '0000-00-00', '0000-00-00 00:00:00', '0000-00-00 00:00:00'),
            (2, '2024-00-15', '2024-02-00 10:00:05.5', NULL), (3, NULL, NULL, NULL);",
        )
        .await;
        exec(&admin, "INSERT INTO sayql_val.geo VALUES (1, POINT(1.5, -2), ST_GeomFromText('POLYGON((0 0,4 0,4 4,0 0))'),
            ST_GeomFromText('MULTIPOINT(1 2,3 4)'))").await;
        let pool = open(Kind::Mysql, &t, "sayql_val").await;
        let r = query(&pool, "SELECT d, dt, ts FROM zd ORDER BY id", true).await;
        assert_eq!(
            r.rows[0],
            [
                json!("0000-00-00"),
                json!("0000-00-00 00:00:00"),
                json!("0000-00-00 00:00:00")
            ]
        );
        assert_eq!(
            r.rows[1],
            [
                json!("2024-00-15"),
                json!("2024-02-00 10:00:05.500000"),
                Value::Null
            ]
        );
        assert_eq!(r.rows[2], [Value::Null, Value::Null, Value::Null]);
        let r = query(&pool, "SELECT p, g, g4326 FROM geo", true).await;
        assert_eq!(
            r.rows[0],
            [
                json!("POINT(1.5 -2)"),
                json!("POLYGON((0 0,4 0,4 4,0 0))"),
                json!("MULTIPOINT((1 2),(3 4))")
            ]
        );
        exec(&admin, "DROP DATABASE sayql_val").await;
    }
    if let Some(t) = env_target("SAYQL_TEST_MSSQL_URL") {
        let pool = open(Kind::Mssql, &t, &t.database).await;
        let r = query(
            &pool,
            "SELECT 1, 2 AS two, COUNT(*) FROM sys.databases WHERE 1 = 0",
            true,
        )
        .await;
        assert_eq!(r.columns, ["(No column name)", "two", "(No column name)"]);
        // money is exact to its 4 places, at the ends of its range too (tiberius is patched for it).
        let r = query(
            &pool,
            "SELECT CAST(922337203685477.5807 AS money), CAST(-922337203685477.5808 AS money), \
            CAST(-214748.3648 AS smallmoney), CAST(0.01 AS money), CAST(NULL AS money)",
            true,
        )
        .await;
        assert_eq!(
            r.rows[0],
            [
                json!("922337203685477.5807"),
                json!("-922337203685477.5808"),
                json!("-214748.3648"),
                json!("0.0100"),
                Value::Null
            ]
        );
        // geometry reads as SQL Server's own STAsText() writes it.
        for wkt in [
            "POINT (1.5 -2)",
            "POINT (1 2 3)",
            "LINESTRING (0 0, 1 1)",
            "LINESTRING (0 0, 1 1, 2 0)",
            "POLYGON ((0 0, 4 0, 4 4, 0 0), (1 1, 2 1, 2 2, 1 1))",
            "MULTIPOINT ((1 2), (3 4))",
            "MULTILINESTRING ((0 0, 1 1), (2 2, 3 3))",
            "MULTIPOLYGON (((0 0, 1 0, 1 1, 0 0)), ((5 5, 6 5, 6 6, 5 5)))",
            "GEOMETRYCOLLECTION (POINT (1 2), LINESTRING (0 0, 1 1))",
            "POINT EMPTY",
        ] {
            let sql = format!(
                "SELECT g, g.STAsText() AS t FROM (SELECT geometry::STGeomFromText('{wkt}', 0) AS g) x"
            );
            let r = query(&pool, &sql, true).await;
            assert_eq!(r.rows[0][0], r.rows[0][1], "{wkt}");
        }
        // geography, SRID-4326 geometry and hierarchyid read as SQL Server writes them too, with
        // NULLs kept, in read-only and confirmed statements alike.
        let sql = "SELECT g, h, p FROM (VALUES (geography::Point(47.6, -122.3, 4326), hierarchyid::Parse('/1/2/'), \
                   geometry::STGeomFromText('LINESTRING (0 0, 3 4)', 4326)), (NULL, NULL, NULL)) x(g, h, p)";
        let want = [
            vec![
                json!("POINT (-122.3 47.6)"),
                json!("/1/2/"),
                json!("LINESTRING (0 0, 3 4)"),
            ],
            vec![Value::Null, Value::Null, Value::Null],
        ];
        assert_eq!(query(&pool, sql, true).await.rows, want);
        assert_eq!(
            query(&pool, &format!("DECLARE @x int = 1; {sql}"), false)
                .await
                .rows,
            want
        );
    }
}

fn my_expected() -> Value {
    serde_json::from_str(
        r#"{
        "id": 1, "b": true, "ti": -5, "tiu": 250, "si": -300, "siu": 60000, "mi": -8000000,
        "i": -2000000000, "iu": 4000000000, "bi": -9000000000, "biu": 18446744073709551615,
        "f": 1.1, "d": 2.5, "dc": "1234.50", "dc_big": "12345678901234567890.123456789000000000000000000000",
        "dt": "2024-03-01 12:34:56", "dt6": "2024-03-01 12:34:56.123456", "ts": "2024-03-01 12:34:56 UTC",
        "da": "2024-03-01", "tm": "12:34:56", "tm_neg": "-01:30:00", "tm_big": "100:00:00", "yr": 2024,
        "ch": "ab", "vc": "world", "tx": "hello", "js": {"a": 1}, "en": "b", "st": "x,z",
        "bin": "0x61620000", "vb": "0xdeadbeef", "bl": "blob text", "bt": 10
    }"#,
    )
    .unwrap()
}

#[tokio::test]
async fn mysql_integration() {
    let Some(t) = env_target("SAYQL_TEST_MYSQL_URL") else {
        return;
    };
    let admin = open(Kind::Mysql, &t, &t.database).await;
    exec(&admin, MY_SEED).await;
    let pool = open(Kind::Mysql, &t, "sayql_it").await;
    let list = list_databases(&pool)
        .await
        .expect("list")
        .expect("a server has databases");
    assert!(
        list.databases.contains(&"sayql_it".to_string())
            && !list.databases.contains(&"mysql".to_string()),
        "{list:?}"
    );
    assert_eq!(list.current.as_deref(), Some("sayql_it"));

    // ---- schema reading
    let schema = introspect(&pool, Kind::Mysql).await.expect("introspect");
    // Sorted here: MySQL and MariaDB order `_` differently.
    let mut names: Vec<&str> = schema.tables.iter().map(|t| t.name.as_str()).collect();
    names.sort();
    assert_eq!(
        names,
        [
            "all_types",
            "customers",
            "order_items",
            "orders",
            "paid_orders"
        ]
    );
    let tb = &schema.tables;
    let customers = table(tb, None, "customers");
    let orders = table(tb, None, "orders");
    let view = table(tb, None, "paid_orders");
    assert_eq!(view.kind, "view");
    assert_eq!(customers.kind, "table");
    assert_eq!(pk_columns(customers), ["id"]);
    assert_eq!(
        pk_columns(table(tb, None, "order_items")),
        ["order_id", "line"]
    );
    assert!(pk_columns(view).is_empty());
    assert_eq!(fks(orders), [fk("customer_id", "customers", "id")]);
    assert_eq!(
        fks(table(tb, None, "order_items")),
        [fk("order_id", "orders", "id")]
    );
    let status = column(orders, "status");
    assert_eq!(status.data_type, "enum");
    assert_eq!(
        status.enum_values.as_deref(),
        Some(&["pending", "paid", "shipped", "it's late"].map(String::from)[..])
    );
    assert_eq!(
        column(customers, "tier").enum_values.as_deref(),
        Some(&["free", "pro", "enterprise"].map(String::from)[..])
    );
    assert_eq!(column(orders, "total").data_type, "decimal(12,2)");
    // MySQL 5.7 and MariaDB still write display widths: `int(10) unsigned`.
    assert_eq!(
        column(customers, "id").data_type.replace("(10)", ""),
        "int unsigned"
    );
    assert!(column(orders, "meta").nullable);
    assert!(!column(orders, "total").nullable);
    exec(&pool, "ANALYZE TABLE customers, orders").await;
    let schema = introspect(&pool, Kind::Mysql).await.expect("introspect");
    // InnoDB row counts are estimates; they should be in the right ballpark.
    let est = |name: &str| table(&schema.tables, None, name).row_estimate;
    assert!(
        est("customers").is_some_and(|n| (150..=250).contains(&n)),
        "customers ~200: {:?}",
        est("customers")
    );
    assert!(
        est("orders").is_some_and(|n| (800..=1200).contains(&n)),
        "orders ~1000: {:?}",
        est("orders")
    );
    assert_eq!(est("paid_orders"), None);

    // ---- value decoding, inside and outside the read-only transaction
    let version = scalar(&pool, "SELECT VERSION()")
        .await
        .as_str()
        .unwrap_or_default()
        .to_string();
    // MariaDB stores JSON as LONGTEXT; sqlx-mysql is patched to read its JSON marker.
    let expected = my_expected();
    let r = query(&pool, "SELECT * FROM all_types ORDER BY id", true).await;
    check_row("mysql read-only", &r, 0, &expected);
    assert!(r.rows[1][1..].iter().all(Value::is_null), "{:?}", r.rows[1]);
    assert!(!r.rows[0].iter().any(is_placeholder));
    let w = query(&pool, "SELECT * FROM all_types ORDER BY id", false).await;
    check_row("mysql read-write", &w, 0, &expected);
    let r = query(
        &pool,
        "SELECT status, COUNT(*) AS n, SUM(total) AS s FROM orders GROUP BY status ORDER BY status",
        true,
    )
    .await;
    assert_eq!(r.columns, ["status", "n", "s"]);
    assert_eq!(
        r.rows
            .iter()
            .map(|row| row[1].as_i64().unwrap())
            .sum::<i64>(),
        1000
    );
    assert_eq!(r.rows[0][0], json!("pending"));
    assert_eq!(r.rows[0][2], json!("62247.50"));

    // ---- other read-only statement kinds still work
    assert!(!query(&pool, "SHOW TABLES", true).await.rows.is_empty());
    assert!(!query(&pool, "DESCRIBE orders", true).await.rows.is_empty());
    assert!(
        !query(&pool, "EXPLAIN SELECT * FROM orders", true)
            .await
            .rows
            .is_empty()
    );
    // MySQL 5.7 has no WITH.
    if !version.starts_with("5.7") {
        assert_eq!(
            scalar(&pool, "WITH x AS (SELECT 41 AS v) SELECT v + 1 FROM x").await,
            json!(42)
        );
    }

    // ---- empty results keep their headers
    for ro in [true, false] {
        let r = query(&pool, "SELECT id, email FROM customers WHERE 1 = 0", ro).await;
        assert_eq!(r.columns, ["id", "email"]);
        assert!(r.rows.is_empty());
        assert_eq!(r.rows_affected, None);
    }

    // ---- read-only enforcement
    // MariaDB before 11.1 calls it tx_read_only.
    let show_ro = if run_query(&pool, "SELECT @@transaction_read_only", true)
        .await
        .is_ok()
    {
        "SELECT @@transaction_read_only"
    } else {
        "SELECT @@tx_read_only"
    };
    check_read_only(
        &pool,
        "SELECT COUNT(*) FROM orders WHERE status = 'paid'",
        &[
            "UPDATE orders SET status = 'paid'",
            "DELETE FROM order_items",
            "SELECT 1; COMMIT; UPDATE orders SET status = 'paid'",
            "SELECT 1; ROLLBACK; UPDATE orders SET status = 'paid'",
        ],
        show_ro,
        "UPDATE orders SET status = 'paid' WHERE id = 1",
    )
    .await;

    exec(&admin, "DROP DATABASE sayql_it").await;
}

/// Connection failures say what went wrong, quickly, instead of "pool timed out".
async fn check_connection_errors(kind: Kind, t: &Target) {
    let db = t.database.clone();
    let base = config(kind, t, &db);
    let started = std::time::Instant::now();
    let closed = ConnConfig {
        host: "127.0.0.1".into(),
        port: Some(1),
        ..base.clone()
    };
    let err = connect(&closed, t.password.clone())
        .await
        .err()
        .expect("closed port")
        .to_string();
    assert!(err.starts_with("Could not reach 127.0.0.1:1:"), "{err}");
    assert!(
        started.elapsed() < std::time::Duration::from_secs(5),
        "took {:?}",
        started.elapsed()
    );

    let err = connect(
        &ConnConfig {
            host: "no-such-host.invalid".into(),
            ..base.clone()
        },
        t.password.clone(),
    )
    .await
    .err()
    .expect("unknown host")
    .to_string();
    assert!(
        err.starts_with("Could not reach no-such-host.invalid:"),
        "{err}"
    );

    let err = connect(&base, Some("wrong-password".into()))
        .await
        .err()
        .expect("wrong password")
        .to_string();
    assert!(
        err.contains("password authentication failed")
            || err.contains("Access denied")
            || err.starts_with("Wrong user or password"),
        "{err}"
    );

    let plain = connect(&base, t.password.clone())
        .await
        .expect("connect with SSL off");
    // A server that offers TLS (MySQL 5.7 included) connects with SSL required, encrypted.
    // One without TLS fails with a message saying how to connect without it.
    let offers_tls = match kind {
        Kind::Postgres => scalar(&plain, "SHOW ssl").await == json!("on"),
        // MySQL 8.4 dropped `have_ssl`; it always has TLS unless built without it.
        _ => query(&plain, "SHOW VARIABLES LIKE 'have_ssl'", true)
            .await
            .rows
            .first()
            .is_none_or(|r| r[1] == json!("YES")),
    };
    match connect(&ConnConfig { ssl: true, ..base }, t.password.clone()).await {
        Ok(tls) => {
            assert!(
                offers_tls,
                "connected with SSL required to a server without TLS"
            );
            let cipher = match kind {
                Kind::Postgres => {
                    scalar(
                        &tls,
                        "SELECT cipher FROM pg_stat_ssl WHERE pid = pg_backend_pid()",
                    )
                    .await
                }
                _ => query(&tls, "SHOW SESSION STATUS LIKE 'Ssl_cipher'", true)
                    .await
                    .rows[0][1]
                    .clone(),
            };
            assert!(
                cipher.as_str().is_some_and(|c| !c.is_empty()),
                "not encrypted: {cipher}"
            );
        }
        Err(e) => {
            assert!(
                !offers_tls,
                "the server offers TLS, but SSL required failed: {e}"
            );
            assert!(e.to_string().contains("Turn off \"Require SSL\""), "{e}");
        }
    }
}

#[tokio::test]
async fn connection_errors_are_clear() {
    if let Some(t) = env_target("SAYQL_TEST_PG_URL") {
        check_connection_errors(Kind::Postgres, &t).await;
    }
    if let Some(t) = env_target("SAYQL_TEST_MYSQL_URL") {
        check_connection_errors(Kind::Mysql, &t).await;
    }
}

const MS_SEED: &[&str] = &[
    "CREATE SCHEMA sales",
    "CREATE TABLE dbo.customers (id int IDENTITY PRIMARY KEY, name nvarchar(100) NOT NULL, email varchar(200) NULL)",
    "CREATE TABLE sales.orders (id int PRIMARY KEY, customer_id int NOT NULL REFERENCES dbo.customers(id),
       total decimal(12,2) NOT NULL, status varchar(20) NOT NULL)",
    "CREATE VIEW sales.paid_orders AS SELECT id, total FROM sales.orders WHERE status = 'paid'",
    "CREATE TABLE dbo.all_types (id int PRIMARY KEY, t_tiny tinyint, t_small smallint, t_big bigint, t_bit bit,
       t_real real, t_float float, t_dec decimal(38,10), t_money money, t_nvarchar nvarchar(max), t_char char(3),
       t_guid uniqueidentifier, t_bin varbinary(8), t_date date, t_time time(3), t_dt datetime, t_dt2 datetime2(7),
       t_sdt smalldatetime, t_dto datetimeoffset(0), t_xml xml)",
    "INSERT INTO dbo.customers (name, email) VALUES (N'Zoë', 'zoe@example.com'), (N'李雷', NULL), (N'O''Brien', 'ob@example.com')",
    "INSERT INTO sales.orders VALUES (1, 1, 10.50, 'paid'), (2, 1, 99.99, 'pending'), (3, 2, 0.01, 'paid'),
       (4, 3, 1234567890.12, 'shipped'), (5, 3, 5.00, 'paid')",
    "INSERT INTO dbo.all_types VALUES (1, 255, -32768, 9007199254740993, 1, 1.1, 2.5,
       12345678901234567890.0123456789, 12.34, N'héllo 世界', 'ab', '6F9619FF-8B86-D011-B42D-00C04FC964FF',
       0xDEADBEEF, '2026-10-21', '13:45:30.123', '2026-10-21T13:45:30.997', '2026-10-21T13:45:30.1234567',
       '2026-10-21T13:45:29', '2026-10-21T13:45:30+05:30', '<a>1</a>'),
       (2, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL)",
];

#[tokio::test]
async fn mssql_integration() {
    let Some(t) = env_target("SAYQL_TEST_MSSQL_URL") else {
        return;
    };
    let admin = open(Kind::Mssql, &t, &t.database).await;
    exec(&admin, "IF DB_ID('sayql_it') IS NOT NULL BEGIN ALTER DATABASE sayql_it SET SINGLE_USER WITH ROLLBACK IMMEDIATE; DROP DATABASE sayql_it; END").await;
    exec(&admin, "CREATE DATABASE sayql_it").await;
    let pool = open(Kind::Mssql, &t, "sayql_it").await;
    for sql in MS_SEED {
        exec(&pool, sql).await;
    }

    // ---- browsing databases: list them from the default connection, then switch with USE
    let list = list_databases(&admin)
        .await
        .expect("list")
        .expect("a server has databases");
    assert!(
        list.databases.contains(&"sayql_it".to_string())
            && list.databases.contains(&"master".to_string()),
        "{list:?}"
    );
    assert_eq!(list.current.as_deref(), Some(t.database.as_str()));
    let switched = open_database(
        &admin,
        &config(Kind::Mssql, &t, &t.database),
        t.password.clone(),
        "sayql_it",
    )
    .await
    .expect("open sayql_it");
    assert_eq!(
        scalar(&switched, "SELECT DB_NAME()").await,
        json!("sayql_it")
    );
    assert_eq!(
        introspect(&switched, Kind::Mssql)
            .await
            .expect("introspect")
            .tables
            .len(),
        4
    );
    drop(switched);

    // ---- schema reading
    let schema = introspect(&pool, Kind::Mssql).await.expect("introspect");
    assert_eq!(schema.dialect, "SQL Server");
    let tb = &schema.tables;
    let names: Vec<String> = tb.iter().map(|t| t.qualified_name()).collect();
    assert_eq!(
        names,
        [
            "dbo.all_types",
            "dbo.customers",
            "sales.orders",
            "sales.paid_orders"
        ]
    );
    let orders = table(tb, Some("sales"), "orders");
    assert_eq!(pk_columns(orders), ["id"]);
    assert_eq!(fks(orders), [fk("customer_id", "dbo.customers", "id")]);
    assert_eq!(column(orders, "total").data_type, "decimal(12,2)");
    assert_eq!(
        column(table(tb, Some("dbo"), "customers"), "name").data_type,
        "nvarchar(100)"
    );
    assert_eq!(
        column(table(tb, Some("dbo"), "all_types"), "t_nvarchar").data_type,
        "nvarchar(max)"
    );
    // Types that report a length but don't take one are written bare.
    assert_eq!(
        column(table(tb, Some("dbo"), "all_types"), "t_xml").data_type,
        "xml"
    );
    assert_eq!(
        column(table(tb, Some("dbo"), "all_types"), "t_bin").data_type,
        "varbinary(8)"
    );
    assert!(column(table(tb, Some("dbo"), "customers"), "email").nullable);
    assert!(!column(orders, "status").nullable);
    assert_eq!(table(tb, Some("sales"), "paid_orders").kind, "view");
    assert_eq!(orders.row_estimate, Some(5));
    assert_eq!(table(tb, Some("sales"), "paid_orders").row_estimate, None);

    // ---- value decoding
    let r = query(&pool, "SELECT * FROM dbo.all_types ORDER BY id", true).await;
    check_row(
        "mssql",
        &r,
        0,
        &json!({
            "id": 1, "t_tiny": 255, "t_small": -32768, "t_big": 9007199254740993_i64, "t_bit": true,
            "t_real": 1.1, "t_float": 2.5, "t_dec": "12345678901234567890.0123456789", "t_money": "12.3400",
            "t_nvarchar": "héllo 世界", "t_char": "ab ", "t_guid": "6f9619ff-8b86-d011-b42d-00c04fc964ff",
            "t_bin": "0xdeadbeef", "t_date": "2026-10-21", "t_time": "13:45:30.123",
            "t_dt": "2026-10-21 13:45:30.997", "t_dt2": "2026-10-21 13:45:30.123456700",
            "t_sdt": "2026-10-21 13:45:00", "t_dto": "2026-10-21 13:45:30 +05:30", "t_xml": "<a>1</a>",
        }),
    );
    assert!(r.rows[1][1..].iter().all(Value::is_null), "{:?}", r.rows[1]);
    let r = query(&pool, "SELECT status, COUNT(*) AS n, SUM(total) AS s FROM sales.orders GROUP BY status ORDER BY status", true).await;
    assert_eq!(r.columns, ["status", "n", "s"]);
    assert_eq!(r.rows[0], [json!("paid"), json!(3), json!("15.51")]);
    let r = query(
        &pool,
        "SELECT id, name FROM dbo.customers WHERE 1 = 0",
        true,
    )
    .await;
    assert_eq!(r.columns, ["id", "name"]);
    assert!(r.rows.is_empty());

    // ---- read-only mode: never changes data, however the statement is phrased
    let count = || scalar(&pool, "SELECT COUNT(*) FROM sales.orders");
    let before = count().await;
    for sql in [
        "DELETE FROM sales.orders",
        "SELECT 1; DELETE FROM sales.orders",
        "SELECT 1; COMMIT; DELETE FROM sales.orders",
        "SELECT * INTO dbo.copied FROM sales.orders",
        "WITH x AS (SELECT 1 AS a) DELETE FROM sales.orders",
    ] {
        assert!(
            run_query(&pool, sql, true).await.is_err(),
            "read-only mode allowed: {sql}"
        );
    }
    assert_eq!(count().await, before, "a read-only statement changed data");
    // 1 is the read-only wrapper's own transaction; a leaked one from earlier would make it 2.
    assert_eq!(
        scalar(&pool, "SELECT @@TRANCOUNT").await,
        json!(1),
        "transaction left open"
    );

    // ---- row cap: cut off at 5000 rows, and the connection still works afterwards
    let big = query(
        &pool,
        "SELECT a.object_id FROM sys.all_objects a CROSS JOIN sys.all_objects b",
        true,
    )
    .await;
    assert!(big.truncated);
    assert_eq!(big.rows.len(), MAX_ROWS);
    assert_eq!(scalar(&pool, "SELECT 40 + 2").await, json!(42));

    // ---- an approved write
    let w = query(
        &pool,
        "UPDATE sales.orders SET status = 'paid' WHERE status = 'pending'",
        false,
    )
    .await;
    assert_eq!(w.rows_affected, Some(1));

    drop(pool);
    exec(
        &admin,
        "ALTER DATABASE sayql_it SET SINGLE_USER WITH ROLLBACK IMMEDIATE; DROP DATABASE sayql_it",
    )
    .await;
}

/// A failed statement's error text, and the same text masked for the AI. Reads run read-only
/// and writes as confirmed, as in the app.
async fn error_and_masked(pool: &DbPool, sql: &str) -> (String, String) {
    let read_only = is_read_only(sql, pool.kind());
    let e = run_query(pool, sql, read_only)
        .await
        .err()
        .unwrap_or_else(|| panic!("{sql} succeeded"))
        .to_string();
    let masked = crate::ai::mask_database_error(&e, sql);
    (e, masked)
}

/// Each database's real errors read as the database's message, and their values (not the
/// names the AI needs to fix the query) are masked before they go to the AI.
#[tokio::test]
async fn database_errors_are_readable_and_masked_for_the_ai() {
    if let Some(t) = env_target("SAYQL_TEST_PG_URL") {
        let db = if t.database.is_empty() {
            "postgres".to_string()
        } else {
            t.database.clone()
        };
        let pool = open(Kind::Postgres, &t, &db).await;
        exec(
            &pool,
            "DROP SCHEMA IF EXISTS sayql_err CASCADE; CREATE SCHEMA sayql_err;
            CREATE TABLE sayql_err.u (id int PRIMARY KEY, email text UNIQUE, n int CHECK (n > 0));
            INSERT INTO sayql_err.u VALUES (1, 'ann@example.com', 5);",
        )
        .await;
        let (e, m) = error_and_masked(
            &pool,
            "INSERT INTO sayql_err.u VALUES (2, 'ann@example.com', 5)",
        )
        .await;
        assert_eq!(
            e,
            "duplicate key value violates unique constraint \"u_email_key\"\nDETAIL: Key (email)=(ann@example.com) already exists."
        );
        // The constraint's name isn't in the SQL, so it is masked too.
        assert_eq!(
            m,
            "duplicate key value violates unique constraint \"…\"\nDETAIL: Key (email)=(…) already exists."
        );
        let (e, m) = error_and_masked(&pool, "SELECT email::int FROM sayql_err.u").await;
        assert_eq!(
            e,
            "invalid input syntax for type integer: \"ann@example.com\""
        );
        assert_eq!(m, "invalid input syntax for type integer: \"…\"");
        let (e, m) = error_and_masked(&pool, "SELECT nme FROM sayql_err.u").await;
        assert!(
            e.starts_with("column \"nme\" does not exist")
                && m.starts_with("column \"nme\" does not exist"),
            "{e}"
        );
        exec(&pool, "DROP SCHEMA sayql_err CASCADE").await;
    }
    if let Some(t) = env_target("SAYQL_TEST_MYSQL_URL") {
        let admin = open(Kind::Mysql, &t, &t.database).await;
        exec(
            &admin,
            "DROP DATABASE IF EXISTS sayql_err; CREATE DATABASE sayql_err;
            CREATE TABLE sayql_err.u (id int PRIMARY KEY, email varchar(100) UNIQUE, n int);
            INSERT INTO sayql_err.u VALUES (1, 'ann@example.com', 5);",
        )
        .await;
        let pool = open(Kind::Mysql, &t, "sayql_err").await;
        let (e, m) = error_and_masked(&pool, "INSERT INTO u SELECT 2, email, 5 FROM u").await;
        assert!(
            e.starts_with("1062 (23000): Duplicate entry 'ann@example.com' for key"),
            "{e}"
        );
        assert!(
            m.starts_with("1062 (23000): Duplicate entry '…' for key"),
            "{m}"
        );
        let (_, m) = error_and_masked(
            &pool,
            "INSERT INTO u SELECT 3, 'b@example.com', email FROM u",
        )
        .await;
        assert!(m.contains("value: '…'") && !m.contains("ann@"), "{m}");
        let (_, m) = error_and_masked(&pool, "SELECT nme FROM u").await;
        assert!(
            m.starts_with("1054 (42S22): Unknown column 'nme' in '"),
            "{m}"
        );
        assert!(!m.ends_with("'…'"), "the clause name is not a value: {m}");
        exec(&admin, "DROP DATABASE sayql_err").await;
    }
    if let Some(t) = env_target("SAYQL_TEST_MSSQL_URL") {
        let admin = open(Kind::Mssql, &t, &t.database).await;
        exec(&admin, "IF DB_ID('sayql_err') IS NOT NULL BEGIN ALTER DATABASE sayql_err SET SINGLE_USER WITH ROLLBACK IMMEDIATE; DROP DATABASE sayql_err; END").await;
        exec(&admin, "CREATE DATABASE sayql_err").await;
        let pool = open(Kind::Mssql, &t, "sayql_err").await;
        exec(&pool, "CREATE TABLE dbo.u (id int PRIMARY KEY, email nvarchar(100) CONSTRAINT u_email UNIQUE, n int);
            INSERT INTO dbo.u VALUES (1, N'ann@example.com', 5);").await;
        let (e, m) =
            error_and_masked(&pool, "INSERT INTO dbo.u SELECT 2, email, 5 FROM dbo.u").await;
        assert_eq!(
            e,
            "Violation of UNIQUE KEY constraint 'u_email'. Cannot insert duplicate key in object 'dbo.u'. \
             The duplicate key value is (ann@example.com).\n(SQL Server error 2627, line 1)"
        );
        assert_eq!(
            m,
            "Violation of UNIQUE KEY constraint '…'. Cannot insert duplicate key in object 'dbo.u'. \
             The duplicate key value is (…).\n(SQL Server error 2627, line 1)"
        );
        let (_, m) = error_and_masked(&pool, "SELECT CAST(email AS int) FROM dbo.u").await;
        assert!(
            m.starts_with(
                "Conversion failed when converting the nvarchar value '…' to data type int."
            ) && !m.contains("ann@"),
            "{m}"
        );
        let (_, m) = error_and_masked(&pool, "SELECT id\nFROM dbo.u\nWHERE nme = 1").await;
        // The column name stays; which line SQL Server reports depends on how the batch ran.
        assert!(
            m.starts_with("Invalid column name 'nme'.\n(SQL Server error 207, line "),
            "{m}"
        );
        drop(pool);
        exec(&admin, "ALTER DATABASE sayql_err SET SINGLE_USER WITH ROLLBACK IMMEDIATE; DROP DATABASE sayql_err").await;
    }
}

#[tokio::test]
async fn mssql_connection_errors() {
    let Some(t) = env_target("SAYQL_TEST_MSSQL_URL") else {
        return;
    };

    // ---- TLS: the login is always encrypted, so an untrusted certificate fails with the hint
    let untrusted = ConnConfig {
        trust_cert: false,
        ..config(Kind::Mssql, &t, &t.database)
    };
    let err = connect(&untrusted, t.password.clone())
        .await
        .err()
        .expect("untrusted certificate accepted")
        .to_string();
    assert!(err.contains("Trust server certificate"), "{err}");

    // ---- an encrypted connection works end to end once the certificate is trusted
    let encrypted = ConnConfig {
        ssl: true,
        ..config(Kind::Mssql, &t, &t.database)
    };
    let pool = connect(&encrypted, t.password.clone())
        .await
        .expect("connect with encryption");
    assert_eq!(scalar(&pool, "SELECT 40 + 2").await, json!(42));

    // ---- a wrong password gets the credentials hint
    let err = connect(&config(Kind::Mssql, &t, &t.database), Some("wrong".into()))
        .await
        .err()
        .expect("wrong password accepted");
    assert!(
        err.to_string()
            .contains("didn't accept this user and password"),
        "{err}"
    );
}

/// The names in the schema text the AI sees must work when copied into a query: mixed case,
/// spaces and reserved words are quoted the way each database needs.
async fn check_quoted_names(pool: &DbPool, kind: Kind, schema: Option<&str>) {
    let s = introspect(pool, kind).await.expect("introspect");
    let t = table(&s.tables, schema, "Order Lines");
    let dialect = kind.dialect();
    let line = crate::schema::render_table(t, dialect);
    let q = |n: &str| crate::schema::quote_ident(n, dialect);
    for name in ["Id", "first name", "order"] {
        assert!(line.contains(&format!("{} ", q(name))), "{name} in {line}");
    }
    let cols: Vec<String> = t.columns.iter().map(|c| q(&c.name)).collect();
    let sql = format!(
        "SELECT {} FROM {}",
        cols.join(", "),
        crate::schema::render_name(t, dialect)
    );
    let r = query(pool, &sql, true).await;
    assert_eq!(r.columns, ["Id", "first name", "order"], "{sql}");
    assert_eq!(r.rows, [[json!(1), json!("Ann"), json!(2)]], "{sql}");
}

#[tokio::test]
async fn quoted_names_work_as_written() {
    if let Some(t) = env_target("SAYQL_TEST_PG_URL") {
        let db = if t.database.is_empty() {
            "postgres".to_string()
        } else {
            t.database.clone()
        };
        let pool = open(Kind::Postgres, &t, &db).await;
        exec(&pool, "DROP SCHEMA IF EXISTS \"Sayql Q\" CASCADE; CREATE SCHEMA \"Sayql Q\";
            CREATE TABLE \"Sayql Q\".\"Order Lines\" (\"Id\" int PRIMARY KEY, \"first name\" text, \"order\" int);
            INSERT INTO \"Sayql Q\".\"Order Lines\" VALUES (1, 'Ann', 2);").await;
        check_quoted_names(&pool, Kind::Postgres, Some("Sayql Q")).await;
        exec(&pool, "DROP SCHEMA \"Sayql Q\" CASCADE").await;
    }
    if let Some(t) = env_target("SAYQL_TEST_MYSQL_URL") {
        let admin = open(Kind::Mysql, &t, &t.database).await;
        exec(&admin, "DROP DATABASE IF EXISTS sayql_q; CREATE DATABASE sayql_q;
            CREATE TABLE sayql_q.`Order Lines` (`Id` int PRIMARY KEY, `first name` text, `order` int);
            INSERT INTO sayql_q.`Order Lines` VALUES (1, 'Ann', 2);").await;
        let pool = open(Kind::Mysql, &t, "sayql_q").await;
        check_quoted_names(&pool, Kind::Mysql, None).await;
        exec(&admin, "DROP DATABASE sayql_q").await;
    }
    if let Some(t) = env_target("SAYQL_TEST_MSSQL_URL") {
        let admin = open(Kind::Mssql, &t, &t.database).await;
        exec(&admin, "IF DB_ID('sayql_q') IS NOT NULL BEGIN ALTER DATABASE sayql_q SET SINGLE_USER WITH ROLLBACK IMMEDIATE; DROP DATABASE sayql_q; END").await;
        exec(&admin, "CREATE DATABASE sayql_q").await;
        let pool = open(Kind::Mssql, &t, "sayql_q").await;
        exec(&pool, "EXEC('CREATE SCHEMA [Sayql Q]')").await;
        exec(&pool, "CREATE TABLE [Sayql Q].[Order Lines] ([Id] int PRIMARY KEY, [first name] nvarchar(20), [order] int);
            INSERT INTO [Sayql Q].[Order Lines] VALUES (1, N'Ann', 2);").await;
        check_quoted_names(&pool, Kind::Mssql, Some("Sayql Q")).await;
        drop(pool);
        exec(
            &admin,
            "ALTER DATABASE sayql_q SET SINGLE_USER WITH ROLLBACK IMMEDIATE; DROP DATABASE sayql_q",
        )
        .await;
    }
}

/// Cancels `sql` after a second, and checks it stopped on the server (`running` counts it
/// there) and that the pool still works afterwards.
async fn check_cancel(pool: &DbPool, sql: &str, read_only: bool, running: &str) {
    let cancel = tokio_util::sync::CancellationToken::new();
    let timer = tokio::spawn({
        let cancel = cancel.clone();
        async move {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            cancel.cancel();
        }
    });
    let started = std::time::Instant::now();
    let r = run_query_with(pool, sql, read_only, MAX_ROWS, &cancel).await;
    timer.await.unwrap();
    assert_eq!(
        r.err().map(|e| e.to_string()).as_deref(),
        Some(CANCELLED),
        "{sql}"
    );
    assert!(
        started.elapsed() < std::time::Duration::from_secs(10),
        "{sql}: took {:?}",
        started.elapsed()
    );
    // The server may take a moment to clear the request.
    let mut left = json!(-1);
    for _ in 0..20 {
        left = query(pool, running, true).await.rows[0][0].clone();
        if left == json!(0) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }
    assert_eq!(left, json!(0), "{sql} is still running on the server");
    assert_eq!(scalar(pool, "SELECT 40 + 2").await, json!(42));
}

/// A cancelled query stops on the server, read-only or confirmed, instead of running on.
#[tokio::test]
async fn cancelled_queries_stop_on_the_server() {
    if let Some(t) = env_target("SAYQL_TEST_PG_URL") {
        let db = if t.database.is_empty() {
            "postgres".to_string()
        } else {
            t.database.clone()
        };
        let pool = open(Kind::Postgres, &t, &db).await;
        let running = "SELECT count(*) FROM pg_stat_activity WHERE query = 'SELECT pg_sleep(30)' AND state = 'active'";
        check_cancel(&pool, "SELECT pg_sleep(30)", true, running).await;
        check_cancel(&pool, "SELECT pg_sleep(30)", false, running).await;
    }
    if let Some(t) = env_target("SAYQL_TEST_MYSQL_URL") {
        let pool = open(Kind::Mysql, &t, &t.database).await;
        let running =
            "SELECT COUNT(*) FROM information_schema.PROCESSLIST WHERE INFO = 'SELECT SLEEP(30)'";
        check_cancel(&pool, "SELECT SLEEP(30)", true, running).await;
        check_cancel(&pool, "SELECT SLEEP(30)", false, running).await;
    }
    if let Some(t) = env_target("SAYQL_TEST_MSSQL_URL") {
        let pool = open(Kind::Mssql, &t, &t.database).await;
        let running = "SELECT COUNT(*) FROM sys.dm_exec_requests WHERE wait_type = 'WAITFOR' AND session_id <> @@SPID";
        check_cancel(
            &pool,
            "SELECT 1 AS a; WAITFOR DELAY '00:00:30'",
            true,
            running,
        )
        .await;
        check_cancel(&pool, "WAITFOR DELAY '00:00:30'", false, running).await;
    }
}

/// MariaDB accounts that use the ed25519 plugin sign in (sqlx-mysql is patched for it).
#[tokio::test]
async fn mariadb_ed25519_accounts_sign_in() {
    let Some(t) = env_target("SAYQL_TEST_MYSQL_URL") else {
        return;
    };
    let admin = open(Kind::Mysql, &t, &t.database).await;
    let version = scalar(&admin, "SELECT VERSION()").await;
    if !version.as_str().is_some_and(|v| v.contains("MariaDB")) {
        return;
    }
    // Loaded by default on some versions; installing it twice is an error worth ignoring.
    let _ = run_query(&admin, "INSTALL SONAME 'auth_ed25519'", false).await;
    let pw = format!("Ed-{}", uuid::Uuid::new_v4().simple());
    exec(&admin, "DROP USER IF EXISTS 'sayql_ed'@'%'").await;
    exec(
        &admin,
        &format!("CREATE USER 'sayql_ed'@'%' IDENTIFIED VIA ed25519 USING PASSWORD('{pw}')"),
    )
    .await;
    let user = Target {
        user: "sayql_ed".into(),
        password: Some(pw),
        ..parse_url(&std::env::var("SAYQL_TEST_MYSQL_URL").unwrap())
    };
    let pool = connect(&config(Kind::Mysql, &user, ""), user.password.clone())
        .await
        .expect("ed25519 sign-in");
    assert_eq!(
        scalar(&pool, "SELECT CURRENT_USER()").await,
        json!("sayql_ed@%")
    );
    let wrong = connect(&config(Kind::Mysql, &user, ""), Some("wrong".into()))
        .await
        .err()
        .expect("wrong password accepted");
    assert!(wrong.to_string().contains("Access denied"), "{wrong}");
    drop(pool);
    exec(&admin, "DROP USER 'sayql_ed'@'%'").await;
}
