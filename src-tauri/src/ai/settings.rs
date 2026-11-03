//! Settings → AI provider and the Ask box's model picker, without Tauri: what the window may
//! know about the settings (never a key), checking and saving a provider's choices, switching
//! models, agent mode's model and limits, and listing a provider's models (which also checks
//! its key and address). The commands in commands/settings.rs call these.

use super::endpoint::{address, endpoint, valid_workspace_id};
use super::{Api, KeyNeed};
use crate::error::{AppError, AppResult};
use crate::store::agent::AgentSettings;
use crate::store::{ProviderSettings, Settings, Store, secrets};
use serde::Serialize;

#[derive(Serialize, specta::Type)]
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

#[derive(Serialize, specta::Type)]
struct ModelOption {
    id: &'static str,
    label: &'static str,
}

/// One provider as Settings shows it: the catalog entry plus what the person chose for it.
#[derive(Serialize, specta::Type)]
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
pub fn view(s: &Settings) -> SettingsView {
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
        models: super::MODELS
            .iter()
            .map(|(id, label)| ModelOption { id, label })
            .collect(),
        providers: super::PROVIDERS
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

/// Saves the choices for `provider` and makes it the one that writes SQL: the saved settings.
/// `api_key: None` keeps the saved key; an empty `expert_model` reuses the typing model.
pub fn save_provider(
    store: &Store,
    provider: &str,
    typing_model: &str,
    expert_model: &str,
    base_url: &str,
    api_key: Option<String>,
    workspace_id: &str,
) -> AppResult<Settings> {
    let p = super::endpoint::provider(provider)?;
    let typing_model = typing_model.trim().to_string();
    let expert_model = expert_model.trim().to_string();
    let base_url = super::clean_base_url(base_url).map_err(AppError::User)?;
    match p.api {
        Api::Anthropic | Api::Plan => {
            for model in [&typing_model, &expert_model] {
                if !super::is_known(model) {
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
    let workspace_id = valid_workspace_id(workspace_id)?;
    if let Some(key) = api_key.filter(|_| p.key != KeyNeed::None) {
        secrets::set_api_key(p.id, &key)?;
    }
    // Claude's address is fixed; a base URL equal to the default is stored as "use the default".
    let base_url = if p.api != Api::OpenAi || base_url == p.base_url {
        String::new()
    } else {
        base_url
    };
    store.update_settings(|s| {
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
    })
}

/// Which model slot the Ask box's model picker sets.
#[derive(serde::Deserialize, Clone, Copy, PartialEq, specta::Type)]
#[serde(rename_all = "lowercase")]
pub enum ModelUse {
    /// Quick mode: writes SQL while the person types.
    Typing,
    /// Agent mode.
    Agent,
}

/// Switches to `provider` (one that is ready) and uses `model` for typing or for agent mode:
/// the saved settings. Other choices for that provider stay as they were.
pub fn choose_model(
    store: &Store,
    provider: &str,
    model: &str,
    purpose: ModelUse,
) -> AppResult<Settings> {
    // Checked before the settings file is locked: the check may read the keychain.
    let provider = choose(&store.settings()?, provider, model.trim(), purpose)?;
    let p = super::endpoint::provider(&provider)?;
    store.update_settings(|s| {
        s.provider = provider;
        let mut chosen = s.provider_settings(p.id);
        match purpose {
            ModelUse::Typing => chosen.typing_model = model.trim().to_string(),
            ModelUse::Agent => chosen.agent_model = model.trim().to_string(),
        }
        s.set_provider_settings(p.id, chosen);
        Ok(s.clone())
    })
}

/// The provider id to switch to, after checking it's ready and the model makes sense for it.
fn choose(s: &Settings, provider: &str, model: &str, purpose: ModelUse) -> AppResult<String> {
    let p = super::endpoint::provider(provider)?;
    if model.is_empty() && purpose == ModelUse::Typing {
        return Err("Choose a model.".into());
    }
    if p.api != Api::OpenAi && !model.is_empty() && !super::is_known(model) {
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

/// Saves the agent model of the current provider (empty: its expert model) and the run
/// limits: the saved settings.
pub fn save_agent(store: &Store, agent_model: &str, agent: AgentSettings) -> AppResult<Settings> {
    let agent = agent.validated()?;
    let agent_model = agent_model.trim().to_string();
    store.update_settings(|s| {
        let p = super::endpoint::provider(&s.provider)?;
        if p.api != Api::OpenAi && !agent_model.is_empty() && !super::is_known(&agent_model) {
            return Err(format!("Unknown model {agent_model}").into());
        }
        let mut chosen = s.provider_settings(p.id);
        chosen.agent_model = agent_model;
        s.set_provider_settings(p.id, chosen);
        s.agent = agent;
        Ok(s.clone())
    })
}

/// The models a provider offers, using the key and base URL in the form (falling back to
/// the saved ones). Doubles as a check that the key and address work.
pub async fn list_models(
    store: &Store,
    http: &reqwest::Client,
    provider: &str,
    base_url: &str,
    api_key: Option<String>,
) -> AppResult<Vec<String>> {
    let p = super::endpoint::provider(provider)?;
    if p.api == Api::Plan {
        // Checks Claude Code is installed and signed in; the models are Claude's.
        super::subscription::check().await?;
        return Ok(super::MODELS.iter().map(|(id, _)| id.to_string()).collect());
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
        let saved = store.settings()?;
        // Only Anthropic keys can need a workspace ID; SayQL credit keys never do.
        let workspace = if p.id == super::ANTHROPIC {
            valid_workspace_id(&saved.workspace_id)?
        } else {
            String::new()
        };
        let base = address(p, &saved.provider_settings(p.id));
        super::check_claude_key(
            http,
            &base,
            &key,
            Some(workspace.as_str()).filter(|w| !w.is_empty()),
        )
        .await?;
        return Ok(super::MODELS.iter().map(|(id, _)| id.to_string()).collect());
    }
    let typed = super::clean_base_url(base_url).map_err(AppError::User)?;
    let saved = store.settings()?.provider_settings(p.id);
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
    super::list_models(http, p.label, &url, key.as_deref()).await
}

/// What's left on the saved SayQL AI credit key, in US dollars; `None` without one. Asks the
/// credits service the key's AI requests already go to, and nothing else.
pub async fn credits_balance(store: &Store, http: &reqwest::Client) -> AppResult<Option<f64>> {
    let p = super::endpoint::provider(super::SAYQL_CREDITS)?;
    let Some(key) = secrets::api_key_async(p.id).await else {
        return Ok(None);
    };
    let base = address(p, &store.settings()?.provider_settings(p.id));
    super::credits_balance(http, &base, &key).await.map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;

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
