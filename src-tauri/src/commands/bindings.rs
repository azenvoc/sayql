//! Writes `src/api/bindings.ts`: the TypeScript types of everything the UI and this backend
//! send each other (command arguments and results, event payloads, shared constants) and a
//! typed wrapper per command, all generated from the Rust types with tauri-specta. Nothing here
//! runs in the app: `pnpm gen:types` runs the test below to write the file, and CI runs it to
//! fail when the committed file no longer matches the Rust types.
//!
//! Types are exported per serde phase: `X_Serialize` is the JSON this backend sends (command
//! results, events) and `X_Deserialize` what it accepts (command arguments), since `default`,
//! `skip_serializing_if` and `Option` make the two differ (`X` is either). A type that is the
//! same both ways is exported once, as `X`.

use std::path::Path;

/// The generated bindings, relative to `src-tauri`.
const OUT: &str = "../src/api/bindings.ts";

const HEADER: &str = "// Generated from the Rust types by `pnpm gen:types` (src-tauri/src/commands/bindings.rs).\n// Change the Rust type, then run `pnpm gen:types`; CI fails when this file is stale.";

fn builder() -> tauri_specta::Builder<tauri::Wry> {
    tauri_specta::Builder::<tauri::Wry>::new()
        .commands(with_commands!(tauri_specta::collect_commands))
        // Event payloads: the app emits them with `Emitter::emit` and the names below, and
        // src/api/events.ts subscribes with these types.
        .typ::<crate::ai::agent::run::Event>()
        .typ::<crate::ai::Delta>()
        .typ::<crate::ai::Done>()
        .typ::<crate::ai::Reset>()
        .typ::<crate::audit::run::AuditEvent>()
        .typ::<super::voice::Progress>()
        .constant("AGENT_EVENT", super::agent::EVENT)
        .constant("AUDIT_EVENT", super::audit::EVENT)
        .constant("AI_DELTA_EVENT", crate::ai::EVENT_DELTA)
        .constant("AI_DONE_EVENT", crate::ai::EVENT_DONE)
        .constant("AI_RESET_EVENT", crate::ai::EVENT_RESET)
        // Messages and limits the UI recognises or shows.
        .constant("WRITE_CONFIRM", crate::error::WRITE_CONFIRM)
        .constant("QUERY_CANCELLED", crate::db::CANCELLED)
        .constant("TEAM_SIGNED_OUT", crate::team::SIGNED_OUT)
        .constant(
            "MAX_SOURCES_FREE",
            crate::store::code_sources::MAX_SOURCES_FREE,
        )
        .constant(
            "SOURCE_CAP_MESSAGE",
            crate::store::code_sources::CAP_MESSAGE,
        )
        // The wrappers reject with the command's error, exactly as `invoke` does.
        .error_handling(tauri_specta::ErrorHandlingMode::Throw)
        // u64 and u128 fields are ids, counts, byte sizes and milliseconds: all far below 2^53,
        // and JSON.parse has always turned them into numbers.
        .dangerously_cast_bigints_to_number()
}

/// Writes the bindings to `path`.
fn export(path: &Path) -> Result<(), specta_typescript::Error> {
    builder().export(
        WithCommandArgs(specta_typescript::Typescript::default().header(HEADER)),
        path,
    )
}

/// The TypeScript exporter, plus a `COMMAND_ARGS` constant: each command's argument names as
/// the webview sends them (camelCase, in order). With the generated wrappers' parameter types
/// it gives each command's arguments object, which types the browser preview's mock backend
/// (src/dev/mockBackend.ts) so it can't drift from the commands either.
struct WithCommandArgs(specta_typescript::Typescript);

impl tauri_specta::LanguageExt for WithCommandArgs {
    type Error = specta_typescript::Error;

    fn export(
        self,
        cfg: &tauri_specta::BuilderConfiguration,
        path: &Path,
    ) -> Result<(), Self::Error> {
        let mut cfg = cfg.clone();
        let args = cfg
            .commands
            .iter()
            .map(|f| {
                let names = f.args().iter().map(|(n, _)| camel_case(n)).collect();
                (f.name().to_string(), serde_json::Value::Array(names))
            })
            .collect();
        cfg.constants
            .insert("COMMAND_ARGS".into(), serde_json::Value::Object(args));
        self.0.export(&cfg, path)
    }
}

