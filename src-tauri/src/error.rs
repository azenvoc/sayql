//! The one error type used across the backend. Commands return it to the UI, where it
//! arrives as its display string.

use serde::{Serialize, Serializer};
use std::fmt;

/// Sent instead of a message when a statement needs the user's confirmation.
/// Must match `WRITE_CONFIRM` in src/api/types.ts.
pub const WRITE_CONFIRM: &str = "WRITE_CONFIRM";

#[derive(Debug)]
pub enum AppError {
    /// The statement changes data and the caller hasn't confirmed it yet.
    NeedsWriteConfirmation,
    /// A message written for the person using the app.
    User(String),
    /// A team server refused the request by its rules (read-only, never send, licence, sign-in):
    /// not a database error, so the UI offers no AI fix.
    TeamRefused(String),
    Database(sqlx::Error),
    SqlServer(tiberius::error::Error),
    Io(std::io::Error),
    Keychain(keyring::Error),
    Json(serde_json::Error),
}

pub type AppResult<T> = Result<T, AppError>;

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AppError::NeedsWriteConfirmation => f.write_str(WRITE_CONFIRM),
            AppError::User(msg) | AppError::TeamRefused(msg) => f.write_str(msg),
            AppError::Database(e) => f.write_str(&database_error(e)),
            AppError::SqlServer(e) => f.write_str(&sql_server_error(e)),
            AppError::Io(e) => write!(f, "File error: {e}"),
            AppError::Keychain(e) => write!(f, "Keychain error: {e}"),
            AppError::Json(e) => write!(f, "Could not read saved data: {e}"),
        }
    }
}

/// The database's own message, without sqlx's "error returned from database:" prefix or the
/// line in the Postgres source code that raised it (which reads like a line of the query).
/// Postgres' DETAIL and HINT lines are kept: they name the key or value at fault and the fix.
/// Values in them are masked before an error goes to the AI (see safety/redact.rs).
fn database_error(e: &sqlx::Error) -> String {
    match e {
        sqlx::Error::Database(db) => match db.try_downcast_ref::<sqlx::postgres::PgDatabaseError>()
        {
            Some(pg) => {
                let mut out = pg.message().to_string();
                if let Some(detail) = pg.detail() {
                    out.push_str(&format!("\nDETAIL: {detail}"));
                }
                if let Some(hint) = pg.hint() {
                    out.push_str(&format!("\nHINT: {hint}"));
                }
                out
            }
            None => db.to_string(),
        },
        sqlx::Error::Io(io) => format!("Lost the connection to the database: {io}"),
        sqlx::Error::PoolTimedOut => "Timed out waiting for a connection to the database.".into(),
        other => other.to_string(),
    }
}

/// SQL Server's message without tiberius' wrapping (`Token error: '…' on server … executing …
/// on line 1 (code: …, state: …, class: …)`): the quotes around the whole message made it
/// look like one quoted value, so masking values for the AI hid the column and table names too.
/// The error number and line go on a line of their own, as in SQL Server's own tools.
fn sql_server_error(e: &tiberius::error::Error) -> String {
    match e {
        tiberius::error::Error::Server(t) => format!(
            "{}\n(SQL Server error {}, line {})",
            t.message(),
            t.code(),
            t.line()
        ),
        other => other.to_string(),
    }
}

impl std::error::Error for AppError {}

/// Errors reach the UI as their message. A team server's refusal comes as
/// `{"message": "…", "teamRefused": true}` (see `isTeamRefusal` in src/lib/errors.ts).
impl Serialize for AppError {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            AppError::TeamRefused(msg) => {
                use serde::ser::SerializeStruct;
                let mut st = s.serialize_struct("TeamRefused", 2)?;
                st.serialize_field("message", msg)?;
                st.serialize_field("teamRefused", &true)?;
                st.end()
            }
            other => s.serialize_str(&other.to_string()),
        }
    }
}

impl From<String> for AppError {
    fn from(msg: String) -> Self {
        AppError::User(msg)
    }
}

impl From<&str> for AppError {
    fn from(msg: &str) -> Self {
        AppError::User(msg.to_string())
    }
}

impl From<sqlx::Error> for AppError {
    fn from(e: sqlx::Error) -> Self {
        AppError::Database(e)
    }
}

impl From<tiberius::error::Error> for AppError {
    fn from(e: tiberius::error::Error) -> Self {
        AppError::SqlServer(e)
    }
}

impl From<std::io::Error> for AppError {
    fn from(e: std::io::Error) -> Self {
        AppError::Io(e)
    }
}

impl From<keyring::Error> for AppError {
    fn from(e: keyring::Error) -> Self {
        AppError::Keychain(e)
    }
}

impl From<serde_json::Error> for AppError {
    fn from(e: serde_json::Error) -> Self {
        AppError::Json(e)
    }
}
