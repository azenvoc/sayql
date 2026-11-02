//! Which part of the schema a request sends to the model.

use super::Task;
use crate::schema::{self, Schema};

/// What schema the model sees. Drafts with picked tables see just those and their neighbours
/// (smaller and faster); fixes and improvements always see the usual schema, so a narrow
/// pick can't make the right query impossible.
pub fn schema_text(
    schema: &Schema,
    full: &str,
    question: &str,
    task: &Task,
    focus: &[String],
) -> String {
    if !task.needs_expert()
        && let Some(text) = schema::schema_with_focus(schema, focus)
    {
        return text;
    }
    // A follow-up like "remove the middle name" names no table; the SQL that comes with it does.
    let relevance = format!("{question} {} {}", task.context_sql(), focus.join(" "));
    schema::schema_for_prompt(schema, full, &relevance)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{Column, ForeignKey, Table};

    fn table(name: &str, fks: &[(&str, &str)]) -> Table {
        Table {
            schema: None,
            name: name.into(),
            kind: "table".into(),
            columns: vec![Column {
                name: "id".into(),
                data_type: "int".into(),
                nullable: false,
                primary_key: true,
                ..Default::default()
            }],
            foreign_keys: fks
                .iter()
                .map(|(col, to)| ForeignKey {
                    column: (*col).into(),
                    ref_table: (*to).into(),
                    ref_column: "id".into(),
                })
                .collect(),
            row_estimate: None,
            ..Default::default()
        }
    }

    fn schema() -> Schema {
        Schema {
            dialect: "SQLite".into(),
            tables: vec![
                table("person", &[]),
                table("app_user", &[("person_id", "person")]),
                table("invoice", &[]),
            ],
        }
    }

    fn draft() -> Task {
        Task::Draft {
            previous: None,
            force_follow_up: false,
        }
    }

    #[test]
    fn a_draft_with_picked_tables_sees_only_those_and_their_neighbours() {
        let s = schema();
        let full = schema::render_full(&s);
        let text = schema_text(&s, &full, "list users", &draft(), &["app_user".into()]);
        assert!(
            text.contains("app_user") && text.contains("person"),
            "{text}"
        );
        assert!(!text.contains("invoice"), "{text}");
    }

    #[test]
    fn without_a_usable_pick_a_draft_sees_the_full_schema() {
        let s = schema();
        let full = schema::render_full(&s);
        assert_eq!(schema_text(&s, &full, "list users", &draft(), &[]), full);
        assert_eq!(
            schema_text(&s, &full, "list users", &draft(), &["nope".into()]),
            full
        );
    }

    #[test]
    fn fixes_and_improvements_ignore_the_pick() {
        let s = schema();
        let full = schema::render_full(&s);
        let fix = Task::Fix {
            sql: "select 1".into(),
            error: String::new(),
        };
        let improve = Task::Improve {
            sql: "select 1".into(),
        };
        for task in [fix, improve] {
            assert_eq!(
                schema_text(&s, &full, "", &task, &["app_user".into()]),
                full
            );
        }
    }

    #[test]
    fn a_large_schema_is_narrowed_by_the_sql_that_comes_with_the_request() {
        let s = schema();
        // Over the full-schema budget, so only relevant tables are sent.
        let full = "x".repeat(400_000);
        let fix = Task::Fix {
            sql: "select * from invoice".into(),
            error: "boom".into(),
        };
        let text = schema_text(&s, &full, "it fails", &fix, &[]);
        assert!(
            text.contains("invoice") && !text.contains("app_user"),
            "{text}"
        );
    }
}
