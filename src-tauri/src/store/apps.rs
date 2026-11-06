//! What AI apps did over MCP (the activity log), which apps the person blocked and, when they
//! only let in apps they approved, which ones they approved.
//!
//! The log is JSON Lines, appended by the MCP server (`sayql-mcp`, one process per AI app) and
//! read by the app. One small `write` per line in append mode keeps lines whole when two AI apps
//! write at once. Past [`MAX_LOG_BYTES`] the file is moved to `.1` (replacing the older one), so
//! the log keeps roughly the last two files' worth. It holds the SQL and counts, never rows.

use super::Store;
use super::jsonl::{JsonlLog, cut};
use crate::error::AppResult;
use crate::store::agent::AppResults;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

const LOG: JsonlLog = JsonlLog {
    file: "mcp-activity.jsonl",
    old: "mcp-activity.1.jsonl",
    max_bytes: MAX_LOG_BYTES,
};
const APPS_FILE: &str = "mcp-apps.json";
/// Size at which the log starts a new file.
const MAX_LOG_BYTES: u64 = 1024 * 1024;
/// Longest SQL kept per entry.
const MAX_SQL: usize = 4000;
/// The tools counted as queries in an app's summary.
const QUERY_TOOLS: &[&str] = &["run_query", "export_csv"];
/// What an AI app that doesn't say its name is called.
pub const UNKNOWN_APP: &str = "Unknown app";
/// The `detail` logged when an app was refused for not being approved.
pub const NOT_APPROVED: &str = "not approved";

/// One thing an AI app did, or tried, over MCP.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct Activity {
    /// Milliseconds since the Unix epoch.
    pub at: u64,
    /// The AI app's name, as it gave it (Claude Code, Cursor, ...).
    pub app: String,
    /// The MCP tool: run_query, export_csv, ...
    pub tool: String,
    #[serde(default)]
    pub connection_id: String,
    #[serde(default)]
    pub connection: String,
    #[serde(default)]
    pub sql: String,
    /// ok, refused or error.
    pub status: String,
    /// Why it was refused or failed, in a few words.
    #[serde(default)]
    pub detail: Option<String>,
    /// Rows the query returned (the app may have seen fewer, or only a summary).
    #[serde(default)]
    pub rows: Option<usize>,
    /// What the connection let AI apps see at the time.
    #[serde(default)]
    pub may_see: Option<AppResults>,
    /// The database a query ran in, when it named or resolved one.
    #[serde(default)]
    pub database: Option<String>,
    /// The workspace this `sayql-mcp` serves.
    #[serde(default)]
    pub workspace: Option<String>,
    /// The group this `sayql-mcp` is limited to.
    #[serde(default)]
    pub group: Option<String>,
}

impl Activity {
    pub fn new(app: &str, tool: &str) -> Self {
        Activity {
            at: chrono::Utc::now().timestamp_millis().max(0) as u64,
            app: app.to_string(),
            tool: tool.to_string(),
            connection_id: String::new(),
            connection: String::new(),
            sql: String::new(),
            status: "ok".into(),
            detail: None,
            rows: None,
            may_see: None,
            database: None,
            workspace: None,
            group: None,
        }
    }
}

/// An AI app SayQL has seen, or that the person blocked or approved.
#[derive(Debug, Clone, PartialEq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct AppSummary {
    pub name: String,
    pub blocked: bool,
    /// On the approved list (what counts while only approved apps are let in).
    pub approved: bool,
    /// Queries and exports in the log.
    pub queries: usize,
    /// When it last asked for anything, refused or not.
    pub last_used: Option<u64>,
}

/// The apps, and whether only approved ones are let in.
#[derive(Debug, Clone, PartialEq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct AppList {
    pub only_approved: bool,
    pub apps: Vec<AppSummary>,
}

/// What [`Store::app_gate`] decided for one app.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppGate {
    Allowed,
    Blocked,
    /// Only approved apps are let in, and this one isn't approved.
    NotApproved,
}

/// mcp-apps.json. Every field defaults, so an older file (only `blocked`) reads as "any app
/// that isn't blocked may connect".
#[derive(Debug, Default, Serialize, Deserialize)]
struct Apps {
    #[serde(default)]
    blocked: Vec<String>,
    #[serde(default)]
    only_approved: bool,
    #[serde(default)]
    approved: Vec<String>,
}

