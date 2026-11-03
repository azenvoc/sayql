//! English → SQL generation, streamed to the UI as Tauri events. Claude goes over the
//! Messages API; every other provider over the OpenAI-compatible Chat Completions API.

pub mod agent;
pub(crate) mod anthropic;
pub mod context;
pub mod endpoint;
mod models;
pub(crate) mod openai;
mod prompt;
mod providers;

#[cfg(test)]
pub(crate) use crate::safety::redact::database_error as mask_database_error;
pub mod settings;
mod sse;
pub mod subscription;
#[cfg(test)]
mod team_tests;

pub use models::{DEFAULT_EXPERT_MODEL, DEFAULT_TYPING_MODEL, MODELS, is_known};
pub use prompt::Task;
pub use providers::{
    ANTHROPIC, Api, CLAUDE_PLAN, KeyNeed, PROVIDERS, Provider, SAYQL_CREDITS, clean_base_url,
    find as find_provider,
};

use crate::error::AppResult;
use serde::Serialize;
use std::time::Instant;
use tauri::{AppHandle, Emitter};

// Event names; the UI gets them from the generated src/api/bindings.ts (`AI_DELTA_EVENT`, …).
pub(crate) const EVENT_DELTA: &str = "ai://delta";
pub(crate) const EVENT_DONE: &str = "ai://done";
pub(crate) const EVENT_RESET: &str = "ai://reset";

/// Where a request goes, with what it needs to authenticate.
pub enum Endpoint {
    /// Claude's API with a key: Anthropic's own (`base` = `anthropic::API_BASE`), or SayQL AI
    /// credits at SayQL's address.
    Anthropic {
        api_key: String,
        workspace_id: Option<String>,
        base: String,
    },
    OpenAi {
        label: &'static str,
        base_url: String,
        api_key: Option<String>,
    },
    /// A team connection with team AI: through the team server, with the team token.
    Team(crate::team::ai::TeamAi),
    /// The person's own Claude Code on their own Claude plan.
    Plan,
}

impl Endpoint {
    /// Speaks the OpenAI-compatible API (tool calling may need the text protocol).
    pub fn is_openai(&self) -> bool {
        match self {
            Endpoint::OpenAi { .. } => true,
            Endpoint::Team(t) => t.api == Api::OpenAi,
            Endpoint::Anthropic { .. } | Endpoint::Plan => false,
        }
    }

    /// The Claude client's access for a Claude endpoint.
    pub(crate) fn claude(&self) -> Option<anthropic::Access<'_>> {
        match self {
            Endpoint::Anthropic {
                api_key,
                workspace_id,
                base,
            } => Some(anthropic::Access::Key {
                api_key,
                workspace_id: workspace_id.as_deref(),
                base,
            }),
            Endpoint::Team(t) if t.api == Api::Anthropic => Some(anthropic::Access::Team(t)),
            _ => None,
        }
    }

    /// The OpenAI-compatible client's service for any other endpoint.
    pub(crate) fn openai(&self) -> Option<openai::Service<'_>> {
        match self {
            Endpoint::OpenAi {
                label,
                base_url,
                api_key,
            } => Some(openai::Service {
                label,
                base_url,
                api_key: api_key.as_deref(),
                team: None,
            }),
            Endpoint::Team(t) if t.api == Api::OpenAi => Some(openai::Service {
                label: "your team's AI",
                base_url: &t.base,
                api_key: Some(&t.token),
                team: Some(t),
            }),
            _ => None,
        }
    }
}

pub struct GenRequest {
    pub req_id: u64,
    pub endpoint: Endpoint,
    pub model: String,
    pub dialect: String,
    pub schema_text: String,
    pub question: String,
    pub task: Task,
    /// The instruction text of the skill the person picked, if any.
    pub skill: Option<String>,
}

#[derive(Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Delta {
    req_id: u64,
    text: String,
    /// Set on the first chunk: milliseconds from request start to the first SQL text.
    first_token_ms: Option<u128>,
}

#[derive(Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Done {
    req_id: u64,
    error: Option<String>,
    total_ms: u128,
    cache_read_tokens: Option<u64>,
}

#[derive(Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Reset {
    req_id: u64,
}

/// Where streamed text goes: the UI, or a string in tests.
enum Sink<'a> {
    App(&'a AppHandle),
    #[cfg(test)]
    Collect(String),
}

/// Sends one request's streamed text to the UI.
struct Output<'a> {
    sink: Sink<'a>,
    req_id: u64,
    start: Instant,
    first: bool,
}

impl<'a> Output<'a> {
    fn new(sink: Sink<'a>, req_id: u64) -> Self {
        Output {
            sink,
            req_id,
            start: Instant::now(),
            first: true,
        }
    }

    fn text(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        let first_token_ms = self.first.then(|| self.start.elapsed().as_millis());
        self.first = false;
        match &mut self.sink {
            Sink::App(app) => {
                let _ = app.emit(
                    EVENT_DELTA,
                    Delta {
                        req_id: self.req_id,
                        text: text.to_string(),
                        first_token_ms,
                    },
                );
            }
            #[cfg(test)]
            Sink::Collect(s) => s.push_str(text),
        }
    }

    /// Discards what was sent so far; the answer starts over.
    fn reset(&mut self) {
        self.first = true;
        match &mut self.sink {
            Sink::App(app) => {
                let _ = app.emit(
                    EVENT_RESET,
                    Reset {
                        req_id: self.req_id,
                    },
                );
            }
            #[cfg(test)]
            Sink::Collect(s) => s.clear(),
        }
    }
}

/// Streams SQL for one question. Emits `ai://delta` chunks, then exactly one `ai://done`.
pub async fn stream_sql(app: AppHandle, http: reqwest::Client, r: GenRequest) {
    let mut out = Output::new(Sink::App(&app), r.req_id);
    let result = match (r.endpoint.claude(), r.endpoint.openai()) {
        _ if matches!(r.endpoint, Endpoint::Plan) => subscription::run(&mut out, &r).await,
        (Some(access), _) => anthropic::run(&mut out, &http, &access, &r).await,
        (None, Some(svc)) => openai::run(&mut out, &http, &svc, &r).await,
        (None, None) => Err("This AI provider isn't supported.".into()),
    };
    let (error, cache_read_tokens) = match result {
        Ok(cached) => (None, cached),
        Err(e) => (Some(e.to_string()), None),
    };
    let done = Done {
        req_id: r.req_id,
        error,
        total_ms: out.start.elapsed().as_millis(),
        cache_read_tokens,
    };
    let _ = app.emit(EVENT_DONE, done);
}

/// Checks a key for Claude's API at `base` (Anthropic's, or SayQL AI credits').
pub async fn check_claude_key(
    http: &reqwest::Client,
    base: &str,
    api_key: &str,
    workspace_id: Option<&str>,
) -> AppResult<()> {
    anthropic::check_key(http, base, api_key, workspace_id).await
}

/// What's left on a SayQL AI credit key at `base`, in US dollars.
pub async fn credits_balance(http: &reqwest::Client, base: &str, api_key: &str) -> AppResult<f64> {
    anthropic::credits_balance(http, base, api_key).await
}

/// The models an OpenAI-compatible service offers; also checks its key and URL.
pub async fn list_models(
    http: &reqwest::Client,
    label: &str,
    base_url: &str,
    api_key: Option<&str>,
) -> AppResult<Vec<String>> {
    openai::list_models(
        http,
        &openai::Service {
            label,
            base_url,
            api_key,
            team: None,
        },
    )
    .await
}
