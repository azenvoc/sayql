//! Saved connections and settings, as JSON files in the app config directory.
//! Secrets never go in these files; see [`secrets`].

pub mod agent;
pub mod apps;
pub mod audit_folders;
pub mod audits;
pub mod code_sources;
pub mod digest;
mod jsonl;
pub mod my_activity;
pub mod note_values;
pub mod schema_snapshots;
pub mod secrets;
pub mod skills;
pub mod suggestions;

use crate::db::ConnConfig;
use crate::error::{AppError, AppResult};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const CONNECTIONS_FILE: &str = "connections.json";
const SETTINGS_FILE: &str = "settings.json";
const SCRIPTS_FILE: &str = "scripts.json";
const SKILLS_FILE: &str = "skills.json";
/// The config folder name used before the app was renamed from AskDB.
const LEGACY_DIR_NAME: &str = "com.askdb.app";

/// Copies saved data from the pre-rename folder next to `dir`, the first time only (when
/// `dir` has no connections yet). The old files are left in place. Only the app's own folder
/// takes them, not a test build's.
fn copy_legacy_files(dir: &std::path::Path) {
    let legacy = dir.with_file_name(LEGACY_DIR_NAME);
    if dir.file_name().is_none_or(|n| n != "com.sayql.app")
        || dir.join(CONNECTIONS_FILE).exists()
        || !legacy.join(CONNECTIONS_FILE).exists()
    {
        return;
    }
    for file in [CONNECTIONS_FILE, SETTINGS_FILE, SCRIPTS_FILE] {
        let from = legacy.join(file);
        if from.exists() {
            let _ = std::fs::copy(&from, dir.join(file));
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    /// The AI provider that writes SQL (an id from `ai::PROVIDERS`).
    #[serde(default = "default_provider")]
    pub provider: String,
    /// Claude model that writes SQL while the person types.
    #[serde(default = "default_typing_model")]
    pub typing_model: String,
    /// Claude model that fixes rejected queries and improves queries on request.
    #[serde(default = "default_expert_model")]
    pub expert_model: String,
    /// Choices for providers other than Claude, by provider id.
    #[serde(default)]
    pub providers: BTreeMap<String, ProviderSettings>,
    /// The demo connection is added once; deleting it keeps it deleted.
    #[serde(default)]
    pub demo_added: bool,
    /// Needed for API keys that aren't scoped to a workspace (`anthropic-workspace-id` header).
    #[serde(default)]
    pub workspace_id: String,
    /// The first-run setup was finished or dismissed, so it isn't shown again.
    #[serde(default)]
    pub onboarded: bool,
    /// Claude model for agent mode; empty means the expert model.
    #[serde(default)]
    pub agent_model: String,
    #[serde(default)]
    pub agent: agent::AgentSettings,
    /// How tools reached each model the last time it ran the agent (`provider/model` →
    /// native or text), so a model without tool calling isn't asked again.
    #[serde(default)]
    pub tool_modes: BTreeMap<String, crate::ai::agent::model::ToolMode>,
    /// The SayQL Team server signed in to, if any; the token is in the keychain.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub team: Option<crate::team::TeamAccount>,
}

/// How SayQL uses one AI provider.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderSettings {
    /// Empty means the provider's usual address.
    #[serde(default)]
    pub base_url: String,
    #[serde(default)]
    pub typing_model: String,
    /// Empty means the typing model does fixes and improvements too.
    #[serde(default)]
    pub expert_model: String,
    /// Empty means the expert model runs agent mode.
    #[serde(default)]
    pub agent_model: String,
}

impl ProviderSettings {
    pub fn model(&self, expert: bool) -> &str {
        if expert && !self.expert_model.is_empty() {
            &self.expert_model
        } else {
            &self.typing_model
        }
    }

    pub fn agent(&self) -> &str {
        if self.agent_model.is_empty() {
            self.model(true)
        } else {
            &self.agent_model
        }
    }
}

impl Settings {
    /// Claude's choices live in the top-level fields, which predate other providers.
    pub fn provider_settings(&self, id: &str) -> ProviderSettings {
        if id == crate::ai::ANTHROPIC {
            ProviderSettings {
                base_url: String::new(),
                typing_model: self.typing_model.clone(),
                expert_model: self.expert_model.clone(),
                agent_model: self.agent_model.clone(),
            }
        } else {
            let mut p = self.providers.get(id).cloned().unwrap_or_default();
            // SayQL AI credits and the Claude plan serve Claude, so they start on its default models.
            if id == crate::ai::SAYQL_CREDITS || id == crate::ai::CLAUDE_PLAN {
                if p.typing_model.is_empty() {
                    p.typing_model = default_typing_model();
                }
                if p.expert_model.is_empty() {
                    p.expert_model = default_expert_model();
                }
            }
            p
        }
    }

    pub fn set_provider_settings(&mut self, id: &str, p: ProviderSettings) {
        if id == crate::ai::ANTHROPIC {
            self.typing_model = p.typing_model;
            self.expert_model = p.expert_model;
            self.agent_model = p.agent_model;
        } else {
            self.providers.insert(id.to_string(), p);
        }
    }
}

fn default_provider() -> String {
    crate::ai::ANTHROPIC.to_string()
}

fn default_typing_model() -> String {
    crate::ai::DEFAULT_TYPING_MODEL.to_string()
}

fn default_expert_model() -> String {
    crate::ai::DEFAULT_EXPERT_MODEL.to_string()
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            provider: default_provider(),
            typing_model: default_typing_model(),
            expert_model: default_expert_model(),
            providers: BTreeMap::new(),
            demo_added: false,
            workspace_id: String::new(),
            onboarded: false,
            agent_model: String::new(),
            agent: agent::AgentSettings::default(),
            tool_modes: BTreeMap::new(),
            team: None,
        }
    }
}