fn has(list: &[String], app: &str) -> bool {
    list.iter().any(|b| b.eq_ignore_ascii_case(app))
}

impl Store {
    /// Appends one entry to the activity log. Errors are the caller's to ignore: a log that
    /// can't be written must not stop the query.
    pub fn log_app_activity(&self, mut entry: Activity) -> AppResult<()> {
        cut(&mut entry.sql, MAX_SQL);
        LOG.append(self.dir(), &entry)
    }

    /// The newest `limit` entries, newest first. Lines that don't parse are skipped.
    pub fn app_activity(&self, limit: usize) -> AppResult<Vec<Activity>> {
        LOG.read(self.dir(), |_: &Activity| true, limit)
    }

    /// Deletes the activity log. The blocked apps stay.
    pub fn clear_app_activity(&self) -> AppResult<()> {
        LOG.clear(self.dir())
    }

    /// Whether the person blocked `app` (by the name the app reports). An unreadable list is
    /// an error, never "not blocked".
    pub fn is_app_blocked(&self, app: &str) -> AppResult<bool> {
        Ok(has(&self.read::<Apps>(APPS_FILE)?.blocked, app))
    }

    /// Whether `app` (by the name it reports) may use SayQL. Blocked wins over approved. An
    /// unreadable file is an error, never "allowed".
    pub fn app_gate(&self, app: &str) -> AppResult<AppGate> {
        let apps: Apps = self.read(APPS_FILE)?;
        Ok(if has(&apps.blocked, app) {
            AppGate::Blocked
        } else if apps.only_approved && !has(&apps.approved, app) {
            AppGate::NotApproved
        } else {
            AppGate::Allowed
        })
    }

    /// Blocks or allows an app. Allowing one also approves it: the person chose to let it in.
    pub fn set_app_blocked(&self, app: &str, blocked: bool) -> AppResult<()> {
        self.update(APPS_FILE, |apps: &mut Apps| {
            apps.blocked.retain(|b| !b.eq_ignore_ascii_case(app));
            if blocked {
                apps.blocked.push(app.to_string());
            } else if !has(&apps.approved, app) {
                apps.approved.push(app.to_string());
            }
            Ok(())
        })
    }

    /// Adds an app to the approved list. A blocked app stays blocked.
    pub fn approve_app(&self, app: &str) -> AppResult<()> {
        self.update(APPS_FILE, |apps: &mut Apps| {
            if !has(&apps.approved, app) {
                apps.approved.push(app.to_string());
            }
            Ok(())
        })
    }

    /// Turns "only allow apps I've approved" on or off. Turning it on approves every app in the
    /// log that isn't blocked, so the apps in use now keep working. An app that was only ever
    /// refused for not being approved (while the mode was on before) stays waiting.
    pub fn set_only_approved_apps(&self, on: bool) -> AppResult<()> {
        // Read before taking the apps file's lock: the log has its own.
        let activity = if on {
            self.app_activity(usize::MAX)?
        } else {
            Vec::new()
        };
        self.update(APPS_FILE, |apps: &mut Apps| {
            if on && !apps.only_approved {
                for a in activity {
                    if a.detail.as_deref() != Some(NOT_APPROVED)
                        && !has(&apps.blocked, &a.app)
                        && !has(&apps.approved, &a.app)
                    {
                        apps.approved.push(a.app);
                    }
                }
            }
            apps.only_approved = on;
            Ok(())
        })
    }

