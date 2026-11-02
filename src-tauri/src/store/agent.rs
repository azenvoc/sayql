//! What agent mode may do on each connection, the run limits, and database notes.

use super::{Store, note_values};
use crate::error::{AppError, AppResult};
use crate::schema::Schema;
use crate::schema::anchors::{self, Anchor};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

const ACCESS_FILE: &str = "agent.json";
const NOTES_FILE: &str = "notes.json";
pub const MAX_NOTE: usize = 300;
pub const MAX_NEVER_SEND: usize = 200;

/// What the AI may see of query results on one connection.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Results {
    /// Only how many rows a query returned, and its column names.
    #[default]
    Off,
    /// Per result: row count, and per column its type, nulls, distinct count, min and max.
    Counts,
    /// Up to 20 rows per query, with likely personal data masked.
    Rows,
}

/// What AI apps using SayQL over MCP (Claude Code, Cursor, ...) may see of query results on
/// one connection. Separate from agent mode's setting; counts and summaries until the person
/// chooses otherwise.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AppResults {
    /// Only how many rows a query returned, and its column names.
    Off,
    /// Per result: row count, and per column its type, nulls, distinct count, min and max.
    #[default]
    Counts,
    /// Rows, with likely personal data replaced by tokens.
    Masked,
    /// Rows as they are (never-send columns still withheld), and CSV export.
    All,
}

/// Agent mode's access to one connection, which is off until the person turns it on, and
/// what AI apps may see of it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentAccess {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub results: Results,
    /// What AI apps over MCP may see. Never-send columns apply to them too.
    #[serde(default)]
    pub apps: AppResults,
    /// Pause before each send to the AI, with Send and Skip.
    #[serde(default = "yes")]
    pub ask_before_send: bool,
    /// Columns never sent (`column` or `table.column`), whatever the setting.
    #[serde(default)]
    pub never_send: Vec<String>,
}

fn yes() -> bool {
    true
}

/// Run limits and agent preferences, in Settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSettings {
    #[serde(default = "default_steps")]
    pub max_steps: u32,
    #[serde(default = "default_queries")]
    pub max_queries: u32,
    #[serde(default = "default_query_secs")]
    pub query_seconds: u64,
    #[serde(default = "default_run_secs")]
    pub run_seconds: u64,
    /// Prompt tokens at the full rate plus output tokens, per run.
    #[serde(default = "default_tokens")]
    pub max_tokens: u64,
    /// Start right away instead of showing the plan first.
    #[serde(default)]
    pub skip_plan: bool,
}

fn default_steps() -> u32 {
    12
}
fn default_queries() -> u32 {
    10
}
fn default_query_secs() -> u64 {
    30
}
fn default_run_secs() -> u64 {
    180
}
fn default_tokens() -> u64 {
    400_000
}

impl Default for AgentSettings {
    fn default() -> Self {
        AgentSettings {
            max_steps: default_steps(),
            max_queries: default_queries(),
            query_seconds: default_query_secs(),
            run_seconds: default_run_secs(),
            max_tokens: default_tokens(),
            skip_plan: false,
        }
    }
}

impl AgentSettings {
    /// The limits within sensible bounds.
    pub fn validated(self) -> AppResult<AgentSettings> {
        let check = |ok: bool, m: &str| {
            if ok {
                Ok(())
            } else {
                Err(AppError::User(m.to_string()))
            }
        };
        check(
            (1..=50).contains(&self.max_steps),
            "Steps per run must be between 1 and 50.",
        )?;
        check(
            (1..=50).contains(&self.max_queries),
            "Queries per run must be between 1 and 50.",
        )?;
        check(
            (5..=600).contains(&self.query_seconds),
            "Time per query must be between 5 and 600 seconds.",
        )?;
        check(
            (30..=3600).contains(&self.run_seconds),
            "Time per run must be between 30 seconds and an hour.",
        )?;
        check(
            (10_000..=5_000_000).contains(&self.max_tokens),
            "Tokens per run must be between 10,000 and 5,000,000.",
        )?;
        Ok(self)
    }
}

