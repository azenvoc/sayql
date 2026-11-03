//! Database notes AI apps suggest over MCP (`suggest_note`). A suggestion waits here until the
//! person accepts it, which saves it as an ordinary note, or dismisses it. A suggestion can
//! also replace a saved note that looks wrong or outdated (accepting it changes that note's
//! text) or remove one (accepting it deletes the note; the text says why). Like notes, a
//! suggestion is schema-level only: its text and evidence are refused when they look like
//! values from the data.

use super::Store;
use super::agent::{Note, NoteCheck, clean_check, clean_note};
use super::note_values::{ALLOWED, REFUSED, note_value};
use crate::error::AppResult;
use serde::{Deserialize, Serialize};

const FILE: &str = "note-suggestions.json";
/// Longest evidence an AI app may give.
pub const MAX_EVIDENCE: usize = 600;
/// Most suggestions waiting per connection.
pub const MAX_PENDING: usize = 50;

/// A note an AI app suggested, waiting for the person.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NoteSuggestion {
    pub id: String,
    pub connection_id: String,
    #[serde(default)]
    pub database: Option<String>,
    /// The note as it would be saved.
    pub text: String,
    /// Why the app thinks it's true (what it checked).
    #[serde(default)]
    pub evidence: String,
    /// The AI app's name, as it gave it when it connected.
    pub app: String,
    /// Milliseconds since the Unix epoch.
    pub created_at: u64,
    /// The id of the saved note this one replaces, or removes when `remove` is set.
    #[serde(default)]
    pub replaces: Option<String>,
    /// That note's text when the suggestion was made, to show what changes.
    #[serde(default)]
    pub replaced_text: Option<String>,
    /// Accepting deletes the note in `replaces`; `text` is why.
    #[serde(default)]
    pub remove: bool,
    /// A read-only query that checks the note (see [`super::agent::NoteCheck`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub check: Option<String>,
}

