use crate::schema::{Column, ForeignKey, Table};
use sqlx::SqlitePool;
use std::collections::HashMap;

pub(super) async fn introspect(p: &SqlitePool) -> Result<Vec<Table>, sqlx::Error> {
    let objects: Vec<(String, String, Option<String>)> = sqlx::query_as(
        "SELECT name, type, sql FROM sqlite_master WHERE type IN ('table', 'view') AND name NOT LIKE 'sqlite_%' ORDER BY name",
    )
    .fetch_all(p)
    .await?;
    let mut tables = Vec::new();
    for (name, kind, sql) in objects {
        let mut described = match &sql {
            Some(sql) if kind == "table" => describe(sql),
            _ => Described::default(),
        };
        let info: Vec<(String, String, bool, i64)> =
            sqlx::query_as("SELECT name, type, \"notnull\", pk FROM pragma_table_info(?)")
                .bind(&name)
                .fetch_all(p)
                .await?;
        let fks: Vec<(String, String, Option<String>)> =
            sqlx::query_as("SELECT \"from\", \"table\", \"to\" FROM pragma_foreign_key_list(?)")
                .bind(&name)
                .fetch_all(p)
                .await?;
        let row_estimate = if kind == "table" {
            let q = format!("SELECT COUNT(*) FROM \"{}\"", name.replace('"', "\"\""));
            sqlx::query_scalar::<_, i64>(sqlx::AssertSqlSafe(q))
                .fetch_one(p)
                .await
                .ok()
        } else {
            None
        };
        tables.push(Table {
            schema: None,
            name,
            kind,
            columns: info
                .into_iter()
                .map(|(n, ty, notnull, pk)| Column {
                    data_type: if ty.is_empty() {
                        "any".into()
                    } else {
                        ty.to_lowercase()
                    },
                    nullable: !notnull,
                    primary_key: pk > 0,
                    enum_values: None,
                    comment: described.columns.remove(&n.to_lowercase()),
                    name: n,
                })
                .collect(),
            foreign_keys: fks
                .into_iter()
                .map(|(from, table, to)| ForeignKey {
                    column: from,
                    ref_table: table,
                    ref_column: to.unwrap_or_else(|| "rowid".into()),
                })
                .collect(),
            row_estimate,
            comment: described.table,
            checks: described.checks,
        });
    }
    Ok(tables)
}

/// What a table's CREATE TABLE text says beyond its columns. SQLite has no COMMENT, but it keeps
/// the statement as written, so `--` and `/* */` comments are there: one beside a column
/// definition (or on its own line just before it) describes that column, and one on the line
/// that opens the column list describes the table.
#[derive(Default, Debug)]
struct Described {
    table: Option<String>,
    /// Lowercased column name → comment
    columns: HashMap<String, String>,
    /// Each `CHECK (…)` clause, column or table level, in order.
    checks: Vec<String>,
}

/// One definition between the top-level commas of the column list.
struct Def {
    text: String,
    comments: Vec<String>,
}

