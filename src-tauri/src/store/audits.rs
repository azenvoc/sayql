//! When an AI app last fetched SayQL's `audit` prompt over MCP, per connection and database,
//! so Settings → AI apps can say when a database was last audited and suggest running it
//! again. Written by the MCP server (`sayql-mcp`), read by the app.

use super::Store;
use crate::error::AppResult;
use serde::{Deserialize, Serialize};

const FILE: &str = "mcp-audits.json";

/// The last audit of one database.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct AuditRun {
    pub connection_id: String,
    /// `None`: a SQLite file, or a connection without a default database.
    #[serde(default)]
    pub database: Option<String>,
    /// The AI app's name, as it gave it when it connected.
    pub app: String,
    /// Milliseconds since the Unix epoch.
    pub at: u64,
}

impl Store {
    /// The last audit of every database audited, in no particular order.
    pub fn audits(&self) -> AppResult<Vec<AuditRun>> {
        self.read(FILE)
    }

    /// Records that `app` fetched the audit prompt for this database now.
    pub fn record_audit(
        &self,
        connection_id: &str,
        database: Option<&str>,
        app: &str,
    ) -> AppResult<AuditRun> {
        let database = database
            .map(str::trim)
            .filter(|d| !d.is_empty())
            .map(str::to_string);
        let run = AuditRun {
            connection_id: connection_id.to_string(),
            database,
            app: app.to_string(),
            at: chrono::Utc::now().timestamp_millis().max(0) as u64,
        };
        let mut all = self.audits()?;
        all.retain(|a| !(a.connection_id == run.connection_id && a.database == run.database));
        all.push(run.clone());
        self.write(FILE, &all)?;
        Ok(run)
    }

    /// Forgets a deleted connection's audits.
    pub(super) fn forget_audits(&self, connection_id: &str) -> AppResult<()> {
        let mut all = self.audits()?;
        let before = all.len();
        all.retain(|a| a.connection_id != connection_id);
        if all.len() != before {
            self.write(FILE, &all)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_the_last_audit_per_database() {
        let dir = std::env::temp_dir().join(format!("sayql-audits-{}", uuid::Uuid::new_v4()));
        let s = Store::new(dir.clone());
        assert!(s.audits().unwrap().is_empty());
        s.record_audit("c1", Some("shop"), "Cursor").unwrap();
        s.record_audit("c1", Some(" "), "Cursor").unwrap();
        let last = s.record_audit("c1", Some("shop"), "Claude Code").unwrap();
        s.record_audit("c2", None, "Claude Code").unwrap();
        let all = s.audits().unwrap();
        assert_eq!(all.len(), 3, "one per connection and database");
        assert!(all.contains(&last));
        assert!(
            all.iter()
                .any(|a| a.connection_id == "c1" && a.database.is_none())
        );
        s.forget_agent("c1").unwrap();
        let left = s.audits().unwrap();
        assert_eq!(
            left.iter()
                .map(|a| a.connection_id.as_str())
                .collect::<Vec<_>>(),
            ["c2"]
        );
        let _ = std::fs::remove_dir_all(dir);
    }
}