/// What a suggestion does to the saved notes. `Replace` and `Remove` name a saved note by the
/// id AI apps see in describe_schema (see [`super::agent::note_ref`]) or its full id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change<'a> {
    /// A new note.
    Add,
    /// The note's text becomes the suggestion's.
    Replace(&'a str),
    /// The note is deleted; the suggestion's text says why.
    Remove(&'a str),
}

/// Lowercase, single spaces: two notes that differ only in that are the same note.
fn same(a: &str, b: &str) -> bool {
    let norm = |s: &str| {
        s.split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase()
    };
    norm(a) == norm(b)
}

/// The evidence tidied up, or why it can't be kept.
fn clean_evidence(text: &str) -> AppResult<String> {
    let text = text.trim().to_string();
    if text.chars().count() > MAX_EVIDENCE {
        return Err(format!("Keep the evidence to {MAX_EVIDENCE} characters.").into());
    }
    if let Some(what) = note_value(&text) {
        return Err(format!(
            "The evidence holds {what}. Describe what you checked; {}; {}.",
            ALLOWED.to_lowercase(),
            REFUSED
        )
        .into());
    }
    Ok(text)
}

impl Store {
    /// Every waiting suggestion, oldest first.
    pub fn note_suggestions(&self) -> AppResult<Vec<NoteSuggestion>> {
        self.read(FILE)
    }

    /// Stores a new note for the person to accept or dismiss (see [`Self::suggest_note_change`]).
    pub fn suggest_note(
        &self,
        connection_id: &str,
        database: Option<&str>,
        text: &str,
        evidence: &str,
        app: &str,
    ) -> AppResult<NoteSuggestion> {
        self.suggest_note_change(connection_id, database, text, evidence, app, Change::Add)
    }

    /// Stores a suggestion for the person to accept or dismiss: a new note, or a change to a
    /// saved one. Refuses values from the data, a note already saved or suggested, a note to
    /// change that doesn't exist, and more than [`MAX_PENDING`] per connection.
    pub fn suggest_note_change(
        &self,
        connection_id: &str,
        database: Option<&str>,
        text: &str,
        evidence: &str,
        app: &str,
        change: Change<'_>,
    ) -> AppResult<NoteSuggestion> {
        self.suggest_checked_note_change(connection_id, database, text, evidence, app, change, None)
    }

    /// [`Self::suggest_note_change`] with a query that checks the note.
    #[allow(
        clippy::too_many_arguments,
        reason = "the fields of one suggestion; existing debt, see AGENTS.md"
    )]
    pub fn suggest_checked_note_change(
        &self,
        connection_id: &str,
        database: Option<&str>,
        text: &str,
        evidence: &str,
        app: &str,
        change: Change<'_>,
        check: Option<&str>,
    ) -> AppResult<NoteSuggestion> {
        let text = clean_note(text)?;
        let evidence = clean_evidence(evidence)?;
        let check = match check.map(str::trim).filter(|c| !c.is_empty()) {
            Some(c) => Some(clean_check(c)?),
            None => None,
        };
        let database = database
            .map(str::trim)
            .filter(|d| !d.is_empty())
            .map(str::to_string);
        let remove = matches!(change, Change::Remove(_));
        let target = match change {
            Change::Add => None,
            Change::Replace(r) | Change::Remove(r) => {
                Some(self.find_note(connection_id, database.as_deref(), r)?)
            }
        };
        if !remove
            && self
                .notes(connection_id, database.as_deref())?
                .iter()
                .any(|n| same(&n.text, &text))
        {
            return Err("The user already saved this note; it's in describe_schema.".into());
        }
        let mut all = self.note_suggestions()?;
        let replaces = target.as_ref().map(|n| n.id.clone());
        // One waiting suggestion per note text, and one waiting removal per note.
        if all.iter().any(|s| {
            s.connection_id == connection_id
                && s.database == database
                && s.remove == remove
                && if remove {
                    s.replaces == replaces
                } else {
                    same(&s.text, &text)
                }
        }) {
            return Err("This note was already suggested and is waiting for the user.".into());
        }
        if all
            .iter()
            .filter(|s| s.connection_id == connection_id)
            .count()
            >= MAX_PENDING
        {
            return Err(format!(
                "{MAX_PENDING} suggested notes are already waiting for the user on this connection. Ask them to review them in SayQL first."
            )
            .into());
        }
        let suggestion = NoteSuggestion {
            id: uuid::Uuid::new_v4().to_string(),
            connection_id: connection_id.to_string(),
            database,
            text,
            evidence,
            app: app.to_string(),
            created_at: chrono::Utc::now().timestamp_millis().max(0) as u64,
            replaced_text: target.map(|n| n.text),
            replaces,
            remove,
            check: if remove { None } else { check },
        };
        all.push(suggestion.clone());
        self.write(FILE, &all)?;
        Ok(suggestion)
    }

    /// Applies the suggestion and removes it: saves a new note through the usual checks,
    /// replaces the text of the note it replaces (a new note when that one was deleted since),
    /// or deletes the note it removes. Returns the note saved, or `None` for a removal.
    pub fn accept_note_suggestion(&self, id: &str) -> AppResult<Option<Note>> {
        self.accept_note_suggestion_with(id, None)
    }

    /// [`Self::accept_note_suggestion`], finding the note's anchors in `schema` when given.
    pub fn accept_note_suggestion_with(
        &self,
        id: &str,
        schema: Option<&crate::schema::Schema>,
    ) -> AppResult<Option<Note>> {
        let all = self.note_suggestions()?;
        let s = all
            .iter()
            .find(|s| s.id == id)
            .ok_or("This suggestion is no longer waiting.")?;
        let note = if s.remove {
            if let Some(target) = &s.replaces {
                self.delete_note(target)?;
            }
            None
        } else {
            let existing = match &s.replaces {
                Some(target) => self.all_notes()?.into_iter().find(|n| &n.id == target),
                None => None,
            };
            let check = match (&s.check, &existing) {
                (Some(sql), _) => Some(NoteCheck {
                    sql: sql.clone(),
                    last: None,
                }),
                // A replacement without a check keeps the note's own.
                (None, Some(n)) => n.check.clone(),
                (None, None) => None,
            };
            Some(
                self.save_note_with(
                    Note {
                        id: existing
                            .map(|n| n.id)
                            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
                        connection_id: s.connection_id.clone(),
                        database: s.database.clone(),
                        text: s.text.clone(),
                        source: s.app.clone(),
                        evidence: Some(s.evidence.clone()).filter(|e| !e.is_empty()),
                        check,
                        ..Default::default()
                    },
                    schema,
                )?,
            )
        };
        self.dismiss_note_suggestion(id)?;
        Ok(note)
    }

    pub fn dismiss_note_suggestion(&self, id: &str) -> AppResult<()> {
        let mut all = self.note_suggestions()?;
        let before = all.len();
        all.retain(|s| s.id != id);
        if all.len() != before {
            self.write(FILE, &all)?;
        }
        Ok(())
    }

    /// Forgets a deleted connection's suggestions.
    pub(super) fn forget_note_suggestions(&self, connection_id: &str) -> AppResult<()> {
        let mut all = self.note_suggestions()?;
        let before = all.len();
        all.retain(|s| s.connection_id != connection_id);
        if all.len() != before {
            self.write(FILE, &all)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::agent::note_ref;

    fn store() -> (std::path::PathBuf, Store) {
        let dir = std::env::temp_dir().join(format!("sayql-suggest-{}", uuid::Uuid::new_v4()));
        (dir.clone(), Store::new(dir))
    }

    const EVIDENCE: &str = "no references in api/ since commit a1b2c3 (2026-03-12); pg stats: 0 writes since 2026-01-01";

    #[test]
    fn suggestions_wait_until_accepted() {
        let (dir, s) = store();
        let sug = s
            .suggest_note(
                "c1",
                Some(" "),
                "orders_legacy is unused since March; use orders_v2",
                EVIDENCE,
                "Claude Code",
            )
            .unwrap();
        assert_eq!(
            (sug.database.clone(), sug.app.as_str()),
            (None, "Claude Code")
        );
        assert!(
            s.notes("c1", None).unwrap().is_empty(),
            "not a note until accepted"
        );
        assert!(s.notes_text("c1", None).is_empty());

        let note = s.accept_note_suggestion(&sug.id).unwrap().unwrap();
        assert_eq!(note.text, sug.text);
        assert_eq!(s.notes("c1", None).unwrap(), vec![note]);
        assert!(s.notes_text("c1", None).contains("orders_legacy is unused"));
        assert!(s.note_suggestions().unwrap().is_empty());
        assert!(s.accept_note_suggestion(&sug.id).is_err(), "only once");
        // A saved note can't be suggested again.
        assert!(
            s.suggest_note(
                "c1",
                None,
                "orders_legacy is  UNUSED since March; use orders_v2",
                "",
                "Cursor"
            )
            .unwrap_err()
            .to_string()
            .contains("already saved")
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn suggestions_can_replace_or_remove_a_saved_note() {
        let (dir, s) = store();
        let save = |text: &str| {
            s.save_note(Note {
                id: uuid::Uuid::new_v4().to_string(),
                connection_id: "c1".into(),
                database: Some("shop".into()),
                text: text.into(),
                updated_at: 0,
                ..Default::default()
            })
            .unwrap()
        };
        let old = save("status 3 = cancelled");
        let gone = save("orders_v1 is the live orders table");
        let text = s.notes_text_with_refs("c1", Some("shop"));
        let r = note_ref(&old.id);
        assert!(
            text.contains(&format!("- [{r}] status 3 = cancelled")),
            "{text}"
        );
        assert_eq!(r.len(), 10);
        let change = |text: &str, change: Change<'_>| {
            s.suggest_note_change("c1", Some("shop"), text, "", "Cursor", change)
        };

        // Replace: by the short id from describe_schema, brackets and case aside.
        let upper = format!("[{}]", r.to_uppercase());
        let fix = s
            .suggest_note_change(
                "c1",
                Some("shop"),
                "status 3 = refunded; 4 = cancelled",
                "OrderStatus enum in api/orders.ts (commit a1b2c3)",
                "Claude Code",
                Change::Replace(&upper),
            )
            .unwrap();
        assert_eq!(
            (
                fix.replaces.as_deref(),
                fix.replaced_text.as_deref(),
                fix.remove
            ),
            (Some(old.id.as_str()), Some("status 3 = cancelled"), false)
        );
        // Remove: the text is the reason.
        let drop = change(
            "orders_v1 was dropped from the code; orders_v2 replaced it",
            Change::Remove(&gone.id),
        )
        .unwrap();
        assert!(drop.remove && drop.replaced_text.as_deref() == Some(gone.text.as_str()));
        assert!(
            change("again", Change::Remove(&gone.id))
                .unwrap_err()
                .to_string()
                .contains("already suggested"),
            "one removal per note waits"
        );
        for missing in ["n_00000000", "", "nope"] {
            assert!(
                change("x is y", Change::Replace(missing))
                    .unwrap_err()
                    .to_string()
                    .contains("No saved note"),
                "{missing}"
            );
        }
        assert!(
            s.suggest_note_change(
                "c1",
                Some("other"),
                "x is y",
                "",
                "Cursor",
                Change::Replace(&r)
            )
            .is_err(),
            "only notes of the same database"
        );
        assert!(
            change("owner is dana@example.com", Change::Replace(&r)).is_err(),
            "the usual value checks"
        );

        // Accepting changes that note's text, keeping its id, and deletes the removed one.
        let saved = s.accept_note_suggestion(&fix.id).unwrap().unwrap();
        assert_eq!(saved.id, old.id);
        assert_eq!(s.accept_note_suggestion(&drop.id).unwrap(), None);
        let left = s.notes("c1", Some("shop")).unwrap();
        assert_eq!(
            left.iter().map(|n| n.text.as_str()).collect::<Vec<_>>(),
            ["status 3 = refunded; 4 = cancelled"]
        );
        assert!(s.note_suggestions().unwrap().is_empty());

        // A note deleted after the suggestion: accepting the replacement saves it as new.
        let late = change("status 5 = disputed", Change::Replace(&saved.id)).unwrap();
        s.delete_note(&saved.id).unwrap();
        let note = s.accept_note_suggestion(&late.id).unwrap().unwrap();
        assert_ne!(note.id, saved.id);
        // Dismissing a removal keeps the note.
        let keep = change("unused", Change::Remove(&note.id)).unwrap();
        s.dismiss_note_suggestion(&keep.id).unwrap();
        assert_eq!(s.notes("c1", Some("shop")).unwrap().len(), 1);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// The real audit of 2026-10-28: six schema facts were refused. Now they go through
    /// suggest_note, and the same check saves them as notes; real values from rows don't.
    #[test]
    fn audit_facts_pass_and_row_values_dont_everywhere() {
        use crate::store::note_values::tests::REFUSED_IN_THE_REAL_RUN;
        let (dir, s) = store();
        for (i, (text, evidence)) in REFUSED_IN_THE_REAL_RUN.iter().enumerate() {
            let sug = s
                .suggest_note("c1", None, text, evidence, "Claude Code")
                .unwrap_or_else(|e| panic!("{text}: {e}"));
            assert_eq!(&sug.text, text);
            // The person's own notes and agent mode's suggestions use the same check.
            clean_note(text).unwrap();
            s.save_note(Note {
                id: format!("n{i}"),
                connection_id: "c2".into(),
                database: None,
                text: text.to_string(),
                updated_at: 0,
                ..Default::default()
            })
            .unwrap();
        }
        for value in [
            "owner is dana@example.com",
            "call +1 555 014 2233",
            "test card 4111 1111 1111 1111",
            "tenant 550e8400-e29b-41d4-a716-446655440000",
            "the admin is John Smith",
            "Priya Raman owns orders",
            "customer 1234567 is special",
        ] {
            let e = s
                .suggest_note("c3", None, value, "", "Claude Code")
                .unwrap_err()
                .to_string();
            assert!(e.contains("describes the database"), "{value}: {e}");
            assert!(
                s.suggest_note("c3", None, "orders.vip is a flag", value, "Claude Code")
                    .unwrap_err()
                    .to_string()
                    .contains("The evidence holds"),
                "{value}"
            );
            assert!(clean_note(value).is_err(), "{value}");
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn dismissing_drops_it() {
        let (dir, s) = store();
        let sug = s
            .suggest_note("c1", Some("shop"), "status 3 = cancelled", "", "Cursor")
            .unwrap();
        s.dismiss_note_suggestion(&sug.id).unwrap();
        assert!(s.note_suggestions().unwrap().is_empty());
        assert!(s.notes("c1", Some("shop")).unwrap().is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn refuses_duplicates_values_and_too_many() {
        let (dir, s) = store();
        s.suggest_note("c1", None, "status 3 = cancelled", EVIDENCE, "Cursor")
            .unwrap();
        assert!(
            s.suggest_note("c1", None, "Status 3 = cancelled", EVIDENCE, "Claude Code")
                .unwrap_err()
                .to_string()
                .contains("already suggested")
        );
        s.suggest_note(
            "c1",
            Some("other"),
            "status 3 = cancelled",
            EVIDENCE,
            "Cursor",
        )
        .unwrap();
        for (text, evidence) in [
            ("owner is dana@example.com", ""),
            ("x".repeat(301).as_str(), ""),
            (
                "customers.vip marks paying customers",
                "row for dana@example.com has vip = true",
            ),
            ("customers.vip marks paying customers", &"y".repeat(601)),
            // Found by the governance test (2026-10-28): names and amounts were accepted.
            ("the VIP customer is Alice Smith", ""),
            ("top salary is 130000", ""),
            ("top salary is 130,000", ""),
            (
                "customers.vip marks paying customers",
                "Alice Smith has vip = true",
            ),
        ] {
            assert!(
                s.suggest_note("c1", None, text, evidence, "Cursor")
                    .is_err(),
                "{text} / {evidence}"
            );
        }
        // Schema facts with dates, codes and product names still go through.
        s.suggest_note(
            "c1",
            None,
            "orders_legacy is unused since March 2026; use orders_v2 (status 3 = cancelled in Stripe)",
            EVIDENCE,
            "Cursor",
        )
        .unwrap();
        for i in 3..MAX_PENDING {
            s.suggest_note("c1", None, &format!("table t{i} is unused"), "", "Cursor")
                .unwrap();
        }
        assert!(
            s.suggest_note("c1", None, "one more", "", "Cursor")
                .unwrap_err()
                .to_string()
                .contains("already waiting")
        );
        assert!(
            s.suggest_note("c2", None, "one more", "", "Cursor").is_ok(),
            "the cap is per connection"
        );
        s.forget_agent("c1").unwrap();
        assert!(
            s.note_suggestions()
                .unwrap()
                .iter()
                .all(|x| x.connection_id == "c2")
        );
        let _ = std::fs::remove_dir_all(dir);
    }
}