/// A query the person saved from a tab, bound to the database it was written for.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Script {
    pub id: String,
    pub name: String,
    pub connection_id: String,
    #[serde(default)]
    pub database: Option<String>,
    #[serde(default)]
    pub question: String,
    pub sql: String,
    /// Milliseconds since the Unix epoch.
    #[serde(default)]
    pub updated_at: u64,
}

/// Saved instructions the person adds to a question with `/slug`. Text only: a skill
/// holds no SQL and runs nothing; it is added to the AI request for that question.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Skill {
    pub slug: String,
    #[serde(default)]
    pub description: String,
    pub instruction: String,
}

#[derive(Clone)]
pub struct Store {
    dir: PathBuf,
    /// Move unreadable files aside (the app). The MCP server reads the app's files but never
    /// renames or replaces them: an unreadable file is only an error there.
    repair: bool,
}

impl Store {
    pub fn new(dir: PathBuf) -> Self {
        let _ = std::fs::create_dir_all(&dir);
        copy_legacy_files(&dir);
        Store { dir, repair: true }
    }

    /// A store that never moves or replaces an unreadable file (for the MCP server, which
    /// runs beside the app and must leave its files to it).
    pub fn without_repair(self) -> Self {
        Store {
            repair: false,
            ..self
        }
    }

