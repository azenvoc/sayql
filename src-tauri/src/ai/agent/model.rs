//! The agent's conversation with a model, the same for every provider: messages of text,
//! tool calls and tool results. `anthropic.rs` and `openai.rs` translate it to their APIs;
//! `text_protocol.rs` covers models without native tool calling; `Mock` scripts it in tests.

use super::text_protocol;
use crate::ai::Endpoint;
use crate::error::AppResult;
use serde_json::Value;

#[derive(Debug, Clone, PartialEq)]
pub enum Block {
    Text(String),
    ToolUse {
        id: String,
        name: String,
        input: Value,
    },
    ToolResult {
        id: String,
        content: String,
        is_error: bool,
    },
    /// A block the provider needs back unchanged on the next turn (Claude's thinking blocks).
    Opaque(Value),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Role {
    User,
    Assistant,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Msg {
    pub role: Role,
    pub blocks: Vec<Block>,
}

impl Msg {
    pub fn user(text: impl Into<String>) -> Msg {
        Msg {
            role: Role::User,
            blocks: vec![Block::Text(text.into())],
        }
    }
}

/// A tool the model may call.
#[derive(Debug, Clone)]
pub struct ToolSpec {
    pub name: &'static str,
    pub description: &'static str,
    /// JSON Schema of the input.
    pub schema: Value,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, serde::Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct Usage {
    /// Prompt tokens billed at the full rate (not read from cache).
    pub input: u64,
    /// Prompt tokens read from the provider's cache.
    pub cached: u64,
    pub output: u64,
}

impl Usage {
    pub fn add(&mut self, u: Usage) {
        self.input += u.input;
        self.cached += u.cached;
        self.output += u.output;
    }

    /// What counts against the run's token budget.
    pub fn billed(&self) -> u64 {
        self.input + self.output
    }
}

/// The model's reply on one turn.
#[derive(Debug, Clone, Default)]
pub struct Turn {
    pub blocks: Vec<Block>,
    pub usage: Usage,
    /// The reply was cut off at the output limit.
    pub truncated: bool,
}

impl Turn {
    pub fn tool_uses(&self) -> impl Iterator<Item = (&str, &str, &Value)> {
        self.blocks.iter().filter_map(|b| match b {
            Block::ToolUse { id, name, input } => Some((id.as_str(), name.as_str(), input)),
            _ => None,
        })
    }

    pub fn text(&self) -> String {
        self.blocks
            .iter()
            .filter_map(|b| {
                if let Block::Text(t) = b {
                    Some(t.as_str())
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// How tools reach the model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ToolMode {
    /// The provider's own tool calling.
    Native,
    /// One fenced JSON action per reply (`text_protocol.rs`).
    Text,
}

/// One turn on whichever client the endpoint speaks.
async fn turn(
    http: &reqwest::Client,
    endpoint: &Endpoint,
    model: &str,
    system: &str,
    schema: &str,
    tools: &[ToolSpec],
    messages: &[Msg],
) -> AppResult<Turn> {
    match (endpoint.claude(), endpoint.openai()) {
        _ if matches!(endpoint, Endpoint::Plan) => {
            crate::ai::subscription::turn(model, system, schema, messages).await
        }
        (Some(access), _) => {
            crate::ai::anthropic::turn(http, &access, model, system, schema, tools, messages).await
        }
        (None, Some(svc)) => {
            crate::ai::openai::turn(http, &svc, model, system, schema, tools, messages).await
        }
        (None, None) => Err("This AI provider isn't supported.".into()),
    }
}

/// A model to run the agent with.
pub enum Llm {
    Remote {
        http: reqwest::Client,
        endpoint: Endpoint,
        model: String,
        mode: ToolMode,
    },
    #[cfg(test)]
    Mock(Mock),
}

impl Llm {
    pub async fn turn(
        &self,
        system: &str,
        schema: &str,
        tools: &[ToolSpec],
        messages: &[Msg],
    ) -> AppResult<Turn> {
        match self {
            Llm::Remote {
                http,
                endpoint,
                model,
                mode: ToolMode::Native,
            } => turn(http, endpoint, model, system, schema, tools, messages).await,
            Llm::Remote {
                http,
                endpoint,
                model,
                mode: ToolMode::Text,
            } => {
                let system = format!("{system}\n\n{}", text_protocol::instructions(tools));
                let messages = text_protocol::to_text(messages);
                let reply = turn(http, endpoint, model, &system, schema, &[], &messages).await?;
                Ok(text_protocol::from_text(reply))
            }
            #[cfg(test)]
            Llm::Mock(m) => m.turn(system, schema, messages).await,
        }
    }

    pub fn mode(&self) -> ToolMode {
        match self {
            Llm::Remote { mode, .. } => *mode,
            #[cfg(test)]
            Llm::Mock(_) => ToolMode::Native,
        }
    }

    pub fn model_name(&self) -> &str {
        match self {
            Llm::Remote { model, .. } => model,
            #[cfg(test)]
            Llm::Mock(_) => "mock",
        }
    }
}

/// Tests: replies with scripted turns, and records every request it was sent.
#[cfg(test)]
pub struct Mock {
    script: std::sync::Mutex<std::collections::VecDeque<MockTurn>>,
    pub seen: std::sync::Mutex<Vec<String>>,
}

/// Chooses the tool calls of a turn from the last message's text.
#[cfg(test)]
pub type Pick = Box<dyn Fn(&str) -> Vec<(&'static str, Value)> + Send>;

#[cfg(test)]
pub enum MockTurn {
    /// Call these tools: (name, input).
    Tools(Vec<(&'static str, Value)>),
    /// A reply that calls no tool.
    Text(&'static str),
    /// Pick the next turn from what was sent so far (the last message's text).
    Dynamic(Pick),
    /// Never answers, for testing Stop and the run time limit.
    Hang,
}

#[cfg(test)]
impl Mock {
    pub fn new(turns: Vec<MockTurn>) -> Mock {
        Mock {
            script: std::sync::Mutex::new(turns.into()),
            seen: Default::default(),
        }
    }

    async fn turn(&self, system: &str, schema: &str, messages: &[Msg]) -> AppResult<Turn> {
        let sent = format!(
            "{system}\n{schema}\n{}",
            messages.iter().map(render).collect::<Vec<_>>().join("\n")
        );
        self.seen.lock().unwrap().push(sent);
        let last = messages.last().map(render).unwrap_or_default();
        let next = self.script.lock().unwrap().pop_front();
        let next = next.ok_or("The mock model ran out of turns.")?;
        let n = messages.len();
        let tools = |calls: Vec<(&'static str, Value)>| Turn {
            blocks: calls
                .into_iter()
                .enumerate()
                .map(|(i, (name, input))| Block::ToolUse {
                    id: format!("t{n}_{i}"),
                    name: name.into(),
                    input,
                })
                .collect(),
            usage: Usage {
                input: 100,
                cached: 1000,
                output: 50,
            },
            truncated: false,
        };
        Ok(match next {
            MockTurn::Tools(calls) => tools(calls),
            MockTurn::Dynamic(f) => tools(f(&last)),
            MockTurn::Text(t) => Turn {
                blocks: vec![Block::Text(t.into())],
                usage: Usage {
                    input: 10,
                    cached: 0,
                    output: 10,
                },
                truncated: false,
            },
            MockTurn::Hang => {
                tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
                return Err("The mock model hung.".into());
            }
        })
    }
}

/// A message as plain text, for the mock's records and the text protocol.
pub fn render(m: &Msg) -> String {
    m.blocks
        .iter()
        .map(|b| match b {
            Block::Text(t) => t.clone(),
            Block::ToolUse { name, input, .. } => format!("[call {name} {input}]"),
            Block::ToolResult {
                content, is_error, ..
            } => {
                if *is_error {
                    format!("[error] {content}")
                } else {
                    content.clone()
                }
            }
            Block::Opaque(_) => String::new(),
        })
        .collect::<Vec<_>>()
        .join("\n")
}
