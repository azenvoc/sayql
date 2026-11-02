//! One agent run: the model asks for tools, SayQL runs each through the read-only gate,
//! masks or summarizes the result per the connection's setting, and reports every step to
//! the tab. Ends with a cited answer, an error, or Stop.

use super::mask::{Masker, ResultSummary};
use super::model::{Block, Llm, Msg, Role, ToolMode, Turn, Usage};
use super::{checks, prompt, tools};
use crate::db::{self, DbPool, Kind, QueryResult};
use crate::readonly;
use crate::schema::{self, Schema, Table};
use crate::store::Store;
use crate::store::agent::{AgentAccess, AgentSettings, Note, Results, clean_note, note_ref};
use crate::store::suggestions::Change as Suggest;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

/// Rows SayQL reads per query.
pub const ROWS_READ: usize = 1000;
/// Rows the model sees per query, with "Up to 20 rows per query, masked".
pub const ROWS_SENT: usize = 20;
/// Most common values `profile_column` returns.
const TOP_VALUES: usize = 10;

pub struct RunInput {
    pub run_id: String,
    /// The question, or for a follow-up the next message in the conversation.
    pub question: String,
    pub kind: Kind,
    pub schema: Schema,
    /// The schema as the prompt shows it.
    pub schema_text: String,
    pub notes: Vec<Note>,
    /// Where the notes are kept: when set, the run records note checks and doubts there and
    /// its suggested notes wait there for the person.
    pub notes_home: Option<NotesHome>,
    pub focus: Vec<String>,
    pub skill: Option<String>,
    pub access: AgentAccess,
    pub limits: AgentSettings,
    /// The tab's earlier run, for follow-ups and steering.
    pub history: Option<History>,
}

/// The database whose notes a run reads, and the store that keeps them.
#[derive(Clone)]
pub struct NotesHome {
    pub store: Store,
    pub connection_id: String,
    pub database: Option<String>,
    /// On a team connection whose server keeps shared notes: the run read the team's notes,
    /// and its suggested notes go to the server for the knowledge owners to approve.
    pub team: Option<TeamNotes>,
}

/// Where a team connection's shared notes are kept.
#[derive(Clone)]
pub struct TeamNotes {
    pub client: crate::team::TeamClient,
    /// Read again after a suggestion.
    pub cache: Arc<crate::team::knowledge::Cache>,
}

/// How Investigate signs the notes it suggests.
pub const NOTES_SOURCE: &str = "Investigate";

/// A finished run's conversation, kept so the next message in the tab can continue it.
#[derive(Clone)]
pub struct History {
    pub messages: Vec<Msg>,
    pub masker: Masker,
    pub next_step: u32,
}

/// How a run talks to the tab: events out, the person's replies in, Stop.
pub struct Io {
    pub emit: Arc<dyn Fn(Event) + Send + Sync>,
    pub replies: mpsc::UnboundedReceiver<Reply>,
    pub stop: CancellationToken,
}

#[derive(Debug, Clone, Serialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum Event {
    /// The model's plan; with `waiting`, the run waits for Go (or an edited plan).
    Plan {
        run_id: String,
        steps: Vec<String>,
        waiting: bool,
    },
    /// A step started (`running`) or ended.
    Step { run_id: String, step: Step },
    /// The run waits for Send or Skip on what would go to the AI.
    Send {
        run_id: String,
        step_id: u32,
        payload: String,
    },
    /// The model asked the person a question; the run waits for the answer.
    Question {
        run_id: String,
        step_id: u32,
        question: String,
        choices: Vec<String>,
    },
    Progress {
        run_id: String,
        turns: u32,
        max_turns: u32,
        queries: u32,
        max_queries: u32,
        usage: Usage,
        elapsed_ms: u128,
    },
    Done {
        run_id: String,
        answer: Option<Answer>,
        error: Option<String>,
        stopped: bool,
    },
}

