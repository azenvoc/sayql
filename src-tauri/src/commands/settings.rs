use crate::ai::endpoint::{address, endpoint, valid_workspace_id};
use crate::ai::{self, Api, KeyNeed};
use crate::audit;
use crate::error::{AppError, AppResult};
use crate::state::AppState;
use crate::store::agent::AgentSettings;
use crate::store::apps::{Activity, AppList};
use crate::store::audits::AuditRun;
use crate::store::{ProviderSettings, Settings, secrets};
use serde::Serialize;
use tauri::State;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsView {
    /// The provider that writes SQL, and the models it uses.
    provider: String,
    typing_model: String,
    /// Already resolved: the typing model when no separate one was chosen.
    expert_model: String,
    /// The model that runs agent mode, as chosen (empty: the expert model).
    agent_model: String,
    agent: AgentSettings,
    /// The provider has everything it needs (key, model) to write SQL.
    ai_ready: bool,
    /// The first-run setup was finished or dismissed.
    onboarded: bool,
    workspace_id: String,
    /// Claude's models; other providers list theirs on request (`list_models`).
    models: Vec<ModelOption>,
    providers: Vec<ProviderView>,
    /// The SayQL Team server signed in to, if any.
    team: Option<crate::team::TeamAccount>,
}

#[derive(Serialize)]
struct ModelOption {
    id: &'static str,
    label: &'static str,
}

/// One provider as Settings shows it: the catalog entry plus what the person chose for it.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ProviderView {
    id: &'static str,
    label: &'static str,
    note: &'static str,
    api: Api,
    key: KeyNeed,
    key_placeholder: &'static str,
    local: bool,
    default_base_url: &'static str,
    base_url: String,
    typing_model: String,
    expert_model: String,
    agent_model: String,
    has_key: bool,
    /// Everything needed to use it is set (model, key, address), so the Ask box can switch to it.
    ready: bool,
}

/// What the UI may know about the settings: never an API key itself.
pub fn settings_view(s: &Settings) -> SettingsView {
    let active = s.provider_settings(&s.provider);
    SettingsView {
        provider: s.provider.clone(),
        typing_model: active.typing_model.clone(),
        expert_model: active.model(true).to_string(),
        agent_model: active.agent_model.clone(),
        agent: s.agent.clone(),
        ai_ready: endpoint(s).is_ok(),
        onboarded: s.onboarded,
        workspace_id: s.workspace_id.clone(),
        team: s.team.clone(),
        models: ai::MODELS
            .iter()
            .map(|(id, label)| ModelOption { id, label })
            .collect(),
        providers: ai::PROVIDERS
            .iter()
            .map(|p| {
                let chosen = s.provider_settings(p.id);
                ProviderView {
                    id: p.id,
                    label: p.label,
                    note: p.note,
                    api: p.api,
                    key: p.key,
                    key_placeholder: p.key_placeholder,
                    local: p.local,
                    default_base_url: p.base_url,
                    base_url: chosen.base_url,
                    typing_model: chosen.typing_model,
                    expert_model: chosen.expert_model,
                    agent_model: chosen.agent_model,
                    has_key: p.key != KeyNeed::None && secrets::api_key(p.id).is_some(),
                    ready: endpoint(&Settings {
                        provider: p.id.to_string(),
                        ..s.clone()
                    })
                    .is_ok(),
                }
            })
            .collect(),
    }
}