    /// Every app in the log, blocked or approved, most recently seen first, and the mode.
    pub fn app_summaries(&self) -> AppResult<AppList> {
        let mut by_name: BTreeMap<String, AppSummary> = BTreeMap::new();
        let summary = |name: &str| AppSummary {
            name: name.to_string(),
            blocked: false,
            approved: false,
            queries: 0,
            last_used: None,
        };
        for a in self.app_activity(usize::MAX)? {
            let s = by_name
                .entry(a.app.clone())
                .or_insert_with(|| summary(&a.app));
            s.last_used = s.last_used.max(Some(a.at));
            // Reads (list_*, describe_schema) and hand-offs are logged too; count queries.
            if QUERY_TOOLS.contains(&a.tool.as_str()) {
                s.queries += 1;
            }
        }
        let apps: Apps = self.read(APPS_FILE)?;
        for b in &apps.blocked {
            by_name
                .entry(b.clone())
                .or_insert_with(|| summary(b))
                .blocked = true;
        }
        for a in &apps.approved {
            // Matched without case, like the gate, so "cursor" approves "Cursor".
            match by_name
                .values_mut()
                .find(|s| s.name.eq_ignore_ascii_case(a))
            {
                Some(s) => s.approved = true,
                None => {
                    by_name.insert(
                        a.clone(),
                        AppSummary {
                            approved: true,
                            ..summary(a)
                        },
                    );
                }
            }
        }
        let mut list: Vec<AppSummary> = by_name.into_values().collect();
        list.sort_by(|a, b| b.last_used.cmp(&a.last_used).then(a.name.cmp(&b.name)));
        Ok(AppList {
            only_approved: apps.only_approved,
            apps: list,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn store() -> (Store, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("sayql-apps-{}", uuid::Uuid::new_v4()));
        (Store::new(dir.clone()), dir)
    }

    fn entry(app: &str, at: u64, sql: &str) -> Activity {
        Activity {
            at,
            sql: sql.into(),
            ..Activity::new(app, "run_query")
        }
    }

    #[test]
    fn logs_newest_first_and_survives_a_bad_line() {
        let (s, dir) = store();
        assert!(s.app_activity(10).unwrap().is_empty());
        s.log_app_activity(entry("Claude Code", 1, "SELECT 1"))
            .unwrap();
        std::fs::OpenOptions::new()
            .append(true)
            .open(dir.join(LOG.file))
            .unwrap()
            .write_all(b"not json\n")
            .unwrap();
        s.log_app_activity(entry("Cursor", 2, "SELECT 2")).unwrap();
        let log = s.app_activity(10).unwrap();
        assert_eq!(
            log.iter().map(|a| a.app.as_str()).collect::<Vec<_>>(),
            ["Cursor", "Claude Code"]
        );
        assert_eq!(s.app_activity(1).unwrap().len(), 1);
        s.clear_app_activity().unwrap();
        assert!(s.app_activity(10).unwrap().is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn long_sql_is_cut_and_big_logs_roll_over() {
        let (s, dir) = store();
        s.log_app_activity(entry("A", 1, &"x".repeat(10_000)))
            .unwrap();
        assert!(s.app_activity(1).unwrap()[0].sql.chars().count() <= MAX_SQL + 1);
        std::fs::write(dir.join(LOG.file), vec![b' '; MAX_LOG_BYTES as usize + 1]).unwrap();
        s.log_app_activity(entry("B", 2, "SELECT 1")).unwrap();
        assert!(dir.join(LOG.old).exists());
        assert_eq!(s.app_activity(10).unwrap()[0].app, "B");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn blocking_is_per_app_and_listed() {
        let (s, dir) = store();
        s.log_app_activity(entry("Claude Code", 5, "SELECT 1"))
            .unwrap();
        s.log_app_activity(entry("Claude Code", 9, "SELECT 2"))
            .unwrap();
        s.set_app_blocked("Cursor", true).unwrap();
        assert!(s.is_app_blocked("cursor").unwrap() && !s.is_app_blocked("Claude Code").unwrap());
        let list = s.app_summaries().unwrap();
        assert!(!list.only_approved);
        let apps = list.apps;
        assert_eq!(
            apps[0],
            AppSummary {
                name: "Claude Code".into(),
                blocked: false,
                approved: false,
                queries: 2,
                last_used: Some(9)
            }
        );
        assert_eq!(
            apps[1],
            AppSummary {
                name: "Cursor".into(),
                blocked: true,
                approved: false,
                queries: 0,
                last_used: None
            }
        );
        s.set_app_blocked("Cursor", false).unwrap();
        assert!(!s.is_app_blocked("Cursor").unwrap());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn an_unreadable_block_list_fails_closed_and_stays_put() {
        let (s, dir) = store();
        std::fs::write(dir.join(APPS_FILE), r#"{"blocked": ["Cursor""#).unwrap();
        let s = s.without_repair();
        assert!(s.is_app_blocked("Claude Code").is_err());
        assert!(s.app_gate("Claude Code").is_err(), "never read as allowed");
        assert!(s.set_app_blocked("Cursor", false).is_err());
        assert!(s.approve_app("Cursor").is_err());
        assert!(s.set_only_approved_apps(true).is_err());
        assert_eq!(
            std::fs::read_to_string(dir.join(APPS_FILE)).unwrap(),
            r#"{"blocked": ["Cursor""#,
            "the file is left for the app to repair"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn an_older_apps_file_reads_as_any_app_allowed() {
        let (s, dir) = store();
        assert_eq!(s.app_gate("Cursor").unwrap(), AppGate::Allowed);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(APPS_FILE), r#"{"blocked": ["Cursor"]}"#).unwrap();
        assert_eq!(s.app_gate("Claude Code").unwrap(), AppGate::Allowed);
        assert_eq!(s.app_gate("cursor").unwrap(), AppGate::Blocked);
        assert!(!s.app_summaries().unwrap().only_approved);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn only_approved_apps_round_trips_and_approves_the_apps_in_use() {
        let (s, dir) = store();
        s.log_app_activity(entry("Claude Code", 1, "SELECT 1"))
            .unwrap();
        s.log_app_activity(Activity::new("Cursor", "list_connections"))
            .unwrap();
        s.set_app_blocked("Cursor", true).unwrap();
        s.set_only_approved_apps(true).unwrap();
        // Seen and not blocked: approved. Blocked: still blocked, not approved. New: waits.
        assert_eq!(s.app_gate("claude code").unwrap(), AppGate::Allowed);
        assert_eq!(s.app_gate("Cursor").unwrap(), AppGate::Blocked);
        assert_eq!(s.app_gate("Zed").unwrap(), AppGate::NotApproved);
        let list = s.app_summaries().unwrap();
        assert!(list.only_approved);
        let by = |n: &str| list.apps.iter().find(|a| a.name == n).unwrap().clone();
        assert!(by("Claude Code").approved && !by("Claude Code").blocked);
        assert!(by("Cursor").blocked && !by("Cursor").approved);
        assert_eq!(by("Cursor").queries, 0, "reads aren't counted as queries");
        assert!(by("Cursor").last_used.is_some(), "but the app is listed");

        // The saved file keeps all of it.
        let saved: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join(APPS_FILE)).unwrap()).unwrap();
        assert_eq!(saved["only_approved"], true);
        assert_eq!(saved["approved"], serde_json::json!(["Claude Code"]));

        // Blocked wins over approved.
        s.approve_app("Cursor").unwrap();
        assert_eq!(s.app_gate("Cursor").unwrap(), AppGate::Blocked);
        s.approve_app("Zed").unwrap();
        assert_eq!(s.app_gate("Zed").unwrap(), AppGate::Allowed);
        s.set_app_blocked("Zed", true).unwrap();
        assert_eq!(s.app_gate("Zed").unwrap(), AppGate::Blocked);
        // Allowing a blocked app lets it in, approved.
        s.set_app_blocked("Zed", false).unwrap();
        assert_eq!(s.app_gate("Zed").unwrap(), AppGate::Allowed);

        // Off: anything not blocked is let in; the approved list is kept for next time.
        s.set_only_approved_apps(false).unwrap();
        assert_eq!(s.app_gate("Windsurf").unwrap(), AppGate::Allowed);
        assert_eq!(s.app_gate("Cursor").unwrap(), AppGate::Blocked);
        s.log_app_activity(Activity {
            status: "refused".into(),
            detail: Some(NOT_APPROVED.into()),
            ..Activity::new("Cline", "run_query")
        })
        .unwrap();
        s.set_only_approved_apps(true).unwrap();
        assert_eq!(s.app_gate("Windsurf").unwrap(), AppGate::NotApproved);
        assert_eq!(
            s.app_gate("Cline").unwrap(),
            AppGate::NotApproved,
            "an app that was only ever refused isn't approved by turning the mode on"
        );
        assert_eq!(s.app_gate("Zed").unwrap(), AppGate::Allowed);
        let _ = std::fs::remove_dir_all(dir);
    }
}
