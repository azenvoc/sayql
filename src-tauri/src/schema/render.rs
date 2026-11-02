//! Compact one-line-per-table text for the AI prompt.

use super::{ForeignKey, Schema, Table};
use std::collections::{HashMap, HashSet};

/// Above this size we send only the tables relevant to the question instead of the
/// whole schema. Below it, sending everything keeps the prompt prefix identical across
/// keystrokes so it stays in the prompt cache, which is what keeps generation fast.
const FULL_SCHEMA_CHAR_BUDGET: usize = 300_000;
const RELEVANT_TABLE_LIMIT: usize = 30;
/// Tables that reference a focus table, included so the model can join to them.
const REFERENCING_TABLE_LIMIT: usize = 15;
/// Longest comment or CHECK we show, in characters.
const NOTE_CHAR_LIMIT: usize = 200;

/// Words that must be quoted to be used as names. Matches `quoteIdent` in src/features/schema/sql.ts.
const RESERVED: &[&str] = &[
    "user",
    "order",
    "group",
    "table",
    "select",
    "where",
    "from",
    "limit",
    "offset",
    "desc",
    "asc",
    "key",
    "index",
    "check",
    "default",
    "column",
    "values",
    "references",
    "primary",
    "case",
    "end",
];

/// `ident` as it must be written in `dialect`'s SQL: quoted only when it needs it (symbols,
/// spaces, reserved words, and in Postgres, which folds unquoted names to lowercase, capitals).
/// The model copies names from the schema text, so they must work as written.
pub fn quote_ident(ident: &str, dialect: &str) -> String {
    let postgres = dialect == "PostgreSQL";
    let plain_char = |c: char| {
        c == '_'
            || c.is_ascii_digit()
            || if postgres {
                c.is_ascii_lowercase()
            } else {
                c.is_ascii_alphabetic()
            }
    };
    let plain =
        ident.chars().next().is_some_and(|c| !c.is_ascii_digit()) && ident.chars().all(plain_char);
    if plain && !RESERVED.contains(&ident.to_ascii_lowercase().as_str()) {
        return ident.to_string();
    }
    match dialect {
        "MySQL" => format!("`{}`", ident.replace('`', "``")),
        "SQL Server" => format!("[{}]", ident.replace(']', "]]")),
        _ => format!("\"{}\"", ident.replace('"', "\"\"")),
    }
}

/// The table's name as written in a query: `schema.table`, each part quoted when needed.
pub fn render_name(t: &Table, dialect: &str) -> String {
    match &t.schema {
        Some(s) if s != "public" => format!(
            "{}.{}",
            quote_ident(s, dialect),
            quote_ident(&t.name, dialect)
        ),
        _ => quote_ident(&t.name, dialect),
    }
}

/// A foreign key's target (`schema.table`, or `table` where tables aren't qualified), quoted
/// like `render_name`.
fn render_ref(ref_table: &str, dialect: &str) -> String {
    match ref_table.split_once('.') {
        Some((s, t)) if dialect == "PostgreSQL" || dialect == "SQL Server" => {
            format!("{}.{}", quote_ident(s, dialect), quote_ident(t, dialect))
        }
        _ => quote_ident(ref_table, dialect),
    }
}

/// A comment or CHECK on one line, cut to [`NOTE_CHAR_LIMIT`]. `None` when it's empty.
fn one_line(text: &str) -> Option<String> {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.is_empty() {
        return None;
    }
    if flat.chars().count() <= NOTE_CHAR_LIMIT {
        return Some(flat);
    }
    let cut: String = flat.chars().take(NOTE_CHAR_LIMIT - 1).collect();
    Some(format!("{}…", cut.trim_end()))
}

/// A database comment as a `/* … */` block, so commas in it can't be mistaken for the next
/// column.
fn comment(text: &Option<String>) -> Option<String> {
    one_line(text.as_deref()?).map(|t| format!("/* {} */", t.replace("*/", "* /")))
}