fn describe(sql: &str) -> Described {
    let c: Vec<char> = sql.chars().collect();
    let mut out = Described::default();
    let mut table_notes: Vec<String> = Vec::new();
    let mut defs: Vec<Def> = Vec::new();
    let mut cur = String::new();
    let mut cur_notes: Vec<String> = Vec::new();
    // Comments on their own line, waiting for the definition that follows them.
    let mut pending: Vec<String> = Vec::new();
    let (mut depth, mut line, mut open_line, mut last_end_line) = (0usize, 0usize, 0usize, None);
    let mut i = 0;
    while i < c.len() {
        let ch = c[i];
        let comment = match (ch, c.get(i + 1)) {
            ('-', Some('-')) => {
                let end = (i..c.len()).find(|&j| c[j] == '\n').unwrap_or(c.len());
                Some((c[i + 2..end].iter().collect::<String>(), end))
            }
            ('/', Some('*')) => {
                let end = (i + 2..c.len().saturating_sub(1))
                    .find(|&j| c[j] == '*' && c[j + 1] == '/')
                    .map_or(c.len(), |j| j + 2);
                let body_end = if end == c.len() { end } else { end - 2 };
                Some((c[i + 2..body_end.max(i + 2)].iter().collect(), end))
            }
            _ => None,
        };
        if let Some((text, end)) = comment {
            let text = text.trim().to_string();
            if !text.is_empty() {
                if depth == 0 || (defs.is_empty() && cur.trim().is_empty() && line == open_line) {
                    table_notes.push(text);
                } else if !cur.trim().is_empty() {
                    cur_notes.push(text);
                } else if last_end_line == Some(line) {
                    if let Some(d) = defs.last_mut() {
                        d.comments.push(text);
                    }
                } else {
                    pending.push(text);
                }
            }
            line += c[i..end].iter().filter(|&&x| x == '\n').count();
            i = end;
            continue;
        }
        if matches!(ch, '\'' | '"' | '`' | '[') {
            let end = quoted_end(&c, i);
            if depth > 0 {
                cur.extend(&c[i..end]);
            }
            line += c[i..end].iter().filter(|&&x| x == '\n').count();
            i = end;
            continue;
        }
        match ch {
            '\n' => {
                line += 1;
                if depth > 0 {
                    cur.push(' ');
                }
            }
            '(' => {
                depth += 1;
                if depth == 1 {
                    open_line = line;
                } else {
                    cur.push(ch);
                }
            }
            ')' if depth > 0 => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
                cur.push(ch);
            }
            ',' if depth == 1 => {
                finish(&mut defs, &mut cur, &mut cur_notes, &mut pending);
                last_end_line = Some(line);
            }
            _ if depth > 0 => cur.push(ch),
            _ => {}
        }
        i += 1;
    }
    finish(&mut defs, &mut cur, &mut cur_notes, &mut pending);

    if !table_notes.is_empty() {
        out.table = Some(table_notes.join(" "));
    }
    for d in defs {
        out.checks.extend(checks_in(&d.text));
        let first = d.text.trim_start();
        let word: String = first
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect::<String>()
            .to_ascii_uppercase();
        let constraint = matches!(
            word.as_str(),
            "CONSTRAINT" | "PRIMARY" | "UNIQUE" | "CHECK" | "FOREIGN"
        );
        if !constraint && !d.comments.is_empty() {
            out.columns
                .insert(column_name(first).to_lowercase(), d.comments.join(" "));
        }
    }
    out
}

fn finish(
    defs: &mut Vec<Def>,
    cur: &mut String,
    notes: &mut Vec<String>,
    pending: &mut Vec<String>,
) {
    if cur.trim().is_empty() {
        return;
    }
    let mut comments = std::mem::take(pending);
    comments.append(notes);
    defs.push(Def {
        text: std::mem::take(cur),
        comments,
    });
}

/// The index just past the quoted text starting at `i`: '…', "…" and `…` (where a doubled quote
/// is part of the text), or […].
fn quoted_end(c: &[char], i: usize) -> usize {
    let close = if c[i] == '[' { ']' } else { c[i] };
    let mut j = i + 1;
    while j < c.len() {
        if c[j] == close {
            if close != ']' && c.get(j + 1) == Some(&close) {
                j += 2;
                continue;
            }
            return j + 1;
        }
        j += 1;
    }
    c.len()
}

/// A definition's column name, unquoted.
fn column_name(def: &str) -> String {
    let c: Vec<char> = def.chars().collect();
    match c.first() {
        Some(q @ ('"' | '`' | '\'' | '[')) => {
            let end = quoted_end(&c, 0);
            let inner: String = c[1..end.saturating_sub(1).max(1)].iter().collect();
            if *q == '[' {
                inner
            } else {
                inner.replace(&format!("{q}{q}"), &q.to_string())
            }
        }
        _ => c
            .iter()
            .take_while(|c| !c.is_whitespace() && **c != '(')
            .collect(),
    }
}