/// Saves the choices for `provider` and makes it the one that writes SQL.
/// `api_key: None` keeps the saved key; an empty `expert_model` reuses the typing model.
#[tauri::command]
pub fn save_settings(
    state: State<'_, AppState>,
    provider: String,
    typing_model: String,
    expert_model: String,
    base_url: String,
    api_key: Option<String>,
    workspace_id: String,
) -> AppResult<SettingsView> {
    let p = ai::endpoint::provider(&provider)?;
    let typing_model = typing_model.trim().to_string();
    let expert_model = expert_model.trim().to_string();
    let base_url = ai::clean_base_url(&base_url).map_err(AppError::User)?;
    match p.api {
        Api::Anthropic | Api::Plan => {
            for model in [&typing_model, &expert_model] {
                if !ai::is_known(model) {
                    return Err(format!("Unknown model {model}").into());
                }
            }
        }
        Api::OpenAi => {
            if typing_model.is_empty() {
                return Err(AppError::User(format!(
                    "Choose the {} model that writes SQL while you type.",
                    p.label
                )));
            }
            if base_url.is_empty() && p.base_url.is_empty() {
                return Err(AppError::User(format!(
                    "Add the base URL of your {} server.",
                    p.label
                )));
            }
        }
    }
    let workspace_id = valid_workspace_id(&workspace_id)?;
    if let Some(key) = api_key.filter(|_| p.key != KeyNeed::None) {
        secrets::set_api_key(p.id, &key)?;
    }
    // Claude's address is fixed; a base URL equal to the default is stored as "use the default".
    let base_url = if p.api != Api::OpenAi || base_url == p.base_url {
        String::new()
    } else {
        base_url
    };
    let s = state.store.update_settings(|s| {
        s.provider = p.id.to_string();
        s.workspace_id = workspace_id;
        let agent_model = s.provider_settings(p.id).agent_model;
        s.set_provider_settings(
            p.id,
            ProviderSettings {
                base_url,
                typing_model,
                expert_model,
                agent_model,
            },
        );
        Ok(s.clone())
    })?;
    Ok(settings_view(&s))
}

/// Which model slot the Ask box's model picker sets.
#[derive(serde::Deserialize, Clone, Copy, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ModelUse {
    /// Quick mode: writes SQL while the person types.
    Typing,
    /// Agent mode.
    Agent,
}

/// Switches to `provider` (one that is ready) and uses `model` for typing or for agent mode,
/// from the Ask box. Other choices for that provider stay as they were.
#[tauri::command]
pub fn choose_model(
    state: State<'_, AppState>,
    provider: String,
    model: String,
    purpose: ModelUse,
) -> AppResult<SettingsView> {
    // Checked before the settings file is locked: the check may read the keychain.
    let provider = choose(&state.store.settings()?, &provider, model.trim(), purpose)?;
    let p = ai::endpoint::provider(&provider)?;
    let s = state.store.update_settings(|s| {
        s.provider = provider;
        let mut chosen = s.provider_settings(p.id);
        match purpose {
            ModelUse::Typing => chosen.typing_model = model.trim().to_string(),
            ModelUse::Agent => chosen.agent_model = model.trim().to_string(),
        }
        s.set_provider_settings(p.id, chosen);
        Ok(s.clone())
    })?;
    Ok(settings_view(&s))
}

/// The provider id to switch to, after checking it's ready and the model makes sense for it.
fn choose(s: &Settings, provider: &str, model: &str, purpose: ModelUse) -> AppResult<String> {
    let p = ai::endpoint::provider(provider)?;
    if model.is_empty() && purpose == ModelUse::Typing {
        return Err("Choose a model.".into());
    }
    if p.api != Api::OpenAi && !model.is_empty() && !ai::is_known(model) {
        return Err(format!("Unknown model {model}").into());
    }
    let candidate = Settings {
        provider: p.id.to_string(),
        ..s.clone()
    };
    endpoint(&candidate)
        .map_err(|e| AppError::User(format!("{} isn't set up yet: {e}", p.label)))?;
    Ok(p.id.to_string())
}

/// Saves the agent model of the current provider (empty: its expert model) and the run limits.
#[tauri::command]
pub fn save_agent_settings(
    state: State<'_, AppState>,
    agent_model: String,
    agent: AgentSettings,
) -> AppResult<SettingsView> {
    let agent = agent.validated()?;
    let agent_model = agent_model.trim().to_string();
    let s = state.store.update_settings(|s| {
        let p = ai::endpoint::provider(&s.provider)?;
        if p.api != Api::OpenAi && !agent_model.is_empty() && !ai::is_known(&agent_model) {
            return Err(format!("Unknown model {agent_model}").into());
        }
        let mut chosen = s.provider_settings(p.id);
        chosen.agent_model = agent_model;
        s.set_provider_settings(p.id, chosen);
        s.agent = agent;
        Ok(s.clone())
    })?;
    Ok(settings_view(&s))
}

