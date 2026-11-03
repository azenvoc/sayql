//! The databases on a server, and connections to one of them.

use super::{ConnConfig, DbPool, connect};
use crate::error::AppResult;
use serde::Serialize;
use sqlx::Row;

#[derive(Debug, Clone, Serialize, specta::Type)]
pub struct DatabaseList {
    /// Databases this login can open, sorted by name.
    pub databases: Vec<String>,
    /// The database the connection opens by default.
    pub current: Option<String>,
}

const MSSQL_DATABASES: &str = "SELECT name FROM sys.databases \
     WHERE HAS_DBACCESS(name) = 1 AND state_desc = 'ONLINE' ORDER BY name";

const PG_DATABASES: &str = "SELECT datname::text FROM pg_database \
     WHERE datallowconn AND NOT datistemplate AND has_database_privilege(datname, 'CONNECT') ORDER BY datname";

/// MySQL's own bookkeeping databases.
const MYSQL_SYSTEM: &[&str] = &["information_schema", "mysql", "performance_schema", "sys"];

/// `None` for SQLite, where a file is a single database.
pub async fn list_databases(pool: &DbPool) -> AppResult<Option<DatabaseList>> {
    let list = match pool {
        DbPool::Lite(_) | DbPool::Team(_) => return Ok(None),
        DbPool::Ms(p) => {
            let mut conn = p.acquire().await?;
            let client = conn.client();
            let rows = client
                .simple_query(MSSQL_DATABASES)
                .await?
                .into_first_result()
                .await?;
            let databases = rows
                .iter()
                .filter_map(|r| r.try_get::<&str, _>(0).ok().flatten().map(str::to_string))
                .collect();
            let current = client
                .simple_query("SELECT DB_NAME()")
                .await?
                .into_row()
                .await?;
            let current =
                current.and_then(|r| r.try_get::<&str, _>(0).ok().flatten().map(str::to_string));
            DatabaseList { databases, current }
        }
        DbPool::Pg(p) => {
            // One transaction keeps both on one server connection behind a pooler (see introspect/postgres.rs).
            let mut tx = p.begin().await?;
            sqlx::raw_sql("DEALLOCATE ALL").execute(&mut *tx).await?;
            let databases = sqlx::query_scalar(PG_DATABASES).fetch_all(&mut *tx).await?;
            let current = sqlx::query_scalar("SELECT current_database()::text")
                .fetch_one(&mut *tx)
                .await
                .ok();
            tx.rollback().await?;
            DatabaseList { databases, current }
        }
        DbPool::My(p) => {
            let rows = sqlx::query(
                "SELECT CAST(SCHEMA_NAME AS CHAR) FROM information_schema.SCHEMATA ORDER BY 1",
            )
            .fetch_all(p)
            .await?;
            let databases = rows
                .iter()
                .filter_map(|r| r.try_get::<String, _>(0).ok())
                .filter(|d| !MYSQL_SYSTEM.contains(&d.as_str()))
                .collect();
            let current: Option<String> = sqlx::query_scalar("SELECT CAST(DATABASE() AS CHAR)")
                .fetch_one(p)
                .await?;
            DatabaseList { databases, current }
        }
    };
    Ok(Some(list))
}

/// A connection that works in `database`, opened alongside `base`.
pub async fn open_database(
    base: &DbPool,
    cfg: &ConnConfig,
    password: Option<String>,
    database: &str,
) -> AppResult<DbPool> {
    match base {
        DbPool::Ms(p) => Ok(DbPool::Ms(p.for_database(database).await?)),
        DbPool::Team(_) => {
            Err("A team connection opens the one database your team admin set up.".into())
        }
        _ => {
            connect(
                &ConnConfig {
                    database: database.to_string(),
                    ..cfg.clone()
                },
                password,
            )
            .await
        }
    }
}