/// Every `CHECK (…)` in a definition (with its comments already removed), balanced over nested
/// parentheses and quoted text.
fn checks_in(def: &str) -> Vec<String> {
    let c: Vec<char> = def.chars().collect();
    let word = |j: usize| c.get(j).is_some_and(|x| x.is_alphanumeric() || *x == '_');
    let mut out = Vec::new();
    let mut i = 0;
    while i < c.len() {
        if matches!(c[i], '\'' | '"' | '`' | '[') {
            i = quoted_end(&c, i);
            continue;
        }
        let is_check = i + 5 <= c.len()
            && c[i..i + 5]
                .iter()
                .collect::<String>()
                .eq_ignore_ascii_case("check")
            && (i == 0 || !word(i - 1))
            && !word(i + 5);
        if !is_check {
            i += 1;
            continue;
        }
        let mut j = i + 5;
        while j < c.len() && c[j].is_whitespace() {
            j += 1;
        }
        if c.get(j) != Some(&'(') {
            i = j;
            continue;
        }
        let start = j;
        let mut depth = 0;
        while j < c.len() {
            match c[j] {
                '\'' | '"' | '`' | '[' => {
                    j = quoted_end(&c, j);
                    continue;
                }
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                _ => {}
            }
            j += 1;
        }
        let end = (j + 1).min(c.len());
        out.push(format!(
            "CHECK {}",
            c[start..end].iter().collect::<String>()
        ));
        i = end;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checks_keep_nested_parentheses_and_quoted_commas() {
        let d = describe(
            r#"CREATE TABLE "odd (name" (
              id INTEGER PRIMARY KEY,
              code TEXT CHECK (length(code) IN (3, 4) AND code NOT LIKE '%),(%'),
              "check" TEXT, -- a column named check, with no CHECK
              note TEXT DEFAULT 'check (x)',
              lo INT, hi INT,
              CONSTRAINT range_ok CHECK ((lo < hi) OR (lo IS NULL AND hi IS NULL)),
              check(hi <> 'a '')'' b')
            )"#,
        );
        assert_eq!(
            d.checks,
            [
                "CHECK (length(code) IN (3, 4) AND code NOT LIKE '%),(%')",
                "CHECK ((lo < hi) OR (lo IS NULL AND hi IS NULL))",
                "CHECK (hi <> 'a '')'' b')",
            ]
        );
        assert_eq!(
            d.columns.get("check").map(String::as_str),
            Some("a column named check, with no CHECK")
        );
        assert_eq!(d.columns.len(), 1);
        assert_eq!(d.table, None);
    }

    #[test]
    fn comments_belong_to_the_column_beside_or_below_them() {
        let d = describe(
            "CREATE TABLE orders -- One row per order\n(\n\
               id INTEGER PRIMARY KEY, -- The order number\n\
               -- p: pending, s: shipped (see docs, page 3)\n\
               status TEXT NOT NULL,\n\
               [Total Price] REAL /* in USD, tax included */ CHECK (\"Total Price\" >= 0),\n\
               shipped_at TEXT\n\
               -- trailing note\n\
             ) STRICT -- after the table",
        );
        assert_eq!(d.table.as_deref(), Some("One row per order"));
        assert_eq!(d.columns["id"], "The order number");
        assert_eq!(
            d.columns["status"],
            "p: pending, s: shipped (see docs, page 3)"
        );
        assert_eq!(d.columns["total price"], "in USD, tax included");
        assert_eq!(d.columns["shipped_at"], "trailing note");
        assert_eq!(d.checks, ["CHECK (\"Total Price\" >= 0)"]);

        let opening_line = describe("CREATE TABLE t ( -- Things\n  a INT\n)");
        assert_eq!(opening_line.table.as_deref(), Some("Things"));
        assert!(opening_line.columns.is_empty());
    }

    #[test]
    fn plain_tables_describe_nothing() {
        let d = describe("CREATE TABLE t (a INT PRIMARY KEY, b TEXT NOT NULL REFERENCES u(id))");
        assert!(d.table.is_none() && d.columns.is_empty() && d.checks.is_empty());
        // Broken or unusual text gives what it can, never a panic.
        for sql in [
            "CREATE TABLE t (a INT CHECK (a > 0",
            "CREATE TABLE t (a '",
            "",
            "CREATE TABLE t AS SELECT 1",
            "/*",
            "--",
        ] {
            let _ = describe(sql);
        }
    }

    #[tokio::test]
    async fn the_demo_database_describes_itself() {
        let path = std::env::temp_dir().join(format!("sayql-demo-{}.sqlite", uuid::Uuid::new_v4()));
        crate::db::ensure_demo(&path).await.expect("seed");
        let pool = SqlitePool::connect(&format!("sqlite://{}", path.display()))
            .await
            .unwrap();
        let tables = introspect(&pool).await.unwrap();
        pool.close().await;
        let _ = std::fs::remove_file(&path);
        let t = |n: &str| tables.iter().find(|t| t.name == n).unwrap();
        let col = |t: &Table, n: &str| t.columns.iter().find(|c| c.name == n).unwrap().clone();
        let orders = t("orders");
        assert!(
            orders
                .comment
                .as_deref()
                .unwrap()
                .starts_with("One row per order")
        );
        assert!(
            col(orders, "status")
                .comment
                .unwrap()
                .contains("refunded: the money was returned")
        );
        assert_eq!(orders.checks.len(), 1);
        assert!(orders.checks[0].starts_with("CHECK (status IN ('pending'"));
        assert_eq!(col(orders, "id").comment, None);
        assert!(
            col(t("products"), "price")
                .comment
                .unwrap()
                .contains("US dollars")
        );
        assert_eq!(t("reviews").checks, ["CHECK (rating BETWEEN 1 AND 5)"]);
        assert!(t("categories").comment.is_none() && t("categories").checks.is_empty());
    }
}