/// The models a provider offers, using the key and base URL in the form (falling back to
/// the saved ones). Doubles as a check that the key and address work.
#[tauri::command]
pub async fn list_models(
    state: State<'_, AppState>,
    provider: String,
    base_url: String,
    api_key: Option<String>,
) -> AppResult<Vec<String>> {
    let p = ai::endpoint::provider(&provider)?;
    if p.api == Api::Plan {
        // Checks Claude Code is installed and signed in; the models are Claude's.
        ai::subscription::check().await?;
        return Ok(ai::MODELS.iter().map(|(id, _)| id.to_string()).collect());
    }
    if p.api == Api::Anthropic {
        // Claude's models are fixed; this checks the key so a typo is caught before it's saved.
        let typed = api_key
            .map(|k| k.trim().to_string())
            .filter(|k| !k.is_empty());
        let key = match typed {
            Some(k) => Some(k),
            None => secrets::api_key_async(p.id).await,
        };
        let key =
            key.ok_or_else(|| AppError::User(format!("Paste your {} key first.", p.label)))?;
        let saved = state.store.settings()?;
        // Only Anthropic keys can need a workspace ID; SayQL credit keys never do.
        let workspace = if p.id == ai::ANTHROPIC {
            valid_workspace_id(&saved.workspace_id)?
        } else {
            String::new()
        };
        let http = state.http.clone();
        let base = address(p, &saved.provider_settings(p.id));
        ai::check_claude_key(
            &http,
            &base,
            &key,
            Some(workspace.as_str()).filter(|w| !w.is_empty()),
        )
        .await?;
        return Ok(ai::MODELS.iter().map(|(id, _)| id.to_string()).collect());
    }
    let typed = ai::clean_base_url(&base_url).map_err(AppError::User)?;
    let saved = state.store.settings()?.provider_settings(p.id);
    let url = if typed.is_empty() {
        address(p, &saved)
    } else {
        typed
    };
    if url.is_empty() {
        return Err(AppError::User(format!(
            "Add the base URL of your {} server first.",
            p.label
        )));
    }
    let typed = api_key
        .map(|k| k.trim().to_string())
        .filter(|k| !k.is_empty());
    let key = match (p.key, typed) {
        (KeyNeed::None, _) => None,
        (_, Some(k)) => Some(k),
        (_, None) => secrets::api_key_async(p.id).await,
    };
    if p.key == KeyNeed::Required && key.is_none() {
        return Err(AppError::User(format!(
            "Paste your {} API key first.",
            p.label
        )));
    }
    let http = state.http.clone();
    ai::list_models(&http, p.label, &url, key.as_deref()).await
}

/// What's left on the saved SayQL AI credit key, in US dollars; `None` without one. Asks the
/// credits service the key's AI requests already go to, and nothing else.
#[tauri::command]
pub async fn credits_balance(state: State<'_, AppState>) -> AppResult<Option<f64>> {
    let p = ai::endpoint::provider(ai::SAYQL_CREDITS)?;
    let Some(key) = secrets::api_key_async(p.id).await else {
        return Ok(None);
    };
    let base = address(p, &state.store.settings()?.provider_settings(p.id));
    ai::credits_balance(&state.http, &base, &key)
        .await
        .map(Some)
}

/// Records that the first-run setup was finished or dismissed, so it isn't shown again.
#[tauri::command]
pub fn finish_onboarding(state: State<'_, AppState>) -> AppResult<SettingsView> {
    let s = state.store.update_settings(|s| {
        s.onboarded = true;
        Ok(s.clone())
    })?;
    Ok(settings_view(&s))
}

/// Where the `sayql-mcp` server binary is, for connecting Claude to SayQL.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpSetup {
    path: String,
    /// False when the binary hasn't been built or bundled next to the app.
    exists: bool,
    /// True when running from an AppImage, which mounts at a new path on every launch,
    /// so a path given to an MCP app would stop working after a restart.
    appimage: bool,
    /// "macos", "windows" or "linux": which config paths and shell quoting the setup shows.
    os: &'static str,
}