/// A fact about a database that helps answer questions on it ("owner = memberships.role
/// = 'owner'"). Schema-level only: notes never hold row values.
///
/// Besides its text, a note knows what it's about (its anchors), where it came from, whether
/// something has made it doubtful since, and optionally how to check it against the data.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Note {
    pub id: String,
    pub connection_id: String,
    #[serde(default)]
    pub database: Option<String>,
    pub text: String,
    /// Milliseconds since the Unix epoch.
    #[serde(default)]
    pub updated_at: u64,
    #[serde(default)]
    pub created_at: u64,
    /// Who wrote it: empty for the person, else "Investigate" or the AI app's name.
    #[serde(default)]
    pub source: String,
    /// Why it was believed when saved (files, commits, queries), from a suggestion.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<String>,
    /// The tables and columns it's about, as the schema was when it was saved.
    #[serde(default)]
    pub anchors: Vec<Anchor>,
    /// Set when something suggests it may no longer be true.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suspect: Option<Suspect>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub check: Option<NoteCheck>,
    /// When its check last passed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verified_at: Option<u64>,
    /// Its earlier texts, newest first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub history: Vec<Revision>,
}

/// A new note with just its text, as tests and fixtures write one.
impl From<&str> for Note {
    fn from(text: &str) -> Note {
        Note {
            id: uuid::Uuid::new_v4().to_string(),
            text: text.to_string(),
            ..Default::default()
        }
    }
}

/// Why a note may no longer be true.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Suspect {
    pub reason: String,
    pub since: u64,
}

