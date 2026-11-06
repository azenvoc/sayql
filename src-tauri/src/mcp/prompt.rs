//! The `audit` prompt: the audit instructions for the connection an AI app names, recorded for
//! "Last audit" and logged.

use super::*;

impl SayqlServer {
    /// The audit instructions for an app the gate let in, for the connection `connection`
    /// names (id or name; the only one when there's one and none is named). The audit is
    /// recorded for "Last audit" in Settings → AI apps, and logged.
    pub(super) fn audit_prompt(
        &self,
        app: &str,
        args: Option<&Map<String, Value>>,
    ) -> Result<String, String> {
        let arg = |k: &str| {
            args.and_then(|a| a.get(k))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
        };
        let mut log = Activity::new(app, audit::PROMPT);
        let found = self
            .state
            .store
            .connections()
            .map_err(|e| e.to_string())
            .and_then(|list| {
                let layout = self.layout()?;
                let list: Vec<_> = list
                    .into_iter()
                    .filter(|c| self.in_reach(&layout, &c.id))
                    .collect();
                Ok(match arg("connection") {
                    Some(wanted) => Some(audit::find_connection(&list, &wanted)?.clone()),
                    None if list.len() == 1 => list.into_iter().next(),
                    None => None,
                })
            });
        let conn = match found {
            Ok(c) => c,
            Err(e) => {
                log.status = "error".into();
                log.detail = Some(e.chars().take(160).collect());
                self.write_log(log);
                return Err(e);
            }
        };
        let Some(conn) = conn else {
            // The app picks the connection; nothing to record until it names one.
            log.detail = Some("audit prompt, connection not named".into());
            self.write_log(log);
            return Ok(audit::instructions(None, false));
        };
        if let Some(database) = arg("database")
            && conn.kind != Kind::Sqlite
            && !self.limit_for(&conn.id)?.allows(&database)
        {
            return Err(scope::no_database(&database));
        }
        // A SQLite file is one database: its notes are kept without one.
        let database = if conn.kind == Kind::Sqlite {
            None
        } else {
            self.notes_database(&conn.id, arg("database").as_deref())
        };
        // A record that can't be written doesn't stop the audit.
        let _ = self
            .state
            .store
            .record_audit(&conn.id, database.as_deref(), app);
        log.connection_id = conn.id.clone();
        log.connection = conn.name.clone();
        log.detail = Some("audit prompt".into());
        self.write_log(log);
        Ok(audit::instructions(
            Some(&audit::Target {
                connection_id: &conn.id,
                name: &conn.name,
                database: database.as_deref(),
            }),
            false,
        ))
    }
}

/// The `audit` prompt as prompts/list shows it.
pub(super) fn audit_prompt_info() -> Prompt {
    Prompt::new(
        audit::PROMPT,
        Some(audit::DESCRIPTION),
        Some(vec![
            PromptArgument::new("connection")
                .with_description(
                    "The SayQL connection to audit: its name or id. Optional when only one is saved; otherwise the app picks it with list_connections.",
                )
                .with_required(false),
            PromptArgument::new("database")
                .with_description(
                    "A database on that connection's server. Omit for the connection's default (and for SQLite).",
                )
                .with_required(false),
        ]),
    )
    .with_title("Audit: teach SayQL your database")
}