/// `run_id` → `runId`: how Tauri names a command's arguments in the webview by default.
fn camel_case(snake: &str) -> serde_json::Value {
    let mut out = String::new();
    let mut upper = false;
    for c in snake.chars() {
        match c {
            '_' => upper = true,
            c if upper => {
                out.extend(c.to_uppercase());
                upper = false;
            }
            c => out.push(c),
        }
    }
    serde_json::Value::String(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};
    use std::error::Error;

    type TestResult = Result<(), Box<dyn Error>>;

    fn out() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join(OUT)
    }

    /// `pnpm gen:types`: writes src/api/bindings.ts.
    #[test]
    #[ignore = "writes src/api/bindings.ts; run by `pnpm gen:types`"]
    fn write_typescript_bindings() -> TestResult {
        Ok(export(&out())?)
    }

    #[test]
    fn typescript_bindings_match_the_rust_types() -> TestResult {
        let dir = std::env::temp_dir().join(format!("sayql-bindings-{}", std::process::id()));
        std::fs::create_dir_all(&dir)?;
        let fresh = dir.join("bindings.ts");
        let exported = export(&fresh);
        let want = std::fs::read_to_string(&fresh);
        let _ = std::fs::remove_dir_all(&dir);
        exported?;
        let have = std::fs::read_to_string(out()).unwrap_or_default();
        assert!(
            have == want?,
            "src/api/bindings.ts doesn't match the Rust types. Run `pnpm gen:types` and commit it."
        );
        Ok(())
    }

    /// The fields of `export type {name}_Serialize = { … }` (or `{name}`) in the bindings, each
    /// with whether it may be left out.
    fn ts_fields(bindings: &str, name: &str) -> Vec<(String, bool)> {
        let heads = [
            format!("export type {name}_Serialize = {{\n"),
            format!("export type {name} = {{\n"),
        ];
        let (_, rest) = heads
            .iter()
            .find_map(|head| bindings.split_once(head.as_str()))
            .unwrap_or_else(|| panic!("{name} isn't an object type in the bindings"));
        let (body, _) = rest.split_once("\n};").unwrap();
        body.lines()
            .filter_map(|l| l.strip_prefix('\t'))
            .filter_map(|l| l.split_once(": "))
            .filter(|(k, _)| {
                k.chars()
                    .all(|c| c.is_alphanumeric() || c == '_' || c == '?')
            })
            .map(|(k, _)| match k.strip_suffix('?') {
                Some(k) => (k.to_string(), true),
                None => (k.to_string(), false),
            })
            .collect()
    }

    /// Checks one Rust value's JSON against its generated type: every key it has is a field of
    /// the type, and every field the type requires is there.
    fn same_fields(bindings: &str, name: &str, value: Value) {
        let Value::Object(map) = value else {
            panic!("{name} doesn't serialize to an object")
        };
        let fields = ts_fields(bindings, name);
        assert!(
            !fields.is_empty(),
            "{name}: no fields found in the bindings"
        );
        for key in map.keys() {
            assert!(
                fields.iter().any(|(f, _)| f == key),
                "{name}: the JSON has `{key}`, the TypeScript type doesn't"
            );
        }
        for (field, optional) in &fields {
            assert!(
                *optional || map.contains_key(field),
                "{name}: the TypeScript type requires `{field}`, the JSON doesn't have it"
            );
        }
    }

    /// `json` read the way the backend reads it, then sent the way it sends it.
    fn sent<T: serde::de::DeserializeOwned + serde::Serialize>(json: Value) -> Value {
        serde_json::to_value(serde_json::from_value::<T>(json).unwrap()).unwrap()
    }

    #[test]
    fn generated_types_have_the_fields_the_backend_sends() -> TestResult {
        use crate::{db, schema, store, team};
        let bindings = std::fs::read_to_string(out())?;
        let b = bindings.as_str();

        same_fields(
            b,
            "Column",
            serde_json::to_value(schema::Column::default())?,
        );
        same_fields(b, "Table", serde_json::to_value(schema::Table::default())?);
        same_fields(
            b,
            "QueryResult",
            serde_json::to_value(db::QueryResult {
                columns: vec!["id".into()],
                rows: vec![vec![json!(1)]],
                rows_affected: None,
                truncated: false,
                elapsed_ms: 3,
                withheld: vec![],
            })?,
        );
        same_fields(
            b,
            "ConnConfig",
            sent::<db::ConnConfig>(json!({ "id": "c1", "name": "Shop", "kind": "postgres" })),
        );
        same_fields(
            b,
            "Note",
            serde_json::to_value(store::agent::Note::default())?,
        );
        same_fields(
            b,
            "AgentAccess",
            serde_json::to_value(store::agent::AgentAccess::default())?,
        );
        same_fields(
            b,
            "AgentSettings",
            serde_json::to_value(store::agent::AgentSettings::default())?,
        );
        same_fields(
            b,
            "NoteSuggestion",
            sent::<store::suggestions::NoteSuggestion>(json!({
                "id": "s1", "connectionId": "c1", "text": "t", "app": "Cursor", "createdAt": 1
            })),
        );
        same_fields(
            b,
            "Script",
            sent::<store::Script>(
                json!({ "id": "s1", "name": "n", "connectionId": "c1", "sql": "" }),
            ),
        );
        same_fields(
            b,
            "Activity",
            sent::<store::apps::Activity>(
                json!({ "at": 1, "app": "Cursor", "tool": "run_query", "status": "ok" }),
            ),
        );
        same_fields(
            b,
            "MyActivity",
            sent::<store::my_activity::MyActivity>(
                json!({ "at": 1, "connectionId": "c1", "source": "ask", "sql": "", "status": "ok" }),
            ),
        );
        // Read from the team server in snake_case, sent to the UI in camelCase.
        same_fields(
            b,
            "ServerInfo",
            sent::<team::ServerInfo>(json!({ "name": "s", "user_limit": 10 })),
        );
        same_fields(
            b,
            "AiStatus",
            serde_json::to_value(team::ai::AiStatus::default())?,
        );
        Ok(())
    }
}
