//! Team AI: on a team connection, AI requests go through the team server, which swaps in the
//! company's key, forwards them to its provider unchanged (streams included) and counts tokens
//! against monthly limits. The app's own Claude and OpenAI-compatible clients send them; only
//! the address, the auth and two labelling headers change.

use super::{SIGNED_OUT, error_text};
use crate::ai::Api;
use crate::error::AppError;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// `GET /api/v1/ai`: whether team AI is on, its models and this month's usage.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all(serialize = "camelCase", deserialize = "snake_case"))]
pub struct AiStatus {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub models: Vec<String>,
    #[serde(default)]
    pub default_model: Option<String>,
    #[serde(default)]
    pub fast_model: Option<String>,
    #[serde(default)]
    pub month: Option<String>,
    #[serde(default)]
    pub you: Option<AiUsage>,
    #[serde(default)]
    pub team: Option<AiUsage>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, specta::Type)]
pub struct AiUsage {
    #[serde(default)]
    pub tokens: u64,
    /// `None`: no limit.
    #[serde(default)]
    pub limit: Option<u64>,
}

impl AiStatus {
    pub fn api(&self) -> Option<Api> {
        match self.provider.as_deref() {
            Some("anthropic") => Some(Api::Anthropic),
            Some("openai") => Some(Api::OpenAi),
            _ => None,
        }
    }

    /// On, with a provider this app speaks and a model to use.
    pub fn usable(&self) -> bool {
        self.enabled
            && self.api().is_some()
            && self.default_model.as_deref().is_some_and(|m| !m.is_empty())
    }

    /// The model for agent mode, fixes and Improve.
    pub fn default_model(&self) -> String {
        self.default_model.clone().unwrap_or_default()
    }

    /// The model for SQL while typing: the fast one when the admin set one.
    pub fn fast_model(&self) -> String {
        self.fast_model
            .clone()
            .filter(|m| !m.is_empty())
            .unwrap_or_else(|| self.default_model())
    }
}

/// What the usage row on the server says the request was for.
pub fn source_of(task: &crate::ai::Task) -> &'static str {
    match task {
        crate::ai::Task::Draft { .. } => "ask",
        crate::ai::Task::Fix { .. } => "fix",
        crate::ai::Task::Improve { .. } => "improve",
        crate::ai::Task::Explain { .. } => "other",
    }
}

/// The route for an AI request on `connection_id`: through the team server when it is a team
/// connection and team AI is usable, else `None` (the person's own provider, as without a team).
pub fn route(
    connection_id: &str,
    status: &AiStatus,
    client: &super::TeamClient,
    source: &str,
    off: Arc<AtomicBool>,
    own_ready: bool,
) -> Option<(crate::ai::Endpoint, AiStatus)> {
    let server_id = connection_id.strip_prefix(super::ID_PREFIX)?;
    if !status.usable() || off.load(Ordering::Relaxed) {
        return None;
    }
    let api = status.api()?;
    let ai = TeamAi::new(
        client.server(),
        client.token().to_string(),
        api,
        source,
        server_id,
        off,
        own_ready,
    );
    Some((crate::ai::Endpoint::Team(ai), status.clone()))
}

/// One route to the team's AI: the server's passthrough for its provider, with the team token.
#[derive(Clone)]
pub struct TeamAi {
    pub api: Api,
    /// `…/api/v1/ai/anthropic` (the Claude client adds `/v1/messages`) or
    /// `…/api/v1/ai/openai/v1` (the OpenAI-compatible client adds `/chat/completions`).
    pub base: String,
    pub token: String,
    /// `x-sayql-source` and `x-sayql-connection`, for the server's usage rows.
    pub headers: Vec<(&'static str, String)>,
    /// Set when the server says team AI is off, so the next request uses the person's own AI.
    off: Arc<AtomicBool>,
    /// The person has their own AI provider set up to fall back on.
    own_ready: bool,
}

impl TeamAi {
    pub fn new(
        server: &str,
        token: String,
        api: Api,
        source: &str,
        connection: &str,
        off: Arc<AtomicBool>,
        own_ready: bool,
    ) -> Self {
        let server = server.trim_end_matches('/');
        let base = match api {
            // A team server speaks Claude's API or the OpenAI one; it never offers a plan.
            Api::Anthropic | Api::Plan => format!("{server}/api/v1/ai/anthropic"),
            Api::OpenAi => format!("{server}/api/v1/ai/openai/v1"),
        };
        let headers = vec![
            ("x-sayql-source", source.to_string()),
            ("x-sayql-connection", connection.to_string()),
        ];
        TeamAi {
            api,
            base,
            token,
            headers,
            off,
            own_ready,
        }
    }

    /// Adds the team token and the labelling headers to a request.
    pub fn authorize(&self, mut req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        req = req.bearer_auth(&self.token);
        for (k, v) in &self.headers {
            req = req.header(*k, v);
        }
        req
    }

    pub fn unreachable(&self, e: reqwest::Error) -> AppError {
        AppError::User(format!(
            "Couldn't reach your team's SayQL server for AI: {e}"
        ))
    }

    /// A non-2xx reply: the team server's own refusal (limits, sign-in, models), or the
    /// provider's error passed through.
    pub fn error(&self, status: u16, body: &str) -> AppError {
        let v: Value = serde_json::from_str(body).unwrap_or(Value::Null);
        let msg = v["error"]["message"]
            .as_str()
            .unwrap_or("")
            .trim()
            .to_string();
        let or = |fallback: &str| {
            if msg.is_empty() {
                fallback.to_string()
            } else {
                msg.clone()
            }
        };
        match v["error"]["code"].as_str() {
            // Plain, and marked a refusal: nothing to retry or fix with AI.
            Some("ai_limit") => {
                AppError::TeamRefused(or("You've used your team's AI allowance for this month."))
            }
            Some("ai_not_configured") => {
                self.off.store(true, Ordering::Relaxed);
                AppError::TeamRefused(if self.own_ready {
                    "Your team admin turned off team AI. Ask again to use your own AI provider."
                        .into()
                } else {
                    "Your team's AI isn't set up, and you haven't set up an AI provider of your own. Add one in Settings, or ask your team admin.".into()
                })
            }
            Some("bad_request") => {
                AppError::TeamRefused(or("Your team's SayQL server refused this AI request."))
            }
            Some("unauthorized") => AppError::TeamRefused(SIGNED_OUT.into()),
            Some(c @ ("forbidden" | "no_licence" | "licence_expired")) => {
                AppError::TeamRefused(error_text(c, &msg))
            }
            _ => {
                let detail = if msg.is_empty() {
                    body.trim().chars().take(300).collect()
                } else {
                    msg
                };
                AppError::User(format!(
                    "Your team's AI provider returned an error ({status}): {detail}"
                ))
            }
        }
    }
}
