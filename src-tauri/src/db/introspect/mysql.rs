use super::{Described, assemble};
use crate::schema::Table;
use sqlx::MySqlPool;
use std::collections::HashMap;

pub(super) async fn introspect(p: &MySqlPool) -> Result<Vec<Table>, sqlx::Error> {
    // CAST AS CHAR: some information_schema columns come back as binary strings on MySQL 8.
    let rows: Vec<(String, String, String, String, bool, String, String)> = sqlx::query_as(
        r#"SELECT CAST(c.TABLE_SCHEMA AS CHAR), CAST(c.TABLE_NAME AS CHAR), CAST(c.COLUMN_NAME AS CHAR),
                  CAST(c.COLUMN_TYPE AS CHAR), c.IS_NULLABLE = 'YES', CAST(t.TABLE_TYPE AS CHAR),
                  CAST(c.COLUMN_COMMENT AS CHAR)
           FROM information_schema.COLUMNS c
           JOIN information_schema.TABLES t ON t.TABLE_SCHEMA = c.TABLE_SCHEMA AND t.TABLE_NAME = c.TABLE_NAME
           WHERE c.TABLE_SCHEMA = DATABASE()
           ORDER BY c.TABLE_NAME, c.ORDINAL_POSITION"#,
    )
    .fetch_all(p)
    .await?;
    let mut described = Described::default();
    let mut cols = Vec::with_capacity(rows.len());
    for (s, t, c, ty, nullable, kind, comment) in rows {
        described.column(s.clone(), t.clone(), c.clone(), comment);
        cols.push((s, t, c, ty, nullable, kind));
    }
    let pks: Vec<(String, String, String)> = sqlx::query_as(
        r#"SELECT CAST(TABLE_SCHEMA AS CHAR), CAST(TABLE_NAME AS CHAR), CAST(COLUMN_NAME AS CHAR)
           FROM information_schema.KEY_COLUMN_USAGE
           WHERE TABLE_SCHEMA = DATABASE() AND CONSTRAINT_NAME = 'PRIMARY'"#,
    )
    .fetch_all(p)
    .await?;
    let fks: Vec<(String, String, String, String, String, String)> = sqlx::query_as(
        r#"SELECT CAST(TABLE_SCHEMA AS CHAR), CAST(TABLE_NAME AS CHAR), CAST(COLUMN_NAME AS CHAR),
                  CAST(REFERENCED_TABLE_SCHEMA AS CHAR), CAST(REFERENCED_TABLE_NAME AS CHAR), CAST(REFERENCED_COLUMN_NAME AS CHAR)
           FROM information_schema.KEY_COLUMN_USAGE
           WHERE TABLE_SCHEMA = DATABASE() AND REFERENCED_TABLE_NAME IS NOT NULL"#,
    )
    .fetch_all(p)
    .await?;
    let est: Vec<(String, String, Option<i64>, String, String)> = sqlx::query_as(
        r#"SELECT CAST(TABLE_SCHEMA AS CHAR), CAST(TABLE_NAME AS CHAR), CAST(TABLE_ROWS AS SIGNED),
                  CAST(TABLE_TYPE AS CHAR), CAST(TABLE_COMMENT AS CHAR)
           FROM information_schema.TABLES WHERE TABLE_SCHEMA = DATABASE()"#,
    )
    .fetch_all(p)
    .await?;
    let mut rows = HashMap::new();
    for (s, t, n, kind, comment) in est {
        // A view's TABLE_COMMENT is just the word "VIEW".
        if kind != "VIEW" {
            described.table(s.clone(), t.clone(), comment);
        }
        if let Some(n) = n {
            rows.insert((s, t), n);
        }
    }
    // CHECK constraints are listed from MySQL 8.0.16 and MariaDB 10.2; older servers have none
    // to read. MariaDB names a column's check after the column, so the same name can be on
    // several tables: its CHECK_CONSTRAINTS has TABLE_NAME, MySQL's needs the join.
    let mariadb: Result<Vec<(String, String, String)>, _> = sqlx::query_as(
        r#"SELECT CAST(CONSTRAINT_SCHEMA AS CHAR), CAST(TABLE_NAME AS CHAR), CAST(CHECK_CLAUSE AS CHAR)
           FROM information_schema.CHECK_CONSTRAINTS
           WHERE CONSTRAINT_SCHEMA = DATABASE() ORDER BY CONSTRAINT_NAME"#,
    )
    .fetch_all(p)
    .await;
    let checks = match mariadb {
        Ok(c) => c,
        Err(_) => sqlx::query_as(
            r#"SELECT CAST(tc.TABLE_SCHEMA AS CHAR), CAST(tc.TABLE_NAME AS CHAR), CAST(cc.CHECK_CLAUSE AS CHAR)
               FROM information_schema.CHECK_CONSTRAINTS cc
               JOIN information_schema.TABLE_CONSTRAINTS tc
                 ON tc.CONSTRAINT_SCHEMA = cc.CONSTRAINT_SCHEMA AND tc.CONSTRAINT_NAME = cc.CONSTRAINT_NAME
               WHERE tc.CONSTRAINT_TYPE = 'CHECK' AND tc.TABLE_SCHEMA = DATABASE()
               ORDER BY tc.CONSTRAINT_NAME"#,
        )
        .fetch_all(p)
        .await
        .unwrap_or_default(),
    };
    for (s, t, def) in checks {
        described.check(s, t, &without_charsets(&def));
    }
    let mut tables = assemble(cols, pks, fks, HashMap::new(), rows, described, false);
    // Everything is in one database; drop the qualifier so names stay short.
    for t in &mut tables {
        t.schema = None;
    }
    Ok(tables)
}

/// MySQL writes the strings in a CHECK with their character set (`_utf8mb4'paid'`); the
/// AI only needs `'paid'`.
fn without_charsets(def: &str) -> String {
    let c: Vec<char> = def.chars().collect();
    let mut out = String::with_capacity(def.len());
    let mut i = 0;
    while i < c.len() {
        match c[i] {
            // Copy quoted text and names as they are.
            q @ ('\'' | '`' | '"') => {
                let mut j = i + 1;
                while j < c.len() {
                    if c[j] == '\\' && q == '\'' {
                        j += 1;
                    } else if c[j] == q {
                        if c.get(j + 1) == Some(&q) {
                            j += 1;
                        } else {
                            break;
                        }
                    }
                    j += 1;
                }
                let end = (j + 1).min(c.len());
                out.extend(&c[i..end]);
                i = end;
            }
            '_' if i == 0 || !(c[i - 1].is_alphanumeric() || c[i - 1] == '_') => {
                let name = (i + 1..c.len())
                    .find(|&j| !c[j].is_ascii_alphanumeric())
                    .unwrap_or(c.len());
                if name > i + 1 && c.get(name) == Some(&'\'') {
                    i = name;
                } else {
                    out.push('_');
                    i += 1;
                }
            }
            ch => {
                out.push(ch);
                i += 1;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn check_strings_lose_their_character_set() {
        assert_eq!(
            super::without_charsets("(`status` in (_utf8mb4'p',_utf8mb4'_x''s'))"),
            "(`status` in ('p','_x''s'))"
        );
        assert_eq!(
            super::without_charsets("(`my_col` > 0 and `a_b` <> _latin1'x')"),
            "(`my_col` > 0 and `a_b` <> 'x')"
        );
    }
}