/// One line per table, e.g.
/// `orders [table, ~1000 rows](id integer PK, customer_id integer -> customers.id, status text)`
///
/// The database's own comments and CHECK constraints are added when it has them:
/// `orders [table, ~1000 rows /* One row per order */](status text /* p = pending */) CHECK (total >= 0)`.
/// A table without them renders exactly as it always has, which keeps the prompt cached.
pub fn render_table(t: &Table, dialect: &str) -> String {
    let fk: HashMap<&str, &ForeignKey> = t
        .foreign_keys
        .iter()
        .map(|f| (f.column.as_str(), f))
        .collect();
    let cols: Vec<String> = t
        .columns
        .iter()
        .map(|c| {
            let mut s = format!("{} {}", quote_ident(&c.name, dialect), c.data_type);
            if c.primary_key {
                s.push_str(" PK");
            }
            if !c.nullable && !c.primary_key {
                s.push_str(" NOT NULL");
            }
            if let Some(f) = fk.get(c.name.as_str()) {
                s.push_str(&format!(
                    " -> {}.{}",
                    render_ref(&f.ref_table, dialect),
                    quote_ident(&f.ref_column, dialect)
                ));
            }
            if let Some(vals) = &c.enum_values {
                s.push_str(&format!(" {{{}}}", vals.join("|")));
            }
            if let Some(note) = comment(&c.comment) {
                s.push(' ');
                s.push_str(&note);
            }
            s
        })
        .collect();
    let rows = t
        .row_estimate
        .filter(|n| *n >= 0)
        .map(|n| format!(", ~{n} rows"))
        .unwrap_or_default();
    let note = comment(&t.comment)
        .map(|n| format!(" {n}"))
        .unwrap_or_default();
    let mut line = format!(
        "{} [{}{}{}]({})",
        render_name(t, dialect),
        t.kind,
        rows,
        note,
        cols.join(", ")
    );
    for check in t.checks.iter().filter_map(|c| one_line(c)) {
        line.push(' ');
        line.push_str(&check);
    }
    line
}

pub fn render_full(schema: &Schema) -> String {
    schema
        .tables
        .iter()
        .map(|t| render_table(t, &schema.dialect))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The schema text to put in the prompt for this question.
/// Returns the full schema when it fits, otherwise the most relevant tables
/// plus the tables they reference.
pub fn schema_for_prompt(schema: &Schema, full: &str, question: &str) -> String {
    if full.len() <= FULL_SCHEMA_CHAR_BUDGET {
        return full.to_string();
    }
    let picked = relevant_tables(schema, question);
    if picked.is_empty() {
        // Nothing matched: send the table list without columns so the model can still orient.
        let names: Vec<String> = schema
            .tables
            .iter()
            .map(|t| render_name(t, &schema.dialect))
            .collect();
        return format!(
            "(schema too large to include in full; table names only)\n{}",
            names.join(", ")
        );
    }
    let mut out = String::from("(only the tables most relevant to the question are listed)\n");
    out.push_str(
        &picked
            .iter()
            .map(|&i| render_table(&schema.tables[i], &schema.dialect))
            .collect::<Vec<_>>()
            .join("\n"),
    );
    out
}

/// The tables whose names and columns share words with the question, most relevant first,
/// plus the tables they reference; in schema order.
pub fn relevant_tables(schema: &Schema, question: &str) -> Vec<usize> {
    let words: HashSet<String> = tokenize(question)
        .into_iter()
        .flat_map(|w| [singular(&w), w])
        .collect();
    let mut scored: Vec<(usize, i64)> = schema
        .tables
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let mut score = 0i64;
            for w in tokenize(&t.name) {
                if words.contains(&w) || words.contains(&singular(&w)) {
                    score += 10;
                }
            }
            for c in &t.columns {
                for w in tokenize(&c.name) {
                    if words.contains(&w) {
                        score += 2;
                    }
                }
            }
            // The database's comments: each matching word counts once per table, and short
            // words ("the", "of", "id") don't count at all.
            let comment_words = |text: &Option<String>| -> HashSet<String> {
                text.as_deref()
                    .map(tokenize)
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|w| w.chars().count() >= 4)
                    .collect()
            };
            let hits = |found: HashSet<String>| {
                found
                    .iter()
                    .filter(|w| words.contains(*w) || words.contains(&singular(w)))
                    .count() as i64
            };
            score += 3 * hits(comment_words(&t.comment));
            score += hits(
                t.columns
                    .iter()
                    .flat_map(|c| comment_words(&c.comment))
                    .collect(),
            );
            (i, score)
        })
        .filter(|(_, s)| *s > 0)
        .collect();
    scored.sort_by_key(|&(_, s)| std::cmp::Reverse(s));

    let mut picked: Vec<usize> = scored
        .iter()
        .take(RELEVANT_TABLE_LIMIT)
        .map(|(i, _)| *i)
        .collect();
    // Pull in referenced tables so joins are possible.
    let by_name: HashMap<String, usize> = schema
        .tables
        .iter()
        .enumerate()
        .map(|(i, t)| (t.qualified_name(), i))
        .collect();
    let mut seen: HashSet<usize> = picked.iter().copied().collect();
    for i in picked.clone() {
        for f in &schema.tables[i].foreign_keys {
            if let Some(&j) = by_name.get(&f.ref_table)
                && seen.insert(j)
            {
                picked.push(j);
            }
        }
    }
    picked.sort();
    picked
}

