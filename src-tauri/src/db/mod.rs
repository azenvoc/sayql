//! Database access for PostgreSQL, MySQL, SQLite and SQL Server: connecting, running queries
//! safely, decoding values for display and reading the schema.

mod classify;
mod connect;
mod databases;
mod decode;
mod demo;
mod execute;
mod introspect;
mod mssql_pool;
pub mod schema_hash;
pub mod usage;

pub use classify::is_read_only;
pub use connect::connect;
pub use databases::{DatabaseList, list_databases, open_database};
pub use demo::ensure_demo;
pub use execute::{CANCELLED, run_query, run_query_with};
pub use introspect::introspect;
pub use mssql_pool::MssqlPool;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{MySqlPool, PgPool, SqlitePool};

/// Rows beyond this are not sent to the UI.
pub(crate) const MAX_ROWS: usize = 5000;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Postgres,
    Mysql,
    Sqlite,
    /// Microsoft SQL Server and Azure SQL.
    Mssql,
}

impl Kind {
    pub fn dialect(&self) -> &'static str {
        match self {
            Kind::Postgres => "PostgreSQL",
            Kind::Mysql => "MySQL",
            Kind::Sqlite => "SQLite",
            Kind::Mssql => "SQL Server",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnConfig {
    pub id: String,
    pub name: String,
    pub kind: Kind,
    #[serde(default)]
    pub host: String,
    #[serde(default)]
    pub port: Option<u16>,
    #[serde(default)]
    pub database: String,
    #[serde(default)]
    pub user: String,
    /// SQLite file path.
    #[serde(default)]
    pub file: String,
    /// Require an encrypted connection.
    #[serde(default)]
    pub ssl: bool,
    /// Accept the server's TLS certificate without verifying it (self-signed internal servers).
    #[serde(default)]
    pub trust_cert: bool,
    /// Tagged as production by the person: the app shows a PROD badge. Display only; it
    /// changes nothing about how queries run. Off for connections saved before 0.4.8.
    #[serde(default)]
    pub production: bool,
}

#[derive(Clone)]
#[allow(
    clippy::large_enum_variant,
    reason = "one pool per connection, cloned rarely; boxing adds an indirection to every query"
)]
pub enum DbPool {
    Pg(PgPool),
    My(MySqlPool),
    Lite(LitePools),
    Ms(MssqlPool),
    /// A SayQL Team connection: queries go to the team server, never to the database directly.
    Team(crate::team::TeamPool),
}

impl DbPool {
    pub fn kind(&self) -> Kind {
        match self {
            DbPool::Pg(_) => Kind::Postgres,
            DbPool::My(_) => Kind::Mysql,
            DbPool::Lite(_) => Kind::Sqlite,
            DbPool::Ms(_) => Kind::Mssql,
            DbPool::Team(t) => t.kind,
        }
    }

    /// Tags queries on a team connection with where they came from and the question asked,
    /// for the team server's audit log. Other pools are unchanged.
    pub fn with_context(self, source: crate::team::Source, question: Option<String>) -> DbPool {
        match self {
            DbPool::Team(t) => DbPool::Team(crate::team::TeamPool {
                source,
                question,
                ..t
            }),
            other => other,
        }
    }
}

/// SQLite runs every statement in a script and lets a script turn `PRAGMA query_only`
/// back off, so read-only queries get connections opened read-only instead.
#[derive(Clone)]
pub struct LitePools {
    pub(crate) rw: SqlitePool,
    pub(crate) ro: SqlitePool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryResult {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Value>>,
    pub rows_affected: Option<u64>,
    pub truncated: bool,
    pub elapsed_ms: u128,
    /// Columns a team server left out because its admin marked them never send.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub withheld: Vec<String>,
}

#[cfg(test)]
mod integration_tests;
#[cfg(test)]
pub(crate) use integration_tests::env_config;
#[cfg(test)]
pub(crate) mod tenant_fixture;
#[cfg(test)]
mod tests;
