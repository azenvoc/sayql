//! The English → SQL prompt.

use serde::Deserialize;

const INSTRUCTIONS: &str = "You turn a person's plain-English request into one {DIALECT} query for the database described below. \
The person is usually not a developer, so infer what they mean.

Rules:
- Reply with the SQL only: no Markdown fences, no explanation before or after. Your reply must begin with \
the SQL itself or a `-- ` comment line, never with prose. \
The one exception: when the message asks you to explain a query, reply in plain English as it describes.
- Comments: at most one `-- ` line above a statement, and only to state an assumption, written as the assumption itself \
(\"-- Assumes the earliest-created user is the owner\"). Never narrate your reasoning or mention earlier queries.
- Write {DIALECT} syntax, using only tables and columns from the schema. Quote identifiers only when needed.
- One query when a single result answers everything naturally (\"all users, with the owner marked\", \"customers and their \
order counts\"). Separate statements, in the order asked, each ending with `;`, when the answers have different shapes and \
merging them would bend one (a count and a list, a total and a breakdown, unrelated tables): the app runs them in turn and \
shows each result in its own tab.
- Prefer readable results: pick useful columns instead of SELECT * when joining, use clear aliases, and join to show names rather than bare IDs.
- When the request lists rows without saying how many, return at most 100 rows using {DIALECT} syntax (TOP 100 in SQL Server, LIMIT 100 elsewhere). When it asks for all of them (\"all users\", \"every order\"), add no limit: the app caps the rows it shows itself. Never limit aggregates the person asked for in full.
- Interpret relative dates (\"last week\", \"this year\") relative to the current date in SQL, not a hard-coded date.
- Write INSERT, UPDATE, DELETE or DDL only when the person explicitly asks to change data or structure.
- If something is ambiguous, make the most reasonable choice and state the assumption in one leading `-- ` comment line.
- If the request cannot be answered from this schema, reply with only a `-- ` comment saying what is missing.

Follow-ups: the message may include the query the person ran last. When the new request reads as a change to it \
(\"remove the middle name\", \"only active ones\", \"sort by date\", \"add their email\", \"show 50\"), return that query \
with the change applied and everything else kept as it was. When the new request is a different question, ignore it \
and write a fresh query, without saying so.

Saved instructions: the message may end with instructions the person saved for requests like this one. Follow them \
unless they conflict with the rules above, which always win.";

/// The query the person ran last in the same tab, for follow-up requests.
#[derive(Debug, Clone, Deserialize)]
pub struct PreviousQuery {
    pub question: String,
    pub sql: String,
}

/// What the model is asked to do with the request.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Task {
    /// Write SQL while the person types, building on the previous query for follow-ups.
    Draft {
        previous: Option<PreviousQuery>,
        /// `/followup`: the request is a change to `previous`, whatever it sounds like.
        #[serde(default, rename = "forceFollowUp")]
        force_follow_up: bool,
    },
    /// Correct a query the database rejected, or (empty `error`) one the person says is wrong.
    Fix { sql: String, error: String },
    /// Review the current query and return the best version of it.
    Improve { sql: String },
    /// Describe the current query in plain English. The reply is prose, not SQL.
    Explain { sql: String },
}

impl Task {
    /// Fixes and improvements go to the stronger model; drafts to the fast one.
    pub fn needs_expert(&self) -> bool {
        matches!(self, Task::Fix { .. } | Task::Improve { .. })
    }

    /// Drafts must be fast; fixes and reviews are worth a little more thought.
    pub(super) fn effort(&self) -> &'static str {
        if self.needs_expert() { "medium" } else { "low" }
    }

    /// SQL that comes with the request, for picking the relevant tables of a large schema.
    pub fn context_sql(&self) -> &str {
        match self {
            Task::Draft { previous, .. } => previous.as_ref().map_or("", |p| p.sql.as_str()),
            Task::Fix { sql, .. } | Task::Improve { sql } | Task::Explain { sql } => sql,
        }
    }
}

fn tagged(tag: &str, body: &str) -> String {
    format!("<{tag}>\n{}\n</{tag}>", body.trim())
}

/// The instructions, for the given SQL dialect.
pub(super) fn instructions(dialect: &str) -> String {
    INSTRUCTIONS.replace("{DIALECT}", dialect)
}

/// The schema, as sent after the instructions. Identical across keystrokes for a
/// connection, so providers with prompt caching only process the short request fresh.
pub(super) fn schema_block(dialect: &str, schema_text: &str) -> String {
    format!("<schema dialect=\"{dialect}\">\n{schema_text}\n</schema>")
}

/// The user turn for a task, ending with the saved instructions of a skill, if any.
pub(super) fn user_message(question: &str, task: &Task, skill: Option<&str>) -> String {
    let message = task_message(question, task);
    match skill.map(str::trim).filter(|s| !s.is_empty()) {
        None => message,
        Some(text) => format!("{message}\n\n{}", tagged("saved_instructions", text)),
    }
}

fn task_message(question: &str, task: &Task) -> String {
    let request = if question.trim().is_empty() {
        "(not stated; infer it from the query)"
    } else {
        question.trim()
    };
    match task {
        Task::Draft {
            previous,
            force_follow_up,
        } => match previous.as_ref().filter(|p| !p.sql.trim().is_empty()) {
            None => question.to_string(),
            Some(p) => {
                let asked = if p.question.trim().is_empty() {
                    String::new()
                } else {
                    format!("Request: {}\n", p.question.trim())
                };
                let label = if *force_follow_up {
                    "Change to apply to the previous query (keep everything else as it was)"
                } else {
                    "New request"
                };
                format!(
                    "<previous_query>\n{asked}SQL:\n{}\n</previous_query>\n\n{label}: {question}",
                    p.sql.trim()
                )
            }
        },
        Task::Fix { sql, error } if error.trim().is_empty() => format!(
            "{}\n\n{}\n\nThe person says this query is wrong. Find the problem using the request and the schema, and \
             return the corrected query. Keep everything that was not part of the problem.",
            tagged("request", request),
            tagged("current_query", sql)
        ),
        Task::Fix { sql, error } => format!(
            "{}\n\n{}\n\n{}\n\nThe database rejected this query. Find the cause in the error and the schema, and return \
             the corrected query for the request. Keep everything that was not part of the problem.",
            tagged("request", request),
            tagged("failed_query", sql),
            tagged(
                "database_error",
                &crate::safety::redact::database_error(error, sql)
            )
        ),
        Task::Improve { sql } => format!(
            "{}\n\n{}\n\nReview this query against the request and the schema: joins, filters, date logic, grouping, \
             NULL handling and the columns shown. Return the best query for the request. If it is already right, \
             return it unchanged.",
            tagged("request", request),
            tagged("current_query", sql)
        ),
        Task::Explain { sql } => {
            let focus = if question.trim().is_empty() {
                String::new()
            } else {
                format!("\n\nAlso answer this about it: {}", question.trim())
            };
            format!(
                "{}\n\nExplain what this query does for someone who doesn't read SQL: what it returns, from which \
                 tables, and any filters, grouping or sorting. Reply in plain English, in at most five short sentences. \
                 No SQL, no Markdown headings.{focus}",
                tagged("current_query", sql)
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draft(previous: Option<PreviousQuery>) -> Task {
        Task::Draft {
            previous,
            force_follow_up: false,
        }
    }

    #[test]
    fn a_new_question_is_sent_as_is() {
        assert_eq!(
            user_message("top 10 customers", &draft(None), None),
            "top 10 customers"
        );
        let empty = PreviousQuery {
            question: "x".into(),
            sql: "  ".into(),
        };
        assert_eq!(
            user_message("top 10 customers", &draft(Some(empty)), None),
            "top 10 customers"
        );
    }

    #[test]
    fn a_follow_up_carries_the_previous_query() {
        let prev = PreviousQuery {
            question: "list a few clients from account 42".into(),
            sql: "SELECT TOP 10 first_name, middle_name, last_name FROM clients WHERE account_id = 42;".into(),
        };
        let m = user_message("remove the middle name", &draft(Some(prev)), None);
        assert!(
            m.starts_with(
                "<previous_query>\nRequest: list a few clients from account 42\nSQL:\nSELECT TOP 10"
            ),
            "{m}"
        );
        assert!(
            m.ends_with("</previous_query>\n\nNew request: remove the middle name"),
            "{m}"
        );
    }

    #[test]
    fn fixes_carry_the_error_and_go_to_the_expert() {
        let t = Task::Fix {
            sql: "SELECT nme FROM clients".into(),
            error: "Invalid column name 'nme'.".into(),
        };
        let m = user_message("list clients", &t, None);
        assert!(
            m.contains("<failed_query>\nSELECT nme FROM clients\n</failed_query>"),
            "{m}"
        );
        assert!(
            m.contains("<database_error>\nInvalid column name 'nme'.\n</database_error>"),
            "{m}"
        );
        let t = Task::Fix {
            sql: "INSERT INTO users VALUES (1)".into(),
            error: "Duplicate entry 'ann@example.com' for key 'users.email'".into(),
        };
        let m = user_message("add a user", &t, None);
        assert!(
            m.contains("Duplicate entry '…' for key '…'") && !m.contains("ann@"),
            "{m}"
        );
        assert!(t.needs_expert());
        assert_eq!(t.effort(), "medium");
        assert!(!draft(None).needs_expert());
    }

    #[test]
    fn tasks_deserialize_from_the_ui() {
        let t: Task = serde_json::from_str(r#"{"kind":"improve","sql":"SELECT 1"}"#).unwrap();
        assert!(matches!(t, Task::Improve { .. }));
        let t: Task = serde_json::from_str(r#"{"kind":"draft","previous":null}"#).unwrap();
        assert!(
            user_message(
                "",
                &Task::Improve {
                    sql: "SELECT 1".into()
                },
                None
            )
            .contains("(not stated")
        );
        assert!(matches!(
            t,
            Task::Draft {
                previous: None,
                force_follow_up: false
            }
        ));
    }

    #[test]
    fn a_forced_follow_up_says_to_change_the_previous_query() {
        let prev = PreviousQuery {
            question: "".into(),
            sql: "SELECT id FROM orders;".into(),
        };
        let t: Task = serde_json::from_value(serde_json::json!({
            "kind": "draft", "previous": { "question": "", "sql": prev.sql }, "forceFollowUp": true
        }))
        .unwrap();
        let m = user_message("count them", &t, None);
        assert!(
            m.starts_with("<previous_query>\nSQL:\nSELECT id FROM orders;"),
            "{m}"
        );
        assert!(
            m.ends_with(
                "Change to apply to the previous query (keep everything else as it was): count them"
            ),
            "{m}"
        );
    }

    #[test]
    fn a_skill_adds_its_instructions_to_any_task() {
        let pii = Some("Never select email, phone or address columns.");
        let m = user_message("list customers", &draft(None), pii);
        assert_eq!(
            m,
            "list customers\n\n<saved_instructions>\nNever select email, phone or address columns.\n</saved_instructions>"
        );
        let m = user_message(
            "",
            &Task::Improve {
                sql: "SELECT * FROM customers".into(),
            },
            pii,
        );
        assert!(m.ends_with("</saved_instructions>"), "{m}");
        assert_eq!(
            user_message("list customers", &draft(None), Some("  ")),
            "list customers"
        );
    }

    #[test]
    fn explain_asks_for_prose_with_the_fast_model() {
        let t: Task = serde_json::from_str(r#"{"kind":"explain","sql":"SELECT 1"}"#).unwrap();
        let m = user_message("why the join?", &t, None);
        assert!(
            m.starts_with("<current_query>\nSELECT 1\n</current_query>"),
            "{m}"
        );
        assert!(
            m.contains("plain English") && m.ends_with("Also answer this about it: why the join?"),
            "{m}"
        );
        assert!(!t.needs_expert());
        assert_eq!(t.context_sql(), "SELECT 1");
    }

    #[test]
    fn a_fix_without_a_database_error_says_the_person_found_it_wrong() {
        let t = Task::Fix {
            sql: "SELECT 1".into(),
            error: "".into(),
        };
        let m = user_message("count orders", &t, None);
        assert!(
            m.contains("The person says this query is wrong") && !m.contains("database_error"),
            "{m}"
        );
        assert!(t.needs_expert());
    }
}
