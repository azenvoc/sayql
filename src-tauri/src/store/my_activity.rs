//! Your own activity: the queries you ran in SayQL, from the editor, the Ask box or Investigate.
//! It stays on this computer, like the rest of the store.
//!
//! JSON Lines, appended by the app; past [`MAX_LOG_BYTES`] the file moves to `.1` (replacing the
//! older one), so it keeps roughly the last two files' worth. It holds the SQL and counts, never
//! rows.

use super::Store;
use super::jsonl::{JsonlLog, cut};
use crate::error::AppResult;
use serde::{Deserialize, Serialize};

const LOG: JsonlLog = JsonlLog {
    file: "my-activity.jsonl",
    old: "my-activity.1.jsonl",
    // Size at which the log starts a new file.
    max_bytes: 1024 * 1024,
};
/// Longest SQL kept per entry.
const MAX_SQL: usize = 4000;
/// Longest error or detail kept per entry.
const MAX_DETAIL: usize = 300;

/// One query you ran.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct MyActivity {
    /// Milliseconds since the Unix epoch.
    pub at: u64,
    pub connection_id: String,
    #[serde(default)]
    pub database: Option<String>,
    /// "editor", "ask" or "investigate".
    pub source: String,
    pub sql: String,
    /// ok, error or cancelled.
    pub status: String,
    #[serde(default)]
    pub rows: Option<usize>,
    #[serde(default)]
    pub ms: Option<u64>,
    /// The error, or for Investigate what went to the AI ("sent row count only").
    #[serde(default)]
    pub detail: Option<String>,
    /// The question it answered, when there was one.
    #[serde(default)]
    pub question: Option<String>,
}

impl MyActivity {
    pub fn new(connection_id: &str, database: Option<&str>, source: &str, sql: &str) -> Self {
        MyActivity {
            at: chrono::Utc::now().timestamp_millis().max(0) as u64,
            connection_id: connection_id.to_string(),
            database: database.map(str::to_string),
            source: source.to_string(),
            sql: sql.to_string(),
            status: "ok".into(),
            rows: None,
            ms: None,
            detail: None,
            question: None,
        }
    }
}

impl Store {
    /// Appends one entry. Errors are the caller's to ignore: a log that can't be written must
    /// not stop the query.
    pub fn log_my_activity(&self, mut entry: MyActivity) -> AppResult<()> {
        cut(&mut entry.sql, MAX_SQL);
        if let Some(d) = entry.detail.as_mut() {
            cut(d, MAX_DETAIL);
        }
        if let Some(q) = entry.question.as_mut() {
            cut(q, MAX_DETAIL);
        }
        LOG.append(self.dir(), &entry)
    }

    /// The newest `limit` entries, newest first; for one connection when given. Lines that
    /// don't parse are skipped.
    pub fn my_activity(
        &self,
        connection_id: Option<&str>,
        limit: usize,
    ) -> AppResult<Vec<MyActivity>> {
        let keep = |a: &MyActivity| connection_id.is_none_or(|id| a.connection_id == id);
        LOG.read(self.dir(), keep, limit)
    }

    /// Deletes your activity log.
    pub fn clear_my_activity(&self) -> AppResult<()> {
        LOG.clear(self.dir())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn store() -> (Store, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("sayql-my-activity-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        (Store::new(dir.clone()), dir)
    }

    #[test]
    fn logs_newest_first_per_connection_and_clears() {
        let (s, dir) = store();
        s.log_my_activity(MyActivity::new("a", None, "editor", "SELECT 1"))
            .unwrap();
        s.log_my_activity(MyActivity::new("b", Some("shop"), "ask", "SELECT 2"))
            .unwrap();
        s.log_my_activity(MyActivity::new("a", None, "investigate", "SELECT 3"))
            .unwrap();
        std::fs::OpenOptions::new()
            .append(true)
            .open(dir.join(LOG.file))
            .unwrap()
            .write_all(b"not json\n")
            .unwrap();
        let all = s.my_activity(None, 10).unwrap();
        assert_eq!(
            all.iter().map(|a| a.sql.as_str()).collect::<Vec<_>>(),
            ["SELECT 3", "SELECT 2", "SELECT 1"]
        );
        let a = s.my_activity(Some("a"), 10).unwrap();
        assert_eq!(a.len(), 2);
        assert_eq!(s.my_activity(None, 1).unwrap().len(), 1);
        s.clear_my_activity().unwrap();
        assert!(s.my_activity(None, 10).unwrap().is_empty());
    }

    #[test]
    fn long_sql_and_errors_are_cut() {
        let (s, _dir) = store();
        let mut e = MyActivity::new("a", None, "editor", &"x".repeat(MAX_SQL + 50));
        e.detail = Some("é".repeat(MAX_DETAIL));
        s.log_my_activity(e).unwrap();
        let got = &s.my_activity(None, 1).unwrap()[0];
        assert!(got.sql.ends_with('…') && got.sql.len() <= MAX_SQL + 3);
        assert!(got.detail.as_ref().unwrap().ends_with('…'));
    }
}