/// Where the `sayql-mcp` binary is: next to the app's own executable.
pub(crate) fn mcp_binary() -> AppResult<std::path::PathBuf> {
    let exe = std::env::current_exe()?;
    let name = if cfg!(windows) {
        "sayql-mcp.exe"
    } else {
        "sayql-mcp"
    };
    Ok(exe.with_file_name(name))
}

/// The MCP server ships next to the app's own executable (release builds bundle it).
#[tauri::command]
pub fn mcp_setup() -> AppResult<McpSetup> {
    let path = mcp_binary()?;
    Ok(McpSetup {
        exists: path.exists(),
        path: path.to_string_lossy().into_owned(),
        appimage: std::env::var_os("APPIMAGE").is_some(),
        os: std::env::consts::OS,
    })
}

/// The coding agents that look installed (ids from src/features/agent/agents.ts), to preselect
/// one in the setup. Only checks for files; runs nothing.
#[tauri::command]
pub async fn installed_coding_agents() -> AppResult<Vec<&'static str>> {
    tauri::async_runtime::spawn_blocking(crate::coding_agents::installed)
        .await
        .map_err(|e| AppError::User(format!("Could not look for coding agents: {e}")))
}

/// The install links SayQL may open: Cursor's and VS Code's own "add an MCP server" links.
const INSTALL_LINKS: [&str; 2] = [
    "cursor://anysphere.cursor-deeplink/mcp/install?",
    "vscode:mcp/install?",
];

/// Whether `url` is one of the install links, with only URL-encoded text after it.
pub(crate) fn install_link_ok(url: &str) -> bool {
    INSTALL_LINKS.iter().any(|p| {
        url.strip_prefix(p).is_some_and(|rest| {
            !rest.is_empty()
                && rest
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || "%-_.~=&".contains(c))
        })
    })
}

/// Opens Cursor's or VS Code's link that adds SayQL's MCP server; the app asks before adding it.
#[tauri::command]
pub fn open_install_link(app: tauri::AppHandle, url: String) -> AppResult<()> {
    use tauri_plugin_opener::OpenerExt;
    if !install_link_ok(&url) {
        return Err(AppError::User(
            "SayQL only opens Cursor's and VS Code's MCP install links.".into(),
        ));
    }
    app.opener()
        .open_url(url, None::<&str>)
        .map_err(|e| AppError::User(format!("Could not open the link: {e}")))
}

/// The newest entries in the activity log of AI apps using SayQL over MCP.
#[tauri::command]
pub fn mcp_activity(state: State<'_, AppState>, limit: Option<usize>) -> AppResult<Vec<Activity>> {
    state.store.app_activity(limit.unwrap_or(100).min(1000))
}

/// The AI apps in the log, blocked or approved, and whether only approved apps are let in.
#[tauri::command]
pub fn mcp_apps(state: State<'_, AppState>) -> AppResult<AppList> {
    state.store.app_summaries()
}

fn app_name(name: &str) -> AppResult<&str> {
    let name = name.trim();
    if name.is_empty() {
        return Err(AppError::User("Which app? The name is empty.".into()));
    }
    Ok(name)
}

/// Blocks or allows one AI app; returns the updated list.
#[tauri::command]
pub fn set_mcp_app_blocked(
    state: State<'_, AppState>,
    name: String,
    blocked: bool,
) -> AppResult<AppList> {
    state.store.set_app_blocked(app_name(&name)?, blocked)?;
    state.store.app_summaries()
}

/// Approves one AI app (for when only approved apps are let in); returns the updated list.
#[tauri::command]
pub fn approve_mcp_app(state: State<'_, AppState>, name: String) -> AppResult<AppList> {
    state.store.approve_app(app_name(&name)?)?;
    state.store.app_summaries()
}

/// Turns "only allow apps I've approved" on or off; returns the updated list.
#[tauri::command]
pub fn set_mcp_only_approved(state: State<'_, AppState>, on: bool) -> AppResult<AppList> {
    state.store.set_only_approved_apps(on)?;
    state.store.app_summaries()
}

