//! Settings: the commands for Settings → AI provider, the Ask box's model picker, agent
//! mode's settings, the MCP setup and AI apps, and activity logs. Thin: the AI settings logic
//! is in ai/settings.rs.

use crate::ai;
use crate::ai::settings::{ModelUse, SettingsView, view};
use crate::audit;
use crate::error::{AppError, AppResult};
use crate::state::AppState;
use crate::store::agent::AgentSettings;
use crate::store::apps::{Activity, AppList};
use crate::store::audits::AuditRun;
use serde::Serialize;
use tauri::State;

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
    let s = ai::settings::save_provider(
        &state.store,
        &provider,
        &typing_model,
        &expert_model,
        &base_url,
        api_key,
        &workspace_id,
    )?;
    Ok(view(&s))
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
    let s = ai::settings::choose_model(&state.store, &provider, &model, purpose)?;
    Ok(view(&s))
}

/// Saves the agent model of the current provider (empty: its expert model) and the run limits.
#[tauri::command]
pub fn save_agent_settings(
    state: State<'_, AppState>,
    agent_model: String,
    agent: AgentSettings,
) -> AppResult<SettingsView> {
    let s = ai::settings::save_agent(&state.store, &agent_model, agent)?;
    Ok(view(&s))
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
    ai::settings::list_models(&state.store, &state.http, &provider, &base_url, api_key).await
}

/// What's left on the saved SayQL AI credit key, in US dollars; `None` without one. Asks the
/// credits service the key's AI requests already go to, and nothing else.
#[tauri::command]
pub async fn credits_balance(state: State<'_, AppState>) -> AppResult<Option<f64>> {
    ai::settings::credits_balance(&state.store, &state.http).await
}

/// Records that the first-run setup was finished or dismissed, so it isn't shown again.
#[tauri::command]
pub fn finish_onboarding(state: State<'_, AppState>) -> AppResult<SettingsView> {
    let mut s = state.store.settings()?;
    s.onboarded = true;
    state.store.save_settings(&s)?;
    Ok(view(&s))
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

/// The MCP server ships next to the app's own executable (release builds bundle it).
#[tauri::command]
pub fn mcp_setup() -> AppResult<McpSetup> {
    let path = crate::mcp::binary_path()?;
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
    tauri::async_runtime::spawn_blocking(crate::audit::coding_agents::installed)
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
}

/// Checks the person's own Claude Code for "Your Claude plan": installed, recent enough and
/// signed in. Reads only its version and sign-in status.
#[tauri::command]
pub async fn check_claude_plan() -> AppResult<ai::subscription::PlanCheck> {
    ai::subscription::check().await
}
