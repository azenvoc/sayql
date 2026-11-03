//! `suggest_note`: notes an AI app suggests about a database, kept for the person to accept or
//! dismiss in SayQL, and logged.

use super::*;

impl SayqlServer {
    /// The database notes are kept under: the one named, else the connection's default, as the
    /// app saves them (its tabs name the default database), so both see the same notes.
    pub(super) fn notes_database(
        &self,
        connection_id: &str,
        database: Option<&str>,
    ) -> Option<String> {
        audit::notes_database(&self.state.store, connection_id, database)
    }

    /// Stores a suggested note for the user to accept or dismiss, and logs it.
    pub(super) fn suggest(&self, app: &str, p: SuggestNoteParams) -> Result<String, String> {
        let conn = match self.state.store.connection(&p.connection_id) {
            Ok(c) => c,
            Err(_) => {
                let e = format!(
                    "No saved connection has id {:?}; use one from list_connections.",
                    p.connection_id
                );
                let _ = self.state.store.log_app_activity(Activity {
                    connection_id: p.connection_id.clone(),
                    status: "refused".into(),
                    detail: Some("unknown connection".into()),
                    ..Activity::new(app, "suggest_note")
                });
                return Err(e);
            }
        };
        let mut log = Activity {
            connection_id: conn.id.clone(),
            connection: conn.name.clone(),
            ..Activity::new(app, "suggest_note")
        };
        let remove = p.remove.unwrap_or(false);
        let target = p
            .replaces
            .as_deref()
            .map(str::trim)
            .filter(|r| !r.is_empty());
        let change = match (target, remove) {
            (None, false) => Ok(Change::Add),
            (Some(r), false) => Ok(Change::Replace(r)),
            (Some(r), true) => Ok(Change::Remove(r)),
            (None, true) => Err(
                "To suggest removing a note, set `replaces` to its id from describe_schema's notes.".to_string(),
            ),
        };
        let database = self.notes_database(&conn.id, p.database.as_deref());
        let result = change.and_then(|change| {
            self.state
                .store
                .suggest_checked_note_change(
                    &conn.id,
                    database.as_deref(),
                    &p.text,
                    &p.evidence,
                    app,
                    change,
                    p.check.as_deref(),
                )
                .map(|_| {
                    let what = match change {
                        Change::Add => "the note",
                        Change::Replace(_) => "replacing the note",
                        Change::Remove(_) => "removing the note",
                    };
                    format!(
                        "Suggested {what} for {:?}. It waits in SayQL until the user accepts it; describe_schema \
                         shows the notes as they are until then.",
                        conn.name
                    )
                })
                .map_err(|e| e.to_string())
        });
        // The note itself waits in SayQL for the user; the log says only what was suggested.
        match &result {
            Ok(_) => {
                log.detail = Some(
                    match (target.is_some(), remove) {
                        (false, _) => "suggested a note",
                        (true, false) => "suggested replacing a note",
                        (true, true) => "suggested removing a note",
                    }
                    .into(),
                )
            }
            Err(e) => {
                log.status = "refused".into();
                log.detail = Some(e.chars().take(160).collect());
            }
        }
        let _ = self.state.store.log_app_activity(log);
        result
    }
}