/// The schema text when the person picked tables for the request: those tables, the tables
/// they reference, and (a few of) the tables that reference them. `None` when none of the
/// names exist, so the caller falls back to the usual schema.
pub fn schema_with_focus(schema: &Schema, focus: &[String]) -> Option<String> {
    let wanted: HashSet<&str> = focus.iter().map(String::as_str).collect();
    let focused: Vec<usize> = (0..schema.tables.len())
        .filter(|&i| wanted.contains(schema.tables[i].qualified_name().as_str()))
        .collect();
    if focused.is_empty() {
        return None;
    }
    let by_name: HashMap<String, usize> = schema
        .tables
        .iter()
        .enumerate()
        .map(|(i, t)| (t.qualified_name(), i))
        .collect();
    let mut related: Vec<usize> = Vec::new();
    let mut seen: HashSet<usize> = focused.iter().copied().collect();
    for &i in &focused {
        for f in &schema.tables[i].foreign_keys {
            if let Some(&j) = by_name.get(&f.ref_table)
                && seen.insert(j)
            {
                related.push(j);
            }
        }
    }
    let focus_names: HashSet<String> = focused
        .iter()
        .map(|&i| schema.tables[i].qualified_name())
        .collect();
    let referencing = (0..schema.tables.len())
        .filter(|j| !seen.contains(j))
        .filter(|&j| {
            schema.tables[j]
                .foreign_keys
                .iter()
                .any(|f| focus_names.contains(&f.ref_table))
        })
        .take(REFERENCING_TABLE_LIMIT)
        .collect::<Vec<_>>();
    related.extend(referencing);

    let render = |ids: &[usize]| {
        ids.iter()
            .map(|&i| render_table(&schema.tables[i], &schema.dialect))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let mut out = format!(
        "(The person selected these tables for this request: build the query on them.)\n{}",
        render(&focused)
    );
    if !related.is_empty() {
        out.push_str(&format!(
            "\n\n(Related tables, for joins only when the request needs them.)\n{}",
            render(&related)
        ));
    }
    Some(out)
}

fn tokenize(s: &str) -> Vec<String> {
    s.split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() > 1)
        .map(|w| w.to_lowercase())
        .collect()
}

