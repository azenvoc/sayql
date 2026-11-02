//! Reads tables, columns, keys, enum values, row estimates, comments and CHECK constraints
//! into a [`Schema`].

mod mssql;
mod mysql;
mod postgres;
mod sqlite;

use super::{DbPool, Kind};
use crate::error::AppResult;
use crate::schema::{Column, ForeignKey, Schema, Table};
use std::collections::{HashMap, HashSet};

pub async fn introspect(pool: &DbPool, kind: Kind) -> AppResult<Schema> {
    let tables = match pool {
        DbPool::Pg(p) => postgres::introspect(p).await?,
        DbPool::My(p) => mysql::introspect(p).await?,
        DbPool::Lite(p) => sqlite::introspect(&p.ro).await?,
        DbPool::Ms(p) => mssql::introspect(p).await?,
        DbPool::Team(t) => return t.schema().await,
    };
    Ok(Schema {
        dialect: kind.dialect().to_string(),
        tables,
    })
}

/// (schema, table)
type Key = (String, String);

/// What the database itself says about its tables: comments and CHECK constraints.
#[derive(Default)]
struct Described {
    /// (schema, table, column) → comment
    columns: HashMap<(String, String, String), String>,
    tables: HashMap<Key, String>,
    /// Each starts with `CHECK`.
    checks: HashMap<Key, Vec<String>>,
}

impl Described {
    fn column(&mut self, s: String, t: String, c: String, comment: String) {
        if !comment.trim().is_empty() {
            self.columns.insert((s, t, c), comment);
        }
    }

    fn table(&mut self, s: String, t: String, comment: String) {
        if !comment.trim().is_empty() {
            self.tables.insert((s, t), comment);
        }
    }

    /// `def` with or without its leading `CHECK`.
    fn check(&mut self, s: String, t: String, def: &str) {
        let def = def.trim();
        if def.is_empty() {
            return;
        }
        let def = if def
            .get(..5)
            .is_some_and(|w| w.eq_ignore_ascii_case("check"))
        {
            def.to_string()
        } else {
            format!("CHECK {def}")
        };
        self.checks.entry((s, t)).or_default().push(def);
    }
}

fn assemble(
    cols: Vec<(String, String, String, String, bool, String)>,
    pks: Vec<(String, String, String)>,
    fks: Vec<(String, String, String, String, String, String)>,
    enums: HashMap<String, Vec<String>>,
    rows: HashMap<Key, i64>,
    mut described: Described,
    qualify_refs: bool,
) -> Vec<Table> {
    let pk: HashSet<(String, String, String)> = pks.into_iter().collect();
    let mut fk_map: HashMap<Key, Vec<ForeignKey>> = HashMap::new();
    for (s, t, c, rs, rt, rc) in fks {
        let ref_table = if qualify_refs && rs != "public" {
            format!("{rs}.{rt}")
        } else {
            rt
        };
        fk_map.entry((s, t)).or_default().push(ForeignKey {
            column: c,
            ref_table,
            ref_column: rc,
        });
    }
    let mut order: Vec<Key> = Vec::new();
    let mut map: HashMap<Key, Table> = HashMap::new();
    for (s, t, c, ty, nullable, kind) in cols {
        let key = (s.clone(), t.clone());
        let table = map.entry(key.clone()).or_insert_with(|| {
            order.push(key.clone());
            Table {
                schema: Some(s.clone()),
                name: t.clone(),
                kind: if kind.contains("MATERIALIZED") {
                    "materialized view".into()
                } else if kind.contains("VIEW") {
                    "view".into()
                } else {
                    "table".into()
                },
                columns: vec![],
                foreign_keys: fk_map.remove(&key).unwrap_or_default(),
                row_estimate: rows.get(&key).copied(),
                comment: described.tables.remove(&key),
                checks: described.checks.remove(&key).unwrap_or_default(),
            }
        });
        let enum_values = enums.get(&ty).cloned().or_else(|| parse_mysql_enum(&ty));
        let col_key = (s, t, c);
        table.columns.push(Column {
            primary_key: pk.contains(&col_key),
            comment: described.columns.remove(&col_key),
            name: col_key.2,
            data_type: if enum_values.is_some() && ty.starts_with("enum(") {
                "enum".into()
            } else {
                ty
            },
            nullable,
            enum_values,
        });
    }
    order.into_iter().filter_map(|k| map.remove(&k)).collect()
}

fn parse_mysql_enum(ty: &str) -> Option<Vec<String>> {
    let inner = ty.strip_prefix("enum(")?.strip_suffix(')')?;
    Some(
        inner
            .split("','")
            .map(|v| v.trim_matches('\'').replace("''", "'"))
            .collect(),
    )
}