#[derive(Debug, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum Reply {
    /// Start from the plan; `plan` when the person changed it.
    Go {
        plan: Option<Vec<String>>,
    },
    Send,
    Skip,
    Answer {
        text: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum StepStatus {
    Running,
    Done,
    Error,
}

/// One step as the person sees it. `sql`, `columns` and `rows` are real values: they stay
/// on this computer. `sent` is exactly what went to the AI.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Step {
    pub id: u32,
    pub tool: String,
    pub text: String,
    pub status: StepStatus,
    pub sql: Option<String>,
    pub row_count: Option<usize>,
    pub truncated: bool,
    pub elapsed_ms: Option<u128>,
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Value>>,
    pub sent: Option<String>,
    /// "sent 20 rows, 3 columns masked", "sent row count only", ...
    pub sent_summary: Option<String>,
    pub checks: Vec<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Answer {
    /// With real values filled back in.
    pub text: String,
    pub sentences: Vec<Sentence>,
    pub confidence: String,
    pub assumptions: Vec<String>,
    /// Suggested database notes, already checked to hold no values.
    pub notes: Vec<String>,
    /// The suggested notes (and any fixes to saved ones) wait with the other suggestions for
    /// the person to accept, instead of being offered here to save.
    pub notes_queued: bool,
    /// The suggested notes went to the team server, for its knowledge owners to approve.
    pub notes_shared: bool,
    /// A statement for the person to review and run themselves; SayQL never runs it.
    pub proposed_sql: Option<String>,
    /// The proposed statement could change data (else it is a query to run with another login, say).
    pub proposed_changes_data: bool,
    pub cannot_answer: bool,
    /// Numbers in the answer that no step's result contains.
    pub unverified: Vec<String>,
    pub model: String,
    pub usage: Usage,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Sentence {
    pub text: String,
    pub steps: Vec<u32>,
    pub unverified: Vec<String>,
}

/// What a run leaves behind.
pub struct Outcome {
    pub history: History,
    /// How tools reached the model in the end (it may have fallen back to text).
    pub mode: ToolMode,
    /// The evaluation reads the answer here; the app gets it from the Done event.
    #[cfg_attr(not(test), allow(dead_code, reason = "only the evaluation reads it"))]
    pub answer: Option<Answer>,
}

struct Run<'a> {
    input: RunInput,
    llm: &'a mut Llm,
    pool: &'a DbPool,
    io: &'a mut Io,
    masker: Masker,
    messages: Vec<Msg>,
    next_step: u32,
    steps: Vec<Step>,
    results: Vec<QueryResult>,
    /// SQL the model checked with explain_query, which may then read a big table in full.
    explained: Vec<String>,
    /// Result positions of the last checked query that hold never-send values (see never_send.rs).
    withheld: Vec<usize>,
    /// The answer's claims about all rows were sent back once to be checked.
    reviewed: bool,
    /// The schema was read again after a missing table or column.
    reread: bool,
    turns: u32,
    queries: u32,
    usage: Usage,
    started: Instant,
    deadline: Instant,
}

enum End {
    Answer(Box<Answer>),
    Error(String),
    Stopped,
}

pub async fn run(llm: &mut Llm, pool: &DbPool, mut input: RunInput, io: &mut Io) -> Outcome {
    let started = Instant::now();
    let deadline = started + Duration::from_secs(input.limits.run_seconds);
    let (messages, masker, next_step) = match input.history.take() {
        Some(h) => (h.messages, h.masker, h.next_step),
        None => (Vec::new(), Masker::new(&input.access.never_send), 1),
    };
    let mut r = Run {
        llm,
        pool,
        io,
        masker,
        messages,
        next_step,
        steps: Vec::new(),
        results: Vec::new(),
        explained: Vec::new(),
        withheld: Vec::new(),
        reviewed: false,
        reread: false,
        turns: 0,
        queries: 0,
        usage: Usage::default(),
        started,
        deadline,
        input,
    };
    let first = if r.messages.is_empty() {
        prompt::question(&r.input.question, &r.input.focus, r.input.skill.as_deref())
    } else {
        // A follow-up can quote values the person saw on screen (an edited step's SQL); they go masked.
        r.masker.remask_text(&r.input.question)
    };
    r.messages.push(Msg::user(first));
    let end = r.go().await;
    let run_id = r.input.run_id.clone();
    let (answer, error, stopped) = match end {
        End::Answer(a) => (Some(*a), None, false),
        End::Error(e) => (None, Some(e), false),
        End::Stopped => (None, None, true),
    };
    (r.io.emit)(Event::Done {
        run_id,
        answer: answer.clone(),
        error,
        stopped,
    });
    // A run that ended on a tool call leaves it unanswered; close it so the next message can follow.
    if let Some(Msg {
        role: Role::Assistant,
        blocks,
    }) = r.messages.last()
    {
        let open: Vec<Block> = blocks
            .iter()
            .filter_map(|b| match b {
                Block::ToolUse { id, .. } => Some(Block::ToolResult {
                    id: id.clone(),
                    content: "The run ended here.".into(),
                    is_error: false,
                }),
                _ => None,
            })
            .collect();
        if !open.is_empty() {
            r.messages.push(Msg {
                role: Role::User,
                blocks: open,
            });
        }
    }
    let mode = r.llm.mode();
    Outcome {
        history: History {
            messages: r.messages,
            masker: r.masker,
            next_step: r.next_step,
        },
        mode,
        answer,
    }
}

impl Run<'_> {
    async fn go(&mut self) -> End {
        let dialect = self.input.schema.dialect.clone();
        let system = prompt::instructions(&dialect, self.input.access.results);
        // The notes about the tables the prompt shows, and those about no table; the rest come
        // with describe_table.
        let shown = crate::schema::anchors::tables_in(&self.input.schema_text, &self.input.schema);
        let notes: Vec<Note> = self
            .input
            .notes
            .iter()
            .filter(|n| {
                n.anchors.is_empty()
                    || n.anchors.iter().any(|a| {
                        shown
                            .iter()
                            .any(|t| crate::team::knowledge::names(&a.table, t))
                    })
            })
            .cloned()
            .collect();
        let mut schema = prompt::schema_block(&dialect, &self.input.schema_text, &notes);
        if let Some(warning) = self.row_security().await {
            schema.push_str(&format!(
                "\n\nWarning: {warning} Say so in the answer when results look empty or small."
            ));
        }
        let mut nudged = false;
        let mut switched = false;
        loop {
            let last_turn = self.turns + 1 >= self.input.limits.max_turns();
            let offered = if last_turn {
                tools::finish_only()
            } else {
                tools::all()
            };
            if last_turn {
                self.say("You have reached the step limit. Call finish now with what you have.");
            }
            let turn = tokio::select! {
                _ = self.io.stop.cancelled() => return End::Stopped,
                _ = tokio::time::sleep_until(self.deadline) => return End::Error(self.out_of_time()),
                t = self.llm.turn(&system, &schema, &offered, &self.messages) => t,
            };
            let turn: Turn = match turn {
                Ok(t) => t,
                // A model that rejects `tools` gets the text protocol instead.
                Err(e)
                    if self.can_fall_back(switched)
                        && e.to_string().to_lowercase().contains("tool") =>
                {
                    self.fall_back(&mut switched);
                    continue;
                }
                Err(e) => return End::Error(e.to_string()),
            };
            self.turns += 1;
            self.usage.add(turn.usage);
            if self.usage.billed() > self.input.limits.max_tokens {
                return End::Error(format!(
                    "The run used its token budget ({} tokens) and stopped. Raise it in Settings, or ask a narrower question.",
                    self.input.limits.max_tokens
                ));
            }
            self.progress();
            let calls: Vec<(String, String, Value)> = turn
                .tool_uses()
                .map(|(id, name, input)| (id.to_string(), name.to_string(), input.clone()))
                .collect();
            if calls.is_empty() && turn.truncated {
                return End::Error(
                    "The model's reply was cut off before it finished. Try a narrower question."
                        .into(),
                );
            }
            if calls.is_empty() {
                // A model that ignores `tools` on its first turn gets the text protocol.
                if self.turns == 1 && self.can_fall_back(switched) {
                    self.fall_back(&mut switched);
                    self.turns = 0;
                    continue;
                }
                self.messages.push(Msg {
                    role: Role::Assistant,
                    blocks: turn.blocks.clone(),
                });
                if !nudged && !last_turn {
                    nudged = true;
                    self.say("Call one of the tools, or finish with your answer.");
                    continue;
                }
                // Take the text as the answer, unchecked by tools.
                let text = turn.text();
                return End::Answer(Box::new(
                    self.answer(&json!({ "answer": text, "confidence": "low" }))
                        .await,
                ));
            }
            self.messages.push(Msg {
                role: Role::Assistant,
                blocks: turn.blocks.clone(),
            });
            let mut results = Vec::new();
            for (id, name, input) in calls {
                if name == tools::FINISH {
                    // Claims about all rows are checked once more, over all of them, before the answer stands.
                    let claims = checks::sweeping_claims(input["answer"].as_str().unwrap_or(""));
                    let cannot = input["cannot_answer"].as_bool().unwrap_or(false);
                    if !self.reviewed
                        && !last_turn
                        && self.queries > 0
                        && !cannot
                        && !claims.is_empty()
                    {
                        self.reviewed = true;
                        let i =
                            self.new_step("review", "Checking claims about all rows".into(), None);
                        let list: Vec<String> = claims.iter().map(|c| format!("- {c}")).collect();
                        let msg = format!(
                            "Before this answer stands, check each sentence below that says something about all, every or no \
                             rows. Run a query over all the relevant rows (not only the subset an earlier query filtered), \
                             and correct any sentence the result doesn't support. Then call finish again.\n{}",
                            list.join("\n")
                        );
                        let (content, is_error) =
                            self.end_step(i, Ok(msg), "asked the AI to check its claims");
                        results.push(Block::ToolResult {
                            id,
                            content,
                            is_error,
                        });
                        continue;
                    }
                    return End::Answer(Box::new(self.answer(&input).await));
                }
                let out = match self.tool(&name, &input).await {
                    Ok(out) => out,
                    Err(Stop) => return End::Stopped,
                };
                results.push(Block::ToolResult {
                    id,
                    content: out.0,
                    is_error: out.1,
                });
                if self.io.stop.is_cancelled() {
                    return End::Stopped;
                }
            }
            self.messages.push(Msg {
                role: Role::User,
                blocks: results,
            });
            if last_turn {
                return End::Error("The run reached its step limit without an answer.".into());
            }
        }
    }

    /// On PostgreSQL, the tables whose row-level security this login can't bypass, as a
    /// warning for the agent and a step for the person. Supabase turns it on for app tables, so
    /// a read-only login there can see empty tables that aren't empty.
    async fn row_security(&mut self) -> Option<String> {
        // Checked once per conversation, not again for each follow-up. A team server's login is
        // its admin's business, and the check would be one more query in their audit log.
        if self.input.kind != Kind::Postgres
            || self.messages.len() > 1
            || matches!(self.pool, DbPool::Team(_))
        {
            return None;
        }
        let sql = "SELECT n.nspname || '.' || c.relname FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace \
                   WHERE c.relkind IN ('r', 'p') AND c.relrowsecurity \
                   AND n.nspname NOT IN ('pg_catalog', 'information_schema') \
                   AND (c.relforcerowsecurity OR pg_get_userbyid(c.relowner) <> current_user) \
                   AND NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = current_user AND (rolsuper OR rolbypassrls)) \
                   ORDER BY 1";
        let r = readonly::run_limited_with(
            self.pool,
            sql,
            ROWS_READ,
            Duration::from_secs(10),
            &self.io.stop,
        )
        .await
        .ok()?;
        let tables: Vec<String> = r
            .rows
            .iter()
            .filter_map(|row| row.first()?.as_str().map(String::from))
            .collect();
        if tables.is_empty() {
            return None;
        }
        let some: Vec<&str> = tables.iter().take(5).map(String::as_str).collect();
        let more = if tables.len() > 5 {
            format!(" and {} more", tables.len() - 5)
        } else {
            String::new()
        };
        let warning = format!(
            "Row-level security is on for {} tables ({}{more}), and this login can't bypass it, so queries on them may \
             return only some rows or none.",
            tables.len(),
            some.join(", ")
        );
        let i = self.new_step(
            "notice",
            "Row-level security limits what this login can see".into(),
            None,
        );
        self.steps[i].checks = vec![warning.clone()];
        self.end_step(i, Ok(warning.clone()), "sent the warning");
        Some(warning)
    }

    fn can_fall_back(&self, switched: bool) -> bool {
        !switched
            && matches!(&*self.llm, Llm::Remote { endpoint, mode: ToolMode::Native, .. } if endpoint.is_openai())
    }

    fn fall_back(&mut self, switched: &mut bool) {
        *switched = true;
        #[cfg_attr(
            not(test),
            allow(irrefutable_let_patterns, reason = "Llm has a test-only variant")
        )]
        if let Llm::Remote { mode, .. } = self.llm {
            *mode = ToolMode::Text;
        }
        // Drop a reply that ignored the tools; the question goes again.
        while self
            .messages
            .last()
            .is_some_and(|m| m.role == Role::Assistant)
        {
            self.messages.pop();
        }
    }

    /// Adds a line of text to the next message to the model.
    fn say(&mut self, text: &str) {
        match self.messages.last_mut() {
            Some(Msg {
                role: Role::User,
                blocks,
            }) => blocks.push(Block::Text(text.into())),
            _ => self.messages.push(Msg::user(text)),
        }
    }

    fn out_of_time(&self) -> String {
        format!(
            "The run reached its time limit ({} seconds) and stopped. The steps so far are below.",
            self.input.limits.run_seconds
        )
    }

    fn progress(&self) {
        (self.io.emit)(Event::Progress {
            run_id: self.input.run_id.clone(),
            turns: self.turns,
            max_turns: self.input.limits.max_turns(),
            queries: self.queries,
            max_queries: self.input.limits.max_queries,
            usage: self.usage,
            elapsed_ms: self.started.elapsed().as_millis(),
        });
    }

    fn new_step(&mut self, tool: &str, text: String, sql: Option<String>) -> usize {
        let step = Step {
            id: self.next_step,
            tool: tool.into(),
            text,
            status: StepStatus::Running,
            sql,
            row_count: None,
            truncated: false,
            elapsed_ms: None,
            columns: Vec::new(),
            rows: Vec::new(),
            sent: None,
            sent_summary: None,
            checks: Vec::new(),
            error: None,
        };
        self.next_step += 1;
        (self.io.emit)(Event::Step {
            run_id: self.input.run_id.clone(),
            step: step.clone(),
        });
        self.steps.push(step);
        self.steps.len() - 1
    }

    /// Ends step `i` and tells the tab. Returns the text for the model, `Step n: ...`.
    fn end_step(
        &mut self,
        i: usize,
        sent: Result<String, String>,
        summary: &str,
    ) -> (String, bool) {
        let step = &mut self.steps[i];
        let (content, is_error) = match sent {
            Ok(s) => {
                step.status = StepStatus::Done;
                (format!("Step {}: {s}", step.id), false)
            }
            Err(e) => {
                step.status = StepStatus::Error;
                step.error = Some(e.clone());
                (format!("Step {}: {e}", step.id), true)
            }
        };
        step.sent = Some(content.clone());
        step.sent_summary = Some(summary.into());
        (self.io.emit)(Event::Step {
            run_id: self.input.run_id.clone(),
            step: step.clone(),
        });
        self.progress();
        (content, is_error)
    }

    /// Waits for the person; the wait doesn't count against the run's time limit.
    async fn wait(&mut self) -> Result<Reply, Stop> {
        let from = Instant::now();
        let reply = tokio::select! {
            _ = self.io.stop.cancelled() => None,
            r = self.io.replies.recv() => r,
        };
        self.deadline += from.elapsed();
        reply.ok_or(Stop)
    }

    async fn tool(&mut self, name: &str, input: &Value) -> Result<(String, bool), Stop> {
        let s = |k: &str| input[k].as_str().unwrap_or("").trim().to_string();
        Ok(match name {
            tools::PLAN => {
                let steps: Vec<String> = input["steps"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|v| v.as_str())
                    .map(|t| self.masker.unmask_text(t))
                    .collect();
                let waiting = !self.input.limits.skip_plan;
                (self.io.emit)(Event::Plan {
                    run_id: self.input.run_id.clone(),
                    steps: steps.clone(),
                    waiting,
                });
                if !waiting {
                    return Ok(("Plan noted. Go ahead.".into(), false));
                }
                match self.wait().await? {
                    Reply::Go { plan: Some(edited) } if edited != steps => {
                        let lines: Vec<String> = edited
                            .iter()
                            .enumerate()
                            .map(|(i, s)| format!("{}. {s}", i + 1))
                            .collect();
                        (
                            format!(
                                "The person changed the plan. Follow this one:\n{}",
                                lines.join("\n")
                            ),
                            false,
                        )
                    }
                    Reply::Answer { text } => {
                        (format!("The person replied to the plan: {text}"), false)
                    }
                    _ => ("The person approved the plan. Go ahead.".into(), false),
                }
            }
            tools::SEARCH => {
                let words = s("words");
                let i = self.new_step(name, format!("Looking for tables about {words}"), None);
                let mut text = readonly::describe(&self.input.schema, Some(&words));
                let words: Vec<&str> = words.split_whitespace().collect();
                let tables: Vec<String> = self
                    .input
                    .schema
                    .tables
                    .iter()
                    .filter(|t| {
                        words.iter().any(|w| {
                            w.len() > 2 && t.name.to_lowercase().contains(&w.to_lowercase())
                        })
                    })
                    .map(Table::qualified_name)
                    .collect();
                let notes = self.notes_about(&tables, &words);
                if !notes.is_empty() {
                    text.push_str(&format!("\nSaved notes:\n- {}", notes.join("\n- ")));
                }
                self.end_step(i, Ok(text), "sent table and column names")
            }
            tools::CHECK_NOTE => self.check_note(&s("note")).await,
            tools::DESCRIBE => {
                let name_in = s("table");
                let i = self.new_step(name, format!("Reading the columns of {name_in}"), None);
                let out = match find_table(&self.input.schema, &name_in) {
                    Some(t) => {
                        let mut text = schema::render_table(t, &self.input.schema.dialect);
                        if let Some(n) = t.row_estimate {
                            text.push_str(&format!("\nAbout {n} rows."));
                        }
                        let notes = self.notes_about(&[t.qualified_name()], &[t.name.as_str()]);
                        if !notes.is_empty() {
                            text.push_str(&format!("\nSaved notes:\n- {}", notes.join("\n- ")));
                        }
                        Ok(text)
                    }
                    None => Err(format!(
                        "There is no table called {name_in}. Use search_schema to find it."
                    )),
                };
                self.end_step(i, out, "sent table and column names")
            }
            tools::PROFILE => {
                self.profile(&s("table"), &s("column"), &s("purpose"))
                    .await?
            }
            tools::QUERY => self.query(&s("sql"), &s("purpose")).await?,
            tools::EXPLAIN => self.explain(&s("sql")).await,
            tools::SHOW if !s("sql").is_empty() => self.show_sql(&s("sql")).await?,
            tools::SHOW => {
                self.show(&s("table"), &s("column"), &s("key_column"), &input["key"])
                    .await?
            }
            tools::ASK => {
                let question = self.masker.unmask_text(&s("question"));
                let choices: Vec<String> = input["choices"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|v| v.as_str())
                    .map(|c| self.masker.unmask_text(c))
                    .collect();
                let i = self.new_step(name, format!("Asked you: {question}"), None);
                let id = self.steps[i].id;
                (self.io.emit)(Event::Question {
                    run_id: self.input.run_id.clone(),
                    step_id: id,
                    question,
                    choices,
                });
                let answer = match self.wait().await? {
                    Reply::Answer { text } => text,
                    _ => "(no answer)".into(),
                };
                self.steps[i].text.push_str(&format!(" — {answer}"));
                // The person's answer may name values; they go to the AI masked like any other.
                let answer = self.masker.remask_text(&answer);
                self.end_step(
                    i,
                    Ok(format!("The person answered: {answer}")),
                    "sent your answer",
                )
            }
            other => (format!("There is no tool called {other}."), true),
        })
    }

    /// The saved notes about any of `tables` (by their anchors), and notes tied to no table
    /// that mention one of `words`, each as the model reads it: with its id and how sure.
    fn notes_about(&self, tables: &[String], words: &[&str]) -> Vec<String> {
        self.input
            .notes
            .iter()
            .filter(|n| {
                if n.anchors.is_empty() {
                    let text = n.text.to_lowercase();
                    words
                        .iter()
                        .any(|w| w.len() > 2 && text.contains(&w.to_lowercase()))
                } else {
                    n.anchors.iter().any(|a| {
                        tables
                            .iter()
                            .any(|t| crate::team::knowledge::names(&a.table, t))
                    })
                }
            })
            .map(|n| {
                let doubt = match &n.suspect {
                    Some(s) => format!("(may be out of date: {}) ", s.reason),
                    None => String::new(),
                };
                format!("[{}] {doubt}{}", note_ref(&n.id), n.text)
            })
            .collect()
    }

    /// Runs a saved note's check (see `crate::notes::run_check`) and records the result.
    async fn check_note(&mut self, reference: &str) -> (String, bool) {
        let wanted = reference
            .trim()
            .trim_matches(|c| c == '[' || c == ']')
            .to_lowercase();
        let found = self
            .input
            .notes
            .iter()
            .position(|n| note_ref(&n.id) == wanted || n.id.to_lowercase() == wanted);
        let i = self.new_step(tools::CHECK_NOTE, "Checking a saved note".to_string(), None);
        let Some(at) = found else {
            let msg = format!("No saved note has the id {reference}. Use an id like n_1a2b3c4d.");
            return self.end_step(i, Err(msg), "sent the error");
        };
        let note = self.input.notes[at].clone();
        self.steps[i].text = format!("Checking the note: {}", note.text);
        let Some(check) = note.check.clone() else {
            return self.end_step(
                i,
                Err("This note has no check. Test what it says with a query instead.".into()),
                "sent the error",
            );
        };
        self.steps[i].sql = Some(check.sql.clone());
        let run =
            crate::notes::run_check(self.pool, &check.sql, &self.input.access.never_send).await;
        if let Some(home) = &self.input.notes_home
            && let Ok(Some(updated)) = home.store.record_check(&note.id, run.clone())
        {
            self.input.notes[at] = updated;
        }
        match (run.ok, run.error) {
            (Some(true), _) => self.end_step(i, Ok("The check passed: the note holds.".into()), "sent the result"),
            (Some(false), _) => self.end_step(
                i,
                Ok("The check failed: the note may no longer be true. Don't rely on it; if you find what's true now, suggest a replacement in finish.".into()),
                "sent the result",
            ),
            (None, e) => self.end_step(
                i,
                Err(format!("The check couldn't run: {}", e.unwrap_or_default())),
                "sent the error",
            ),
        }
    }

    /// After the database said a table or column doesn't exist: reads the schema again (once
    /// per run), reviews the saved notes against it, and says what changed.
    async fn reread_schema(&mut self) -> Option<String> {
        if self.reread {
            return None;
        }
        self.reread = true;
        let fresh = db::introspect(self.pool, self.input.kind).await.ok()?;
        // The run's schema is the one the AI may read (see `never_send::schema_for_ai`); the
        // notes are reviewed against the whole one.
        let shown = crate::never_send::schema_for_ai(
            &fresh,
            &crate::never_send::entries(&self.input.access.never_send),
        )
        .into_owned();
        let before = crate::schema::anchors::Snapshot::of(&self.input.schema);
        let changes =
            crate::schema::anchors::diff(&before, &crate::schema::anchors::Snapshot::of(&shown));
        // The team server reviews shared notes on its own schema reads.
        if let Some(home) = self.input.notes_home.as_ref().filter(|h| h.team.is_none()) {
            let _ = crate::notes::review(
                &home.store,
                &home.connection_id,
                home.database.as_deref(),
                &fresh,
            );
            if let Ok(notes) = home
                .store
                .notes(&home.connection_id, home.database.as_deref())
            {
                self.input.notes = notes;
            }
        }
        self.input.schema = shown;
        if changes.is_empty() {
            return Some("SayQL read the schema again: it hasn't changed.".into());
        }
        let lines: Vec<String> = changes
            .iter()
            .take(20)
            .map(crate::schema::anchors::Change::describe)
            .collect();
        Some(format!(
            "SayQL read the schema again; it changed since the run started:\n- {}\nUse describe_table for the current columns.",
            lines.join("\n- ")
        ))
    }

    /// The SQL with real values in place of tokens, if it may run: one read-only statement
    /// that names no never-send column.
    async fn real_sql(&mut self, sql: &str) -> Result<String, String> {
        let real = self.real_sql_text(sql)?;
        // Never-send columns in any form (whole rows, renames, views on Postgres), and functions
        // that read server files: see never_send.rs.
        let guard = crate::never_send::check(self.pool, &real, &self.input.access.never_send)
            .await
            .map_err(|f| self.masker.remask_text(&f.safe))?;
        self.withheld = guard.withheld;
        Ok(real)
    }

    fn real_sql_text(&self, sql: &str) -> Result<String, String> {
        let kind = self.input.kind;
        let real = self.masker.unmask_sql(sql, kind)?;
        if checks::statements(&real) > 1 {
            return Err("Run one statement at a time.".into());
        }
        // A team server refuses writes itself and audits the attempt, so it gets to see them.
        let team = matches!(self.pool, DbPool::Team(_));
        if !team && !db::is_read_only(&real, kind) {
            return Err(readonly::AGENT_WRITE_REFUSED.into());
        }
        Ok(real)
    }

    async fn run_sql(&mut self, sql: &str) -> Result<QueryResult, String> {
        let secs = Duration::from_secs(self.input.limits.query_seconds);
        let r = readonly::run_limited_checked(self.pool, sql, ROWS_READ, secs, &self.io.stop).await;
        // Database messages can quote values; the model gets a fixed message for the kind of
        // error (see ai/redact.rs).
        let e = match r {
            Ok(v) => return Ok(v),
            Err(e) => e,
        };
        let mut msg = match e.full.as_str() {
            crate::team::WRITE_REFUSED => readonly::AGENT_WRITE_REFUSED.to_string(),
            readonly::STOPPED => readonly::STOPPED.to_string(),
            _ => self.masker.remask_text(&e.safe),
        };
        // A missing table or column: the schema may have changed under the run.
        if crate::notes::says_missing(&e.full)
            && let Some(more) = self.reread_schema().await
        {
            msg.push('\n');
            msg.push_str(&more);
        }
        Err(msg)
    }

    async fn query(&mut self, sql: &str, purpose: &str) -> Result<(String, bool), Stop> {
        let purpose = if purpose.is_empty() {
            "Running a query".to_string()
        } else {
            self.masker.unmask_text(purpose)
        };
        if self.queries >= self.input.limits.max_queries {
            let i = self.new_step(tools::QUERY, purpose, None);
            let msg = format!(
                "The run has used its {} queries. Call finish with what you have.",
                self.input.limits.max_queries
            );
            return Ok(self.end_step(i, Err(msg), "sent the refusal"));
        }
        let real = self.real_sql(sql).await;
        let i = self.new_step(
            tools::QUERY,
            purpose,
            Some(
                real.clone()
                    .unwrap_or_else(|_| self.masker.unmask_text(sql)),
            ),
        );
        let real = match real {
            Ok(r) => r,
            Err(e) => return Ok(self.end_step(i, Err(e), "sent the refusal")),
        };
        // Gentle on production: reading a big table in full needs a filter, or a plan checked first.
        if let Some((table, rows)) = checks::unfiltered_big_scan(&real, &self.input.schema)
            && !self.explained.contains(&real)
            && self.input.kind != Kind::Mssql
        {
            let msg = format!(
                "This query reads all of {table} (about {rows} rows) with no WHERE clause, so it was not run. Add a \
                 filter, or call explain_query with this SQL to check its cost; if the plan is acceptable, run it again."
            );
            return Ok(self.end_step(i, Err(msg), "sent the refusal"));
        }
        self.queries += 1;
        let r = match self.run_sql(&real).await {
            Ok(r) => r,
            Err(e) if e == readonly::STOPPED => return Err(Stop),
            Err(e) => return Ok(self.end_step(i, Err(e), "sent the error")),
        };
        let checks = checks::join_fan_out(&real, &self.input.schema);
        // A personal column under another name ("full_name AS creator") is masked like the column.
        self.masker.learn_aliases(&real);
        self.masker.withhold_positions(&self.withheld);
        let (payload, summary) = self.payload(&r);
        self.fill_step(i, &r, checks.clone());
        let (payload, summary) = self.maybe_skip(i, &r, payload, summary).await?;
        let mut text = payload;
        for c in &checks {
            text.push_str(&format!("\nCheck: {c}"));
        }
        self.results.push(r);
        Ok(self.end_step(i, Ok(text), &summary))
    }

    async fn profile(
        &mut self,
        table: &str,
        column: &str,
        purpose: &str,
    ) -> Result<(String, bool), Stop> {
        let text = if purpose.is_empty() {
            format!("Profiling {table}.{column}")
        } else {
            purpose.to_string()
        };
        let found = find_table(&self.input.schema, table).and_then(|t| {
            let c = t
                .columns
                .iter()
                .find(|c| c.name.eq_ignore_ascii_case(column))?;
            Some((
                schema::render_name(t, &self.input.schema.dialect),
                schema::quote_ident(&c.name, &self.input.schema.dialect),
                c.name.clone(),
            ))
        });
        let Some((t, c, col_name)) = found else {
            let i = self.new_step(tools::PROFILE, text, None);
            return Ok(self.end_step(
                i,
                Err(format!("There is no column {column} in {table}.")),
                "sent the error",
            ));
        };
        if self.queries >= self.input.limits.max_queries {
            let i = self.new_step(tools::PROFILE, text, None);
            let msg = format!(
                "The run has used its {} queries. Call finish with what you have.",
                self.input.limits.max_queries
            );
            return Ok(self.end_step(i, Err(msg), "sent the refusal"));
        }
        let top = match self.input.kind {
            Kind::Mssql => format!(
                "SELECT TOP {TOP_VALUES} {c} AS {c}, COUNT(*) AS n FROM {t} GROUP BY {c} ORDER BY COUNT(*) DESC"
            ),
            _ => format!(
                "SELECT {c} AS {c}, COUNT(*) AS n FROM {t} GROUP BY {c} ORDER BY COUNT(*) DESC LIMIT {TOP_VALUES}"
            ),
        };
        let stats = format!(
            "SELECT COUNT(*) AS total_rows, COUNT(*) - COUNT({c}) AS nulls, COUNT(DISTINCT {c}) AS distinct_values, \
             MIN({c}) AS min_value, MAX({c}) AS max_value FROM {t}"
        );
        // A never-send column isn't profiled at all.
        let never = crate::never_send::entries(&self.input.access.never_send);
        let dialect = crate::never_send::dialect(self.input.kind);
        if let Err(e) = crate::never_send::check_text(&stats, &never, dialect, false) {
            let i = self.new_step(tools::PROFILE, text, None);
            return Ok(self.end_step(i, Err(e), "sent the refusal"));
        }
        // Which never-send entries apply follows the table profiled.
        self.masker.learn_aliases(&stats);
        let i = self.new_step(tools::PROFILE, text, Some(format!("{stats};\n{top}")));
        self.queries += 1;
        let (stats_r, top_r) = match (self.run_sql(&stats).await, self.run_sql(&top).await) {
            (Ok(a), Ok(b)) => (a, b),
            (Err(e), _) | (_, Err(e)) if e == readonly::STOPPED => return Err(Stop),
            (Err(e), _) | (_, Err(e)) => return Ok(self.end_step(i, Err(e), "sent the error")),
        };
        let row = stats_r.rows.first().cloned().unwrap_or_default();
        let get = |k: usize| row.get(k).cloned().unwrap_or(Value::Null);
        let personal = self.masker.is_personal_column(&col_name)
            || [get(3), get(4)]
                .iter()
                .chain(top_r.rows.iter().filter_map(|r| r.first()))
                .any(|v| super::mask::value_kind(v).is_some());
        let numeric_or_date = [get(3), get(4)].iter().all(|v| {
            v.is_null()
                || v.is_number()
                || v.as_str()
                    .is_some_and(|s| s.parse::<f64>().is_ok() || crate::pii::starts_with_date(s))
        });
        let mut out = json!({ "rows": get(0), "nulls": get(1), "distinct": get(2) });
        let summary;
        match self.input.access.results {
            Results::Off => {
                out = json!({ "rows": get(0) });
                summary = "sent row count only".to_string();
            }
            Results::Counts => {
                if numeric_or_date && !personal {
                    out["min"] = get(3);
                    out["max"] = get(4);
                }
                summary = "sent counts and min/max only".to_string();
            }
            Results::Rows => {
                out["min"] = self.masker.mask_value(&col_name, &get(3));
                out["max"] = self.masker.mask_value(&col_name, &get(4));
                let top: Vec<Value> = top_r
                    .rows
                    .iter()
                    .map(|r| json!([self.masker.mask_value(&col_name, &r[0]), r.get(1)]))
                    .collect();
                out["top_values"] = Value::Array(top);
                summary = if personal {
                    "sent counts and top values, masked".into()
                } else {
                    "sent counts and top values".into()
                };
            }
        }
        let mut local = stats_r.clone();
        local.columns.extend(["top values".to_string()]);
        if let Some(r0) = local.rows.first_mut() {
            r0.push(Value::String(
                top_r
                    .rows
                    .iter()
                    .map(|r| {
                        format!(
                            "{} ({})",
                            plain(&r[0]),
                            plain(r.get(1).unwrap_or(&Value::Null))
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(", "),
            ));
        }
        self.fill_step(i, &local, Vec::new());
        let payload = out.to_string();
        let (payload, summary) = self.maybe_skip(i, &local, payload, summary).await?;
        self.results.push(stats_r);
        self.results.push(top_r);
        Ok(self.end_step(i, Ok(payload), &summary))
    }

    /// Runs a query whose first column holds what to show; the model gets tokens, never values.
    async fn show_sql(&mut self, sql: &str) -> Result<(String, bool), Stop> {
        let text = "Looking up a value to show you".to_string();
        let real = self.real_sql(sql).await;
        let i = self.new_step(
            tools::SHOW,
            text,
            Some(
                real.clone()
                    .unwrap_or_else(|_| self.masker.unmask_text(sql)),
            ),
        );
        let real = match real {
            Ok(r) => r,
            Err(e) => return Ok(self.end_step(i, Err(e), "sent the refusal")),
        };
        let r = match self.run_sql(&real).await {
            Ok(r) => r,
            Err(e) if e == readonly::STOPPED => return Err(Stop),
            Err(e) => return Ok(self.end_step(i, Err(e), "sent the error")),
        };
        self.fill_step(i, &r, Vec::new());
        let column = r.columns.first().cloned().unwrap_or_default();
        let tokens: Vec<String> = r
            .rows
            .iter()
            .take(5)
            .filter_map(|row| row.first())
            .filter(|v| !v.is_null())
            .map(|v| self.masker.token_for(&column, v))
            .collect();
        let out = match tokens.len() {
            0 => "The query returned no value.".to_string(),
            1 => format!(
                "{} (shown to the person, hidden from you). Use this token in your answer.",
                tokens[0]
            ),
            n => format!(
                "{n} values: {} (hidden from you). Use these tokens in your answer.",
                tokens.join(", ")
            ),
        };
        self.results.push(r);
        Ok(self.end_step(i, Ok(out), "sent a token only"))
    }

    /// Looks up one value for the answer. The model gets a token for it, never the value.
    async fn show(
        &mut self,
        table: &str,
        column: &str,
        key_column: &str,
        key: &Value,
    ) -> Result<(String, bool), Stop> {
        let dialect = self.input.schema.dialect.clone();
        let found = find_table(&self.input.schema, table).and_then(|t| {
            let col = |name: &str| {
                t.columns
                    .iter()
                    .find(|c| c.name.eq_ignore_ascii_case(name))
                    .map(|c| c.name.clone())
            };
            Some((
                schema::render_name(t, &dialect),
                col(column)?,
                col(key_column)?,
            ))
        });
        let text = format!("Looking up {table}.{column} to show you");
        let Some((t, col, key_col)) = found else {
            let i = self.new_step(tools::SHOW, text, None);
            return Ok(self.end_step(
                i,
                Err(format!(
                    "There is no column {column} or {key_column} in {table}."
                )),
                "sent the error",
            ));
        };
        // A token as the key stands for its real value.
        let key = match key {
            Value::String(s) if s.contains('‹') => Value::String(self.masker.unmask_text(s)),
            other => other.clone(),
        };
        let literal = match super::mask::literal(&key, self.input.kind) {
            Ok(l) if !key.is_null() && !key.is_array() && !key.is_object() => l,
            _ => {
                let i = self.new_step(tools::SHOW, text, None);
                return Ok(self.end_step(
                    i,
                    Err("Give the key as one number or string.".into()),
                    "sent the error",
                ));
            }
        };
        let (c, k) = (
            schema::quote_ident(&col, &dialect),
            schema::quote_ident(&key_col, &dialect),
        );
        let sql = match self.input.kind {
            Kind::Mssql => format!("SELECT TOP 5 {c} AS {c} FROM {t} WHERE {k} = {literal}"),
            _ => format!("SELECT {c} AS {c} FROM {t} WHERE {k} = {literal} LIMIT 5"),
        };
        let i = self.new_step(tools::SHOW, text, Some(sql.clone()));
        let r = match self.run_sql(&sql).await {
            Ok(r) => r,
            Err(e) if e == readonly::STOPPED => return Err(Stop),
            Err(e) => return Ok(self.end_step(i, Err(e), "sent the error")),
        };
        self.fill_step(i, &r, Vec::new());
        let tokens: Vec<String> = r
            .rows
            .iter()
            .filter_map(|row| row.first())
            .map(|v| self.masker.token_for(&col, v))
            .collect();
        let out = match tokens.len() {
            0 => "No row has that key.".to_string(),
            1 => format!(
                "{} (shown to the person, hidden from you). Use this token in your answer.",
                tokens[0]
            ),
            n => format!(
                "{n} rows: {} (hidden from you). Use these tokens in your answer.",
                tokens.join(", ")
            ),
        };
        self.results.push(r);
        Ok(self.end_step(i, Ok(out), "sent a token only"))
    }

    async fn explain(&mut self, sql: &str) -> (String, bool) {
        let real = self.real_sql(sql).await;
        let i = self.new_step(
            tools::EXPLAIN,
            "Checking the query plan".into(),
            Some(real.clone().unwrap_or_else(|_| sql.to_string())),
        );
        let prefix = match self.input.kind {
            Kind::Postgres | Kind::Mysql => "EXPLAIN",
            Kind::Sqlite => "EXPLAIN QUERY PLAN",
            Kind::Mssql => {
                return self.end_step(i, Err("SQL Server plans aren't available here. Keep the query selective; it stops after the time limit.".into()), "sent the refusal");
            }
        };
        let real = match real {
            Ok(r) => r,
            Err(e) => return self.end_step(i, Err(e), "sent the refusal"),
        };
        match self.run_sql(&format!("{prefix} {real}")).await {
            Ok(r) => {
                self.explained.push(real.clone());
                let text = r
                    .rows
                    .iter()
                    .map(|row| row.iter().map(plain).collect::<Vec<_>>().join(" | "))
                    .collect::<Vec<_>>()
                    .join("\n");
                self.fill_step(i, &r, Vec::new());
                let plan = self
                    .masker
                    .remask_text(&format!("{}\n{text}", r.columns.join(" | ")));
                self.end_step(i, Ok(plan), "sent the plan")
            }
            Err(e) => self.end_step(i, Err(e), "sent the error"),
        }
    }

    fn fill_step(&mut self, i: usize, r: &QueryResult, checks: Vec<String>) {
        let step = &mut self.steps[i];
        step.row_count = Some(r.rows.len());
        step.truncated = r.truncated;
        step.elapsed_ms = Some(r.elapsed_ms);
        step.columns = r.columns.clone();
        step.rows = r.rows.clone();
        step.checks = checks;
    }

    /// What the model may see of a result under this connection's setting.
    fn payload(&mut self, r: &QueryResult) -> (String, String) {
        let more = if r.truncated {
            " (more rows exist; SayQL read the first 1,000)"
        } else {
            ""
        };
        match self.input.access.results {
            Results::Off => (
                format!(
                    "{} rows{more}. Columns: {}. No values are sent on this connection.",
                    r.rows.len(),
                    r.columns.join(", ")
                ),
                "sent row count only".into(),
            ),
            Results::Counts => {
                let s: ResultSummary = self.masker.summarize(r);
                (
                    format!(
                        "Summary{more}: {}",
                        serde_json::to_string(&s).unwrap_or_default()
                    ),
                    "sent counts and column summaries".into(),
                )
            }
            Results::Rows => {
                let m = self.masker.mask(r, ROWS_SENT);
                let shown = m.rows.len();
                let mut text = format!("{} rows{more}.", m.row_count);
                text.push_str(&format!(
                    "\nColumns: {}",
                    serde_json::to_string(&m.columns).unwrap_or_default()
                ));
                for row in &m.rows {
                    text.push('\n');
                    text.push_str(&serde_json::to_string(row).unwrap_or_default());
                }
                if m.row_count > shown {
                    text.push_str(&format!("\n({} more rows not sent.)", m.row_count - shown));
                }
                let summary = match m.masked_columns.len() {
                    0 => format!("sent {shown} rows"),
                    1 => format!("sent {shown} rows, 1 column masked"),
                    n => format!("sent {shown} rows, {n} columns masked"),
                };
                (text, summary)
            }
        }
    }

    /// With Ask before send, pauses on the payload; Skip sends the row count only.
    async fn maybe_skip(
        &mut self,
        i: usize,
        r: &QueryResult,
        payload: String,
        summary: String,
    ) -> Result<(String, String), Stop> {
        if !self.input.access.ask_before_send || self.input.access.results == Results::Off {
            return Ok((payload, summary));
        }
        let step_id = self.steps[i].id;
        (self.io.emit)(Event::Send {
            run_id: self.input.run_id.clone(),
            step_id,
            payload: format!("Step {step_id}: {payload}"),
        });
        Ok(match self.wait().await? {
            Reply::Send => (payload, summary),
            _ => (
                format!(
                    "{} rows. The person chose not to send this result.",
                    r.rows.len()
                ),
                "skipped: sent row count only".into(),
            ),
        })
    }

    /// The notes `finish` suggests: new facts, and fixes to saved notes (`replaces` or `remove`
    /// with a note's id), each optionally with a check. Checked as the model wrote them: a token
    /// means the note holds a value from the data. Where the notes are kept, they wait with the
    /// other suggestions for the person (returns true); otherwise they're offered in the answer.
    /// On a team connection with shared notes, they go to the team server instead.
    async fn suggest_notes(&self, raw: &Value) -> (Vec<String>, bool) {
        let items: Vec<&Value> = raw.as_array().into_iter().flatten().take(3).collect();
        let Some(home) = &self.input.notes_home else {
            let texts = items
                .iter()
                .filter_map(|v| v.as_str().or_else(|| v["text"].as_str()))
                .filter_map(|n| clean_note(n).ok())
                .collect();
            return (texts, false);
        };
        if let Some(team) = &home.team {
            return (self.suggest_team_notes(home, team, &items).await, true);
        }
        let mut texts = Vec::new();
        for v in items {
            let (text, replaces, remove, check, evidence) = match v {
                Value::String(s) => (s.as_str(), None, false, None, None),
                o => (
                    o["text"].as_str().unwrap_or(""),
                    o["replaces"].as_str().filter(|r| !r.trim().is_empty()),
                    o["remove"].as_bool().unwrap_or(false),
                    o["check"].as_str(),
                    o["evidence"].as_str(),
                ),
            };
            let change = match (replaces, remove) {
                (Some(r), true) => Suggest::Remove(r),
                (Some(r), false) => Suggest::Replace(r),
                (None, _) => Suggest::Add,
            };
            // Evidence that quotes a value (a token) is replaced, not allowed to sink the note.
            let evidence = evidence
                .filter(|e| crate::store::note_values::note_value(e).is_none())
                .unwrap_or("Found while answering a question in Investigate.");
            let made = home.store.suggest_checked_note_change(
                &home.connection_id,
                home.database.as_deref(),
                text,
                evidence,
                NOTES_SOURCE,
                change,
                check,
            );
            if let Ok(s) = made {
                texts.push(s.text);
            }
        }
        (texts, true)
    }

    /// Sends `finish`'s notes to the team server as suggestions, with the same checks as local
    /// ones: no values from the data in the text, the check or the evidence. Returns the texts
    /// the server took.
    async fn suggest_team_notes(
        &self,
        home: &NotesHome,
        team: &TeamNotes,
        items: &[&Value],
    ) -> Vec<String> {
        let Some(server_id) = home.connection_id.strip_prefix(crate::team::ID_PREFIX) else {
            return Vec::new();
        };
        let mut texts = Vec::new();
        for v in items {
            let (text, replaces, remove, check, evidence) = match v {
                Value::String(s) => (s.as_str(), None, false, None, None),
                o => (
                    o["text"].as_str().unwrap_or(""),
                    o["replaces"].as_str().filter(|r| !r.trim().is_empty()),
                    o["remove"].as_bool().unwrap_or(false),
                    o["check"].as_str().filter(|c| !c.trim().is_empty()),
                    o["evidence"].as_str(),
                ),
            };
            let Ok(text) = clean_note(text) else {
                continue;
            };
            let check = match check.map(crate::store::agent::clean_check) {
                Some(Ok(c)) => Some(c),
                Some(Err(_)) => continue,
                None => None,
            };
            // The model knows notes by their short id: send the server the note's own id.
            let replaces = replaces.map(|r| {
                let wanted = r
                    .trim()
                    .trim_matches(|c| c == '[' || c == ']')
                    .to_lowercase();
                self.input
                    .notes
                    .iter()
                    .find(|n| note_ref(&n.id) == wanted || n.id.to_lowercase() == wanted)
                    .map(|n| n.id.clone())
                    .unwrap_or(wanted)
            });
            if remove && replaces.is_none() {
                continue;
            }
            let evidence = evidence
                .filter(|e| crate::store::note_values::note_value(e).is_none())
                .unwrap_or("Found while answering a question in Investigate.");
            let suggestion = crate::team::knowledge::Suggestion {
                text: text.clone(),
                evidence: evidence
                    .chars()
                    .take(crate::store::suggestions::MAX_EVIDENCE)
                    .collect(),
                check,
                replaces,
                remove,
                source: Some(NOTES_SOURCE.into()),
            };
            match team.client.suggest_knowledge(server_id, &suggestion).await {
                Ok(()) => texts.push(text),
                Err(e) => eprintln!("sayql: the team server didn't take a suggested note: {e}"),
            }
        }
        team.cache.forget(&home.connection_id);
        texts
    }

    async fn answer(&self, input: &Value) -> Answer {
        let text = self
            .masker
            .unmask_text(input["answer"].as_str().unwrap_or("").trim());
        let ids: Vec<u32> = self.steps.iter().map(|s| s.id).collect();
        let strings = |k: &str| -> Vec<String> {
            input[k]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|v| v.as_str())
                .map(|s| self.masker.unmask_text(s.trim()))
                .filter(|s| !s.is_empty())
                .collect()
        };
        let cannot = input["cannot_answer"].as_bool().unwrap_or(false);
        // Numbers are checked against what the steps returned, the question, and the SQL that ran.
        let results: Vec<&[Vec<Value>]> = self.steps.iter().map(|s| s.rows.as_slice()).collect();
        let mut known: Vec<f64> = ids.iter().map(|i| f64::from(*i)).collect();
        known.extend(
            self.steps
                .iter()
                .filter_map(|s| s.row_count.map(|n| n as f64)),
        );
        let context = format!(
            "{} {}",
            self.input.question,
            self.steps
                .iter()
                .filter_map(|s| s.sql.clone())
                .collect::<Vec<_>>()
                .join(" ")
        );
        let check = |t: &str| {
            if cannot {
                Vec::new()
            } else {
                checks::unbacked_numbers(t, &results, &known, &context)
            }
        };
        let mut sentences: Vec<Sentence> = input["citations"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|c| {
                let t = self.masker.unmask_text(c["sentence"].as_str()?.trim());
                let steps = c["steps"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|s| s.as_u64())
                    .map(|s| s as u32)
                    .filter(|s| ids.contains(s))
                    .collect();
                Some(Sentence {
                    unverified: check(&t),
                    text: t,
                    steps,
                })
            })
            .collect();
        if sentences.is_empty() && !text.is_empty() {
            sentences.push(Sentence {
                unverified: check(&text),
                text: text.clone(),
                steps: Vec::new(),
            });
        }
        // Checked as the model wrote them: a token means the note holds a value from the data.
        let (notes, notes_queued) = self.suggest_notes(&input["notes"]).await;
        let notes_shared = notes_queued
            && self
                .input
                .notes_home
                .as_ref()
                .is_some_and(|h| h.team.is_some());
        let proposed_sql = input["proposed_sql"]
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| {
                self.masker
                    .unmask_sql(s, self.input.kind)
                    .unwrap_or_else(|_| self.masker.unmask_text(s))
            });
        let confidence = match input["confidence"].as_str() {
            Some(c @ ("high" | "medium" | "low")) => c.to_string(),
            _ => "medium".into(),
        };
        let proposed_changes_data = proposed_sql
            .as_deref()
            .is_some_and(|s| !db::is_read_only(s, self.input.kind));
        Answer {
            proposed_changes_data,
            unverified: check(&text),
            text,
            sentences,
            confidence,
            assumptions: strings("assumptions"),
            notes,
            notes_queued,
            notes_shared,
            proposed_sql,
            cannot_answer: cannot,
            model: self.llm.model_name().to_string(),
            usage: self.usage,
        }
    }
}

/// The run was stopped while it waited.
struct Stop;

impl AgentSettings {
    pub fn max_turns(&self) -> u32 {
        self.max_steps
    }
}

fn find_table<'a>(schema: &'a Schema, name: &str) -> Option<&'a Table> {
    let clean: String = name.chars().filter(|c| !"\"`[]".contains(*c)).collect();
    schema
        .tables
        .iter()
        .find(|t| t.qualified_name().eq_ignore_ascii_case(&clean))
        .or_else(|| {
            schema
                .tables
                .iter()
                .find(|t| t.name.eq_ignore_ascii_case(&clean))
        })
}

fn plain(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => "NULL".into(),
        other => other.to_string(),
    }
}