    /// A missing file reads as the default. A file that doesn't parse is moved aside (see
    /// [`Self::set_aside`]) and is an error: reading it as empty would let the next save
    /// erase everything in it.
    fn read<T: for<'de> Deserialize<'de> + Default>(&self, name: &str) -> AppResult<T> {
        let text = match std::fs::read_to_string(self.dir.join(name)) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(T::default()),
            Err(e) => return Err(e.into()),
        };
        serde_json::from_str(&text).map_err(|_| self.unreadable(name))
    }

    /// The error for a file that doesn't parse: moved aside in the app, left alone elsewhere.
    fn unreadable(&self, name: &str) -> AppError {
        if self.repair {
            self.set_aside(name)
        } else {
            AppError::User(format!(
                "SayQL's saved file {name} couldn't be read. Open SayQL to fix it."
            ))
        }
    }

    /// Renames an unreadable file to `<name>.corrupt-<timestamp>` so nothing overwrites it,
    /// and returns the error that tells the person where it went.
    fn set_aside(&self, name: &str) -> AppError {
        let path = self.dir.join(name);
        let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
        let mut backup = self.dir.join(format!("{name}.corrupt-{stamp}"));
        let mut n = 1;
        while backup.exists() {
            n += 1;
            backup = self.dir.join(format!("{name}.corrupt-{stamp}-{n}"));
        }
        match std::fs::rename(&path, &backup) {
            Ok(()) => AppError::User(format!(
                "SayQL's saved file {name} was unreadable, so it was moved to {} and left untouched. Restart SayQL to continue without it, or fix the file and move it back.",
                backup.display()
            )),
            Err(e) => AppError::User(format!(
                "SayQL's saved file {} is unreadable and couldn't be moved aside ({e}). SayQL won't save over it; fix or move the file, then restart SayQL.",
                path.display()
            )),
        }
    }

    /// Writes to a temporary file in the same folder, flushes it to disk, then renames it
    /// over the old file, so a crash leaves either the old file or the new one. Never
    /// replaces a file that doesn't parse.
    fn write<T: Serialize>(&self, name: &str, value: &T) -> AppResult<()> {
        let path = self.dir.join(name);
        if let Ok(text) = std::fs::read_to_string(&path)
            && serde_json::from_str::<serde_json::Value>(&text).is_err()
        {
            return Err(self.unreadable(name));
        }
        let tmp = self
            .dir
            .join(format!(".{name}.{}.tmp", uuid::Uuid::new_v4()));
        let result = (|| -> AppResult<()> {
            use std::io::Write;
            let mut file = std::fs::File::create(&tmp)?;
            file.write_all(serde_json::to_string_pretty(value)?.as_bytes())?;
            file.sync_all()?;
            std::fs::rename(&tmp, &path)?;
            #[cfg(unix)]
            if let Ok(dir) = std::fs::File::open(&self.dir) {
                let _ = dir.sync_all();
            }
            Ok(())
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
        result
    }

    /// The app's config folder.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn connections(&self) -> AppResult<Vec<ConnConfig>> {
        self.read(CONNECTIONS_FILE)
    }

    pub fn connection(&self, id: &str) -> AppResult<ConnConfig> {
        self.connections()?
            .into_iter()
            .find(|c| c.id == id)
            .ok_or_else(|| "Connection not found.".into())
    }

    pub fn save_connections(&self, list: &[ConnConfig]) -> AppResult<()> {
        self.write(CONNECTIONS_FILE, &list)
    }

    pub fn settings(&self) -> AppResult<Settings> {
        self.read(SETTINGS_FILE)
    }

    pub fn save_settings(&self, s: &Settings) -> AppResult<()> {
        self.write(SETTINGS_FILE, s)
    }

    pub fn scripts(&self) -> AppResult<Vec<Script>> {
        self.read(SCRIPTS_FILE)
    }

    pub fn save_scripts(&self, list: &[Script]) -> AppResult<()> {
        self.write(SCRIPTS_FILE, &list)
    }

    pub fn skills(&self) -> AppResult<Vec<Skill>> {
        self.read(SKILLS_FILE)
    }

    pub fn save_skills(&self, list: &[Skill]) -> AppResult<()> {
        self.write(SKILLS_FILE, &list)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_saved_before_other_providers_still_read_as_claude() {
        let s: Settings = serde_json::from_str(
            r#"{"typingModel":"claude-sonnet-5-5","expertModel":"claude-opus-5-5"}"#,
        )
        .unwrap();
        assert_eq!(s.provider, "anthropic");
        assert_eq!(
            s.provider_settings("anthropic").model(false),
            "claude-sonnet-5-5"
        );
        assert_eq!(s.provider_settings("ollama"), ProviderSettings::default());
    }

    #[test]
    fn an_empty_expert_model_falls_back_to_the_typing_model() {
        let mut s = Settings::default();
        s.set_provider_settings(
            "ollama",
            ProviderSettings {
                typing_model: "qwen3".into(),
                ..Default::default()
            },
        );
        let p = s.provider_settings("ollama");
        assert_eq!((p.model(false), p.model(true)), ("qwen3", "qwen3"));
        assert_eq!(
            s.typing_model,
            crate::ai::DEFAULT_TYPING_MODEL,
            "Claude's choice is untouched"
        );
    }

    #[test]
    fn copies_pre_rename_data_once_without_overwriting() {
        let base = std::env::temp_dir().join(format!("sayql-store-{}", uuid::Uuid::new_v4()));
        let legacy = base.join(LEGACY_DIR_NAME);
        std::fs::create_dir_all(&legacy).unwrap();
        std::fs::write(
            legacy.join(CONNECTIONS_FILE),
            r#"[{"id":"x","name":"Sales","kind":"mssql"}]"#,
        )
        .unwrap();
        std::fs::write(legacy.join(SCRIPTS_FILE), "[]").unwrap();

        let store = Store::new(base.join("com.sayql.app"));
        assert_eq!(store.connections().unwrap()[0].name, "Sales");
        assert!(legacy.join(CONNECTIONS_FILE).exists(), "the old file stays");

        // Later starts never overwrite the new folder.
        std::fs::write(legacy.join(CONNECTIONS_FILE), "[]").unwrap();
        let again = Store::new(base.join("com.sayql.app"));
        assert_eq!(again.connections().unwrap().len(), 1);
        let _ = std::fs::remove_dir_all(base);
    }

    fn temp_store() -> (PathBuf, Store) {
        let dir = std::env::temp_dir()
            .join(format!("sayql-store-{}", uuid::Uuid::new_v4()))
            .join("com.sayql.app");
        (dir.clone(), Store::new(dir))
    }

    fn files_in(dir: &std::path::Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn a_missing_file_reads_as_the_default() {
        let (dir, store) = temp_store();
        assert!(store.connections().unwrap().is_empty());
        assert!(!store.settings().unwrap().demo_added);
        let _ = std::fs::remove_dir_all(dir.parent().unwrap());
    }

    #[test]
    fn a_corrupt_file_is_moved_aside_never_overwritten() {
        let (dir, store) = temp_store();
        let bad = r#"[{"id":"x","name":"Sales","kind":"mssql"#;
        std::fs::write(dir.join(CONNECTIONS_FILE), bad).unwrap();

        let err = store.connections().unwrap_err().to_string();
        let backups: Vec<String> = files_in(&dir)
            .into_iter()
            .filter(|n| n.starts_with("connections.json.corrupt-"))
            .collect();
        assert_eq!(backups.len(), 1);
        assert!(
            err.contains(&backups[0]),
            "the error says where the backup is: {err}"
        );
        assert_eq!(
            std::fs::read_to_string(dir.join(&backups[0])).unwrap(),
            bad,
            "the backup is the original bytes"
        );
        assert!(!dir.join(CONNECTIONS_FILE).exists());

        // Saving now starts a fresh file; the backup is untouched.
        store.save_connections(&[]).unwrap();
        assert_eq!(std::fs::read_to_string(dir.join(&backups[0])).unwrap(), bad);
        let _ = std::fs::remove_dir_all(dir.parent().unwrap());
    }

    #[test]
    fn saving_refuses_to_replace_a_corrupt_file() {
        let (dir, store) = temp_store();
        std::fs::write(dir.join(SETTINGS_FILE), "{not json").unwrap();

        let err = store
            .save_settings(&Settings::default())
            .unwrap_err()
            .to_string();
        assert!(err.contains("settings.json.corrupt-"), "{err}");
        let backup = files_in(&dir)
            .into_iter()
            .find(|n| n.starts_with("settings.json.corrupt-"))
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.join(backup)).unwrap(),
            "{not json"
        );
        assert!(
            !dir.join(SETTINGS_FILE).exists(),
            "nothing was written over it"
        );
        let _ = std::fs::remove_dir_all(dir.parent().unwrap());
    }

    #[test]
    fn writes_replace_the_file_whole_and_leave_no_temp_files() {
        let (dir, store) = temp_store();
        let skill = |slug: &str| Skill {
            slug: slug.into(),
            description: String::new(),
            instruction: "x".into(),
        };
        store.save_skills(&[skill("a"), skill("b")]).unwrap();
        store.save_skills(&[skill("c")]).unwrap();

        assert_eq!(store.skills().unwrap(), vec![skill("c")]);
        assert_eq!(files_in(&dir), vec![SKILLS_FILE.to_string()]);
        let _ = std::fs::remove_dir_all(dir.parent().unwrap());
    }
}