#[tauri::command]
pub fn clear_mcp_activity(state: State<'_, AppState>) -> AppResult<()> {
    state.store.clear_app_activity()
}

/// Your own queries, newest first: from the editor, the Ask box and Investigate; for one
/// connection when given.
#[tauri::command]
pub fn my_activity(
    state: State<'_, AppState>,
    connection_id: Option<String>,
    limit: Option<usize>,
) -> AppResult<Vec<crate::store::my_activity::MyActivity>> {
    state
        .store
        .my_activity(connection_id.as_deref(), limit.unwrap_or(200).min(2000))
}

/// Deletes your activity log.
#[tauri::command]
pub fn clear_my_activity(state: State<'_, AppState>) -> AppResult<()> {
    state.store.clear_my_activity()
}

/// When an AI app last fetched the audit prompt, per connection and database.
#[tauri::command]
pub fn mcp_audits(state: State<'_, AppState>) -> AppResult<Vec<AuditRun>> {
    state.store.audits()
}

/// The audit instructions for one saved connection, to paste into an AI app that can't fetch
/// MCP prompts. The same text as the `audit` prompt, plus which tools to use.
#[tauri::command]
pub fn audit_prompt(
    state: State<'_, AppState>,
    id: String,
    database: Option<String>,
) -> AppResult<String> {
    let conn = state.store.connection(&id)?;
    let database = audit::target_database(&state.store, &conn, database.as_deref());
    Ok(audit::instructions(
        Some(&audit::Target {
            connection_id: &conn.id,
            name: &conn.name,
            database: database.as_deref(),
        }),
        true,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_cursor_and_vscode_install_links_open() {
        assert!(install_link_ok(
            "cursor://anysphere.cursor-deeplink/mcp/install?name=sayql&config=eyJjb21tYW5kIjoiL0EifQ%3D%3D"
        ));
        assert!(install_link_ok(
            "vscode:mcp/install?%7B%22name%22%3A%22sayql%22%7D"
        ));
        assert!(!install_link_ok("vscode:mcp/install?"));
        assert!(!install_link_ok(
            "https://example.com/?vscode:mcp/install?x"
        ));
        assert!(!install_link_ok("vscode:extension/evil"));
        assert!(!install_link_ok("vscode:mcp/install?{\"name\":\"x\"}"));
        assert!(!install_link_ok(
            "cursor://anysphere.cursor-deeplink/mcp/install?a b"
        ));
        assert!(!install_link_ok("file:///etc/passwd"));
    }

    #[test]
    fn the_ask_box_switches_only_to_providers_that_are_ready() {
        let mut s = Settings::default();
        // Ollama needs no key: once it has a model, it's ready.
        assert!(
            choose(&s, "ollama", "qwen3", ModelUse::Typing)
                .unwrap_err()
                .to_string()
                .contains("isn't set up yet")
        );
        s.set_provider_settings(
            "ollama",
            ProviderSettings {
                typing_model: "qwen3".into(),
                ..Default::default()
            },
        );
        assert_eq!(
            choose(&s, "ollama", "llama3.3", ModelUse::Typing).unwrap(),
            "ollama"
        );
        assert!(choose(&s, "ollama", "", ModelUse::Typing).is_err());
        assert_eq!(
            choose(&s, "ollama", "", ModelUse::Agent).unwrap(),
            "ollama",
            "empty agent model means the expert model"
        );
        assert!(
            choose(&s, "anthropic", "claude-unknown", ModelUse::Typing)
                .unwrap_err()
                .to_string()
                .contains("Unknown model")
        );
        assert!(choose(&s, "nope", "x", ModelUse::Typing).is_err());
    }
}

/// Checks the person's own Claude Code for "Your Claude plan": installed, recent enough and
/// signed in. Reads only its version and sign-in status.
#[tauri::command]
pub async fn check_claude_plan() -> AppResult<ai::subscription::PlanCheck> {
    ai::subscription::check().await
}