/// A read-only query that answers whether a note still holds: it returns one row with one
/// column, true or false (1 or 0). Only that answer is kept, never data.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NoteCheck {
    pub sql: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last: Option<CheckRun>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckRun {
    /// `None` when the check couldn't run (`error` says why).
    pub ok: Option<bool>,
    pub at: u64,
    /// Why it couldn't run, for the person: the database's own message (it can quote values).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// The same, as an AI may see it (`safety::redact::safe_error`). Not saved: only the run
    /// that just happened hands it to agent mode.
    #[serde(skip)]
    pub ai_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Revision {
    pub text: String,
    pub at: u64,
    #[serde(default)]
    pub source: String,
}

/// Earlier texts kept per note.
const MAX_HISTORY: usize = 5;
/// Longest check query.
pub const MAX_CHECK: usize = 2000;

impl Store {
    pub fn agent_access(&self, connection_id: &str) -> AppResult<AgentAccess> {
        let all: BTreeMap<String, AgentAccess> = self.read(ACCESS_FILE)?;
        Ok(all.get(connection_id).cloned().unwrap_or_default())
    }

    /// Whether the agent and AI app settings file can be read (it may be missing).
    pub fn agent_access_readable(&self) -> AppResult<()> {
        self.read::<BTreeMap<String, AgentAccess>>(ACCESS_FILE)
            .map(|_| ())
    }

    pub fn save_agent_access(&self, connection_id: &str, access: AgentAccess) -> AppResult<()> {
        let mut all: BTreeMap<String, AgentAccess> = self.read(ACCESS_FILE)?;
        all.insert(connection_id.to_string(), clean_access(access)?);
        self.write(ACCESS_FILE, &all)
    }

    /// Forgets a deleted connection's agent access, notes, suggested notes, audits and audit folder.
    pub fn forget_agent(&self, connection_id: &str) -> AppResult<()> {
        let mut all: BTreeMap<String, AgentAccess> = self.read(ACCESS_FILE)?;
        if all.remove(connection_id).is_some() {
            self.write(ACCESS_FILE, &all)?;
        }
        let mut notes = self.all_notes()?;
        let before = notes.len();
        notes.retain(|n| n.connection_id != connection_id);
        if notes.len() != before {
            self.write(NOTES_FILE, &notes)?;
        }
        self.forget_note_suggestions(connection_id)?;
        self.forget_schema_snapshots(connection_id)?;
        self.forget_audits(connection_id)?;
        self.forget_knowledge_sources(connection_id)?;
        self.forget_audit_folder(connection_id)
    }

    pub fn all_notes(&self) -> AppResult<Vec<Note>> {
        self.read(NOTES_FILE)
    }

    /// The notes for one database (`None`: the connection's default database). A note kept
    /// without a database and one kept under the default database's name are the same place.
    pub fn notes(&self, connection_id: &str, database: Option<&str>) -> AppResult<Vec<Note>> {
        let default = self
            .connection(connection_id)
            .ok()
            .map(|c| c.database.trim().to_string())
            .filter(|d| !d.is_empty());
        let place = |d: Option<&str>| {
            d.map(str::trim)
                .filter(|d| !d.is_empty())
                .map(str::to_string)
                .or_else(|| default.clone())
        };
        let wanted = place(database);
        Ok(self
            .all_notes()?
            .into_iter()
            .filter(|n| n.connection_id == connection_id && place(n.database.as_deref()) == wanted)
            .collect())
    }

    /// The notes about any of `tables` (qualified names), plus those not tied to a table.
    pub fn notes_for(
        &self,
        connection_id: &str,
        database: Option<&str>,
        tables: &[String],
    ) -> AppResult<Vec<Note>> {
        Ok(self
            .notes(connection_id, database)?
            .into_iter()
            .filter(|n| {
                n.anchors.is_empty()
                    || n.anchors
                        .iter()
                        .any(|a| tables.iter().any(|t| t.eq_ignore_ascii_case(&a.table)))
            })
            .collect())
    }

    /// Adds or replaces (by id) a note, after checking it holds no row values.
    pub fn save_note(&self, note: Note) -> AppResult<Note> {
        self.save_note_with(note, None)
    }

    /// Adds or replaces (by id) a note, after checking it holds no row values. With the
    /// database's schema, its anchors are found again from its text.
    ///
    /// Only the text and the check come from `note` when it replaces one: where it came from,
    /// its history and its check results stay. A new text is kept in the history, and editing
    /// a note or its check counts as reviewing it, so it's no longer doubtful.
    pub fn save_note_with(&self, note: Note, schema: Option<&Schema>) -> AppResult<Note> {
        let text = clean_note(&note.text)?;
        let check = match note.check {
            Some(c) if !c.sql.trim().is_empty() => Some(NoteCheck {
                sql: clean_check(&c.sql)?,
                last: c.last,
            }),
            _ => None,
        };
        let now = now_ms();
        let mut all = self.all_notes()?;
        let existing = all.iter().find(|n| n.id == note.id).cloned();
        let mut saved = match existing {
            Some(old) => {
                let mut n = Note {
                    database: note.database.clone().or(old.database.clone()),
                    ..old.clone()
                };
                if old.text != text {
                    n.history.insert(
                        0,
                        Revision {
                            text: old.text.clone(),
                            at: old.updated_at,
                            source: old.source.clone(),
                        },
                    );
                    n.history.truncate(MAX_HISTORY);
                    n.text = text;
                    n.source = note.source.clone();
                    if note.evidence.is_some() {
                        n.evidence = note.evidence.clone();
                    }
                    n.suspect = None;
                }
                let same_check = old.check.as_ref().map(|c| c.sql.trim())
                    == check.as_ref().map(|c| c.sql.trim());
                if !same_check {
                    n.check = check.map(|c| NoteCheck { last: None, ..c });
                    n.verified_at = None;
                    n.suspect = None;
                }
                n
            }
            None => Note {
                created_at: if note.created_at > 0 {
                    note.created_at
                } else {
                    now
                },
                text,
                check: check.map(|c| NoteCheck { last: None, ..c }),
                suspect: None,
                verified_at: None,
                ..note
            },
        };
        saved.updated_at = now;
        if let Some(schema) = schema {
            saved.anchors = anchors::resolve(&saved.text, schema).anchors;
        }
        match all.iter_mut().find(|n| n.id == saved.id) {
            Some(slot) => *slot = saved.clone(),
            None => all.push(saved.clone()),
        }
        self.write(NOTES_FILE, &all)?;
        Ok(saved)
    }

    /// Applies `change` to the saved note `id`, if it's still there, and returns it.
    pub fn update_note(&self, id: &str, change: impl FnOnce(&mut Note)) -> AppResult<Option<Note>> {
        let mut all = self.all_notes()?;
        let Some(n) = all.iter_mut().find(|n| n.id == id) else {
            return Ok(None);
        };
        let before = n.clone();
        change(n);
        let out = n.clone();
        if out != before {
            self.write(NOTES_FILE, &all)?;
        }
        Ok(Some(out))
    }

    /// Marks a note doubtful, keeping the first reason until it's reviewed.
    pub fn mark_suspect(&self, id: &str, reason: &str) -> AppResult<Option<Note>> {
        self.update_note(id, |n| {
            if n.suspect.is_none() {
                n.suspect = Some(Suspect {
                    reason: reason.to_string(),
                    since: now_ms(),
                });
            }
        })
    }

    /// Records a check's result: passing verifies the note (and clears a doubt the check
    /// can answer); failing makes it doubtful.
    pub fn record_check(&self, id: &str, run: CheckRun) -> AppResult<Option<Note>> {
        self.update_note(id, |n| {
            let Some(check) = n.check.as_mut() else {
                return;
            };
            check.last = Some(run.clone());
            match run.ok {
                Some(true) => {
                    n.verified_at = Some(run.at);
                    if n.suspect
                        .as_ref()
                        .is_some_and(|s| s.reason.starts_with(CHECK_FAILED))
                    {
                        n.suspect = None;
                    }
                }
                Some(false) if n.suspect.is_none() => {
                    n.suspect = Some(Suspect {
                        reason: format!("{CHECK_FAILED} on {}", day(run.at)),
                        since: run.at,
                    });
                }
                _ => {}
            }
        })
    }

    /// The notes as text to add after a schema, or nothing when there are none.
    pub fn notes_text(&self, connection_id: &str, database: Option<&str>) -> String {
        render_notes(
            &self.notes(connection_id, database).unwrap_or_default(),
            false,
        )
    }

    /// The notes as AI apps over MCP read them after a schema: each with its short id
    /// ([`note_ref`]), so an app can suggest replacing or removing one. Nothing when there are none.
    pub fn notes_text_with_refs(&self, connection_id: &str, database: Option<&str>) -> String {
        render_notes(
            &self.notes(connection_id, database).unwrap_or_default(),
            true,
        )
    }

    /// The note of one database that `reference` names: its short id from
    /// [`Self::notes_text_with_refs`] (`n_1a2b3c4d`, brackets optional) or its full id.
    pub fn find_note(
        &self,
        connection_id: &str,
        database: Option<&str>,
        reference: &str,
    ) -> AppResult<Note> {
        let wanted = reference
            .trim()
            .trim_matches(|c| c == '[' || c == ']')
            .to_lowercase();
        let mut found: Vec<Note> = self
            .notes(connection_id, database)?
            .into_iter()
            .filter(|n| n.id.to_lowercase() == wanted || note_ref(&n.id) == wanted)
            .collect();
        if found.len() > 1 {
            return Err("That id matches more than one note; pass the note's full id.".into());
        }
        found.pop().ok_or_else(|| {
            format!(
                "No saved note on this database has the id {:?}. Use an id such as n_1a2b3c4d from describe_schema's notes.",
                reference.trim()
            )
            .into()
        })
    }

    pub fn delete_note(&self, id: &str) -> AppResult<()> {
        let mut all = self.all_notes()?;
        all.retain(|n| n.id != id);
        self.write(NOTES_FILE, &all)
    }
}

/// A note's short id as AI apps see it: `n_` and the first 8 letters or digits of its id.
pub fn note_ref(id: &str) -> String {
    let short: String = id
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .take(8)
        .collect::<String>()
        .to_lowercase();
    format!("n_{short}")
}

/// How a failed check's doubt starts, so a later pass can clear it.
pub const CHECK_FAILED: &str = "its check failed";

/// The notes as the AI reads them after a schema, each marked by how sure we are of it, with
/// the rule for when a note and the schema disagree. With `refs`, each starts with its short
/// id ([`note_ref`]) for suggest_note's `replaces`. Nothing when there are none.
pub fn render_notes(notes: &[Note], refs: bool) -> String {
    if notes.is_empty() {
        return String::new();
    }
    let now = now_ms();
    let mut ordered: Vec<&Note> = notes.iter().collect();
    // Doubtful notes last.
    ordered.sort_by_key(|n| n.suspect.is_some());
    let lines: Vec<String> = ordered
        .iter()
        .map(|n| {
            let id = if refs {
                format!("[{}] ", note_ref(&n.id))
            } else {
                String::new()
            };
            let mark = match (&n.suspect, n.verified_at) {
                (Some(s), _) => format!("(may be out of date: {}) ", s.reason),
                (None, Some(at)) => format!("(checked {}) ", ago(now, at)),
                (None, None) => String::new(),
            };
            format!("- {id}{mark}{}", n.text)
        })
        .collect();
    let refs_hint = if refs {
        " The [n_…] id is for suggest_note's `replaces`."
    } else {
        ""
    };
    format!(
        "\n\nNotes saved for this database (facts learned earlier). The schema above is current: if a note disagrees with it, follow the schema. Check a note marked \"may be out of date\" before relying on it.{refs_hint}\n{}",
        lines.join("\n")
    )
}

/// "today", "yesterday" or "N days ago".
fn ago(now: u64, at: u64) -> String {
    match now.saturating_sub(at) / 86_400_000 {
        0 => "today".into(),
        1 => "yesterday".into(),
        d => format!("{d} days ago"),
    }
}

/// A date as "2026-10-30".
fn day(ms: u64) -> String {
    chrono::DateTime::from_timestamp_millis(ms as i64)
        .map(|d| d.format("%Y-%m-%d").to_string())
        .unwrap_or_default()
}

/// The check query tidied up, or why it can't be kept. Whether it reads only is decided when
/// it runs, by the same rules as every query the AI runs.
pub fn clean_check(sql: &str) -> AppResult<String> {
    let sql = sql.trim().trim_end_matches(';').trim().to_string();
    if sql.chars().count() > MAX_CHECK {
        return Err(format!("Keep a check to {MAX_CHECK} characters.").into());
    }
    if let Some(what) = note_values::note_value(&sql) {
        return Err(format!(
            "A check can't hold values from the data, and this one holds {what}. Compare with codes and names from the schema instead."
        )
        .into());
    }
    Ok(sql)
}

pub(crate) fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn clean_access(a: AgentAccess) -> AppResult<AgentAccess> {
    let mut never_send: Vec<String> = a
        .never_send
        .iter()
        .map(|c| c.trim().to_string())
        .filter(|c| !c.is_empty())
        .collect();
    never_send.sort_by_key(|c| c.to_lowercase());
    never_send.dedup_by_key(|c| c.to_lowercase());
    if never_send.len() > MAX_NEVER_SEND {
        return Err(format!("Keep the never-send list to {MAX_NEVER_SEND} columns.").into());
    }
    Ok(AgentAccess { never_send, ..a })
}

/// The note tidied up, or why it can't be saved. A note that looks like it holds a value
/// from the data is refused, by the same check for every note: saved by the person,
/// suggested by agent mode, or suggested by an AI app (see [`super::note_values`]).
pub fn clean_note(text: &str) -> AppResult<String> {
    let text = text.trim().to_string();
    if text.is_empty() {
        return Err("Write the note.".into());
    }
    if text.chars().count() > MAX_NOTE {
        return Err(format!("Keep a note to {MAX_NOTE} characters.").into());
    }
    if let Some(what) = note_values::note_value(&text) {
        return Err(format!(
            "A note describes the database, not the data in it, and this one holds {what}. {}; {}.",
            note_values::ALLOWED,
            note_values::REFUSED
        )
        .into());
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (std::path::PathBuf, Store) {
        let dir = std::env::temp_dir().join(format!("sayql-agent-store-{}", uuid::Uuid::new_v4()));
        (dir.clone(), Store::new(dir))
    }

    #[test]
    fn access_is_off_until_turned_on() {
        let (dir, s) = store();
        let a = s.agent_access("c1").unwrap();
        assert_eq!(
            (a.enabled, a.results, a.ask_before_send),
            (false, Results::Off, false)
        );
        let saved: AgentAccess =
            serde_json::from_str(r#"{"enabled":true,"results":"counts"}"#).unwrap();
        assert!(
            saved.ask_before_send,
            "Ask before send defaults to on once saved"
        );
        s.save_agent_access(
            "c1",
            AgentAccess {
                never_send: vec![" email ".into(), "EMAIL".into(), "".into()],
                ..saved
            },
        )
        .unwrap();
        assert_eq!(s.agent_access("c1").unwrap().never_send, ["email"]);
        assert!(!s.agent_access("c2").unwrap().enabled);
        s.forget_agent("c1").unwrap();
        assert!(!s.agent_access("c1").unwrap().enabled);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn notes_hold_no_values() {
        let (dir, s) = store();
        let note = |text: &str| Note {
            id: "n1".into(),
            connection_id: "c1".into(),
            database: None,
            text: text.into(),
            updated_at: 0,
            ..Default::default()
        };
        s.save_note(note("owner = memberships.role = 'owner'"))
            .unwrap();
        s.save_note(note("status 3 = cancelled (appointment_statuses)"))
            .unwrap();
        assert_eq!(s.notes("c1", None).unwrap().len(), 1, "same id replaces");
        assert!(s.notes("c1", Some("other")).unwrap().is_empty());
        for bad in [
            "owner is dana@example.com",
            "call +1 (555) 014-2233",
            "owner is ‹name-1›",
            " ",
        ] {
            assert!(s.save_note(note(bad)).is_err(), "{bad}");
        }
        s.delete_note("n1").unwrap();
        assert!(s.all_notes().unwrap().is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn limits_are_checked() {
        assert!(AgentSettings::default().validated().is_ok());
        assert!(
            AgentSettings {
                max_steps: 0,
                ..Default::default()
            }
            .validated()
            .is_err()
        );
        assert!(
            AgentSettings {
                query_seconds: 9999,
                ..Default::default()
            }
            .validated()
            .is_err()
        );
    }
}