fn singular(w: &str) -> String {
    w.strip_suffix('s').unwrap_or(w).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::Column;

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

    #[test]
    fn names_are_quoted_the_way_each_dialect_needs() {
        for d in ["PostgreSQL", "MySQL", "SQLite", "SQL Server"] {
            assert_eq!(quote_ident("customer_id", d), "customer_id");
            assert_eq!(quote_ident("t2", d), "t2");
        }
        assert_eq!(quote_ident("FirstName", "PostgreSQL"), "\"FirstName\"");
        assert_eq!(quote_ident("FirstName", "SQL Server"), "FirstName");
        assert_eq!(quote_ident("FirstName", "MySQL"), "FirstName");
        assert_eq!(quote_ident("first name", "PostgreSQL"), "\"first name\"");
        assert_eq!(quote_ident("first name", "MySQL"), "`first name`");
        assert_eq!(quote_ident("first name", "SQL Server"), "[first name]");
        assert_eq!(quote_ident("first name", "SQLite"), "\"first name\"");
        assert_eq!(quote_ident("order", "PostgreSQL"), "\"order\"");
        assert_eq!(quote_ident("User", "SQL Server"), "[User]");
        assert_eq!(quote_ident("zoë", "MySQL"), "`zoë`");
        assert_eq!(quote_ident("2024_sales", "SQLite"), "\"2024_sales\"");
        assert_eq!(quote_ident("a\"b", "PostgreSQL"), "\"a\"\"b\"");
        assert_eq!(quote_ident("a]b", "SQL Server"), "[a]]b]");
        assert_eq!(quote_ident("a`b", "MySQL"), "`a``b`");
    }

    #[test]
    fn the_schema_text_uses_names_as_they_must_be_written() {
        let t = Table {
            schema: Some("Sales Ops".into()),
            name: "Order Lines".into(),
            kind: "table".into(),
            columns: vec![
                Column {
                    name: "id".into(),
                    data_type: "integer".into(),
                    nullable: false,
                    primary_key: true,
                    ..Default::default()
                },
                Column {
                    name: "CustomerId".into(),
                    data_type: "integer".into(),
                    nullable: true,
                    primary_key: false,
                    ..Default::default()
                },
            ],
            foreign_keys: vec![ForeignKey {
                column: "CustomerId".into(),
                ref_table: "crm.Customers".into(),
                ref_column: "Id".into(),
            }],
            row_estimate: Some(3),
            ..Default::default()
        };
        assert_eq!(
            render_table(&t, "PostgreSQL"),
            r#""Sales Ops"."Order Lines" [table, ~3 rows](id integer PK, "CustomerId" integer -> crm."Customers"."Id")"#
        );
        assert_eq!(
            render_table(&t, "SQL Server"),
            "[Sales Ops].[Order Lines] [table, ~3 rows](id integer PK, CustomerId integer -> crm.Customers.Id)"
        );
        let mysql = Table {
            schema: None,
            foreign_keys: vec![],
            ..t
        };
        assert_eq!(
            render_table(&mysql, "MySQL"),
            "`Order Lines` [table, ~3 rows](id integer PK, CustomerId integer)"
        );
    }

    #[test]
    fn the_database_comments_and_checks_are_shown_on_one_line() {
        let plain = Table {
            schema: None,
            name: "orders".into(),
            kind: "table".into(),
            columns: vec![
                Column {
                    name: "id".into(),
                    data_type: "integer".into(),
                    primary_key: true,
                    ..Default::default()
                },
                Column {
                    name: "status".into(),
                    data_type: "text".into(),
                    ..Default::default()
                },
            ],
            row_estimate: Some(3),
            ..Default::default()
        };
        // Byte for byte what it was before comments existed, so the cached prompt stays valid.
        let before = "orders [table, ~3 rows](id integer PK, status text NOT NULL)";
        assert_eq!(render_table(&plain, "SQLite"), before);
        let blank = Table {
            comment: Some("  \n ".into()),
            checks: vec![" ".into()],
            ..plain.clone()
        };
        assert_eq!(render_table(&blank, "SQLite"), before);

        let mut t = plain.clone();
        t.comment = Some("One row per order.\nSee order_items.".into());
        t.columns[1].comment = Some("p = pending, s = shipped */ sneaky".into());
        t.checks = vec![
            "CHECK (status IN ('p', 's'))".into(),
            "CHECK\n  (id > 0)".into(),
        ];
        assert_eq!(
            render_table(&t, "SQLite"),
            "orders [table, ~3 rows /* One row per order. See order_items. */](id integer PK, \
             status text NOT NULL /* p = pending, s = shipped * / sneaky */) \
             CHECK (status IN ('p', 's')) CHECK (id > 0)"
        );

        // Long ones are cut, on a character boundary.
        t.columns[1].comment = Some("é".repeat(500));
        let text = render_table(&t, "SQLite");
        let shown = text
            .split("/* ")
            .nth(2)
            .unwrap()
            .split(" */")
            .next()
            .unwrap();
        assert_eq!(shown.chars().count(), NOTE_CHAR_LIMIT);
        assert!(shown.ends_with('…'));
    }

    #[test]
    fn comments_help_find_the_relevant_tables() {
        let mut ledger = table("gl_entries", &[]);
        ledger.comment = Some("Refunds and payments, one row per movement".into());
        let mut misc = table("misc", &[]);
        misc.columns[0].comment = Some("The invoice number".into());
        let schema = Schema {
            dialect: "SQLite".into(),
            tables: vec![table("person", &[]), ledger, misc],
        };
        assert_eq!(relevant_tables(&schema, "total refunds last month"), [1]);
        assert_eq!(relevant_tables(&schema, "list invoices"), [2]);
        // Short words in comments ("one", "per") don't pull a table in.
        assert!(relevant_tables(&schema, "one per row").is_empty());
    }

    #[test]
    fn focus_keeps_picked_tables_and_their_neighbours() {
        let schema = Schema {
            dialect: "SQLite".into(),
            tables: vec![
                table("person", &[]),
                table("app_user", &[("person_id", "person")]),
                table("login", &[("user_id", "app_user")]),
                table("invoice", &[]),
            ],
        };
        let text = schema_with_focus(&schema, &["app_user".into()]).unwrap();
        let (picked, related) = text.split_once("(Related tables").unwrap();
        assert!(
            picked.contains("app_user [table]") && !picked.contains("person [table]"),
            "{text}"
        );
        // Referenced (person) and referencing (login) tables come along; unrelated ones don't.
        assert!(
            related.contains("person [table]") && related.contains("login [table]"),
            "{text}"
        );
        assert!(!text.contains("invoice"), "{text}");
        assert!(schema_with_focus(&schema, &["nope".into()]).is_none());
        assert!(schema_with_focus(&schema, &[]).is_none());
    }
}
