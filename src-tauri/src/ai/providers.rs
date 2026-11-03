//! The AI services SayQL can write SQL with. Claude has its own API; every other entry
//! speaks the OpenAI-compatible Chat Completions API, so one client covers them all.
//! Anything not listed works through "Other" with its own base URL.

use serde::Serialize;

/// Which wire protocol a provider speaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "lowercase")]
pub enum Api {
    Anthropic,
    OpenAi,
    /// The person's own installed Claude Code on their own Claude plan (`subscription.rs`).
    Plan,
}

/// Whether a provider needs an API key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "lowercase")]
pub enum KeyNeed {
    Required,
    Optional,
    None,
}

pub struct Provider {
    pub id: &'static str,
    pub label: &'static str,
    /// One line shown under the picker in Settings.
    pub note: &'static str,
    pub api: Api,
    /// Empty when the person must enter one ("Other").
    pub base_url: &'static str,
    pub key: KeyNeed,
    pub key_placeholder: &'static str,
    /// Runs on this computer, so questions and schemas never leave it.
    pub local: bool,
}

pub const ANTHROPIC: &str = "anthropic";
/// Prepaid Claude from SayQL: the same Messages API at SayQL's address, with a `sqlc_` credit key.
pub const SAYQL_CREDITS: &str = "sayql";
/// The person's own Claude plan, through their installed Claude Code.
pub const CLAUDE_PLAN: &str = "claude-plan";

const fn openai(
    id: &'static str,
    label: &'static str,
    note: &'static str,
    base_url: &'static str,
    key_placeholder: &'static str,
) -> Provider {
    Provider {
        id,
        label,
        note,
        api: Api::OpenAi,
        base_url,
        key: KeyNeed::Required,
        key_placeholder,
        local: false,
    }
}

const fn local(
    id: &'static str,
    label: &'static str,
    note: &'static str,
    base_url: &'static str,
) -> Provider {
    Provider {
        id,
        label,
        note,
        api: Api::OpenAi,
        base_url,
        key: KeyNeed::None,
        key_placeholder: "",
        local: true,
    }
}

pub const PROVIDERS: &[Provider] = &[
    Provider {
        id: ANTHROPIC,
        label: "Anthropic Claude",
        note: "Tuned for SayQL, with prompt caching for fast drafts.",
        api: Api::Anthropic,
        base_url: "https://api.anthropic.com",
        key: KeyNeed::Required,
        key_placeholder: "sk-ant-…",
        local: false,
    },
    Provider {
        id: SAYQL_CREDITS,
        label: "SayQL AI credits",
        note: "Claude without an Anthropic account: prepaid credit from SayQL. Requests go through SayQL to Anthropic.",
        api: Api::Anthropic,
        base_url: "https://sayql.dev/api/ai",
        key: KeyNeed::Required,
        key_placeholder: "sqlc_…",
        local: false,
    },
    Provider {
        id: CLAUDE_PLAN,
        label: "Your Claude plan",
        note: "Uses your installed Claude Code, signed in with your own Pro or Max plan. No API key; requests count against your plan's limits.",
        api: Api::Plan,
        base_url: "",
        key: KeyNeed::None,
        key_placeholder: "",
        local: false,
    },
    openai(
        "openai",
        "OpenAI",
        "GPT models with an OpenAI API key.",
        "https://api.openai.com/v1",
        "sk-…",
    ),
    openai(
        "gemini",
        "Google Gemini",
        "Gemini models with a Google AI Studio key.",
        "https://generativelanguage.googleapis.com/v1beta/openai",
        "AIza…",
    ),
    openai(
        "openrouter",
        "OpenRouter",
        "Hundreds of models from many labs with one key.",
        "https://openrouter.ai/api/v1",
        "sk-or-…",
    ),
    openai(
        "mistral",
        "Mistral",
        "Mistral models with a Mistral API key.",
        "https://api.mistral.ai/v1",
        "",
    ),
    openai(
        "groq",
        "Groq",
        "Open models on very fast hardware.",
        "https://api.groq.com/openai/v1",
        "gsk_…",
    ),
    openai(
        "deepseek",
        "DeepSeek",
        "DeepSeek models with a DeepSeek API key.",
        "https://api.deepseek.com/v1",
        "sk-…",
    ),
    local(
        "ollama",
        "Ollama",
        "Runs on this computer. Nothing leaves your machine.",
        "http://localhost:11434/v1",
    ),
    local(
        "lmstudio",
        "LM Studio",
        "Runs on this computer. Nothing leaves your machine.",
        "http://localhost:1234/v1",
    ),
    Provider {
        id: "custom",
        label: "Other (OpenAI-compatible)",
        note: "Any server with an OpenAI-compatible API: Azure, vLLM, LiteLLM, a company gateway…",
        api: Api::OpenAi,
        base_url: "",
        key: KeyNeed::Optional,
        key_placeholder: "",
        local: false,
    },
];

pub fn find(id: &str) -> Option<&'static Provider> {
    PROVIDERS.iter().find(|p| p.id == id)
}

/// A base URL the person typed: trimmed, without a trailing slash, and http(s) only.
pub fn clean_base_url(raw: &str) -> Result<String, String> {
    let url = raw.trim().trim_end_matches('/');
    if url.is_empty() || url.starts_with("https://") || url.starts_with("http://") {
        Ok(url.to_string())
    } else {
        Err(format!(
            "The base URL must start with https:// or http:// (got \"{url}\")."
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_unique_and_claude_is_first() {
        assert_eq!(PROVIDERS[0].id, ANTHROPIC);
        for (i, p) in PROVIDERS.iter().enumerate() {
            assert!(
                PROVIDERS[i + 1..].iter().all(|q| q.id != p.id),
                "duplicate {}",
                p.id
            );
            if p.api != Api::Plan {
                assert_eq!(p.local, p.key == KeyNeed::None, "{}", p.id);
            }
        }
    }

    #[test]
    fn base_urls_are_cleaned_and_checked() {
        assert_eq!(
            clean_base_url(" http://localhost:11434/v1/ ").unwrap(),
            "http://localhost:11434/v1"
        );
        assert_eq!(clean_base_url("").unwrap(), "");
        assert!(clean_base_url("localhost:11434").is_err());
    }
}
