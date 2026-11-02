//! What a database note is about, and whether that still holds.
//!
//! A note's anchors are the tables and columns it names, found by matching its words against
//! the schema when it's saved. Each column anchor keeps a fingerprint of the column (type,
//! nullable, key, enum values, reference), so a later schema shows whether the note's subject
//! was dropped, renamed or changed. Snapshots of the whole schema, kept between loads, tell a
//! rename from a drop: a column that disappeared while one with the same fingerprint appeared.

use super::{Column, Schema, Table};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

/// Most anchors kept per note.
const MAX_ANCHORS: usize = 10;

/// Column names too common to say which table a note means on their own.
const GENERIC: &[&str] = &[
    "id",
    "name",
    "type",
    "status",
    "code",
    "value",
    "key",
    "kind",
    "data",
    "created_at",
    "updated_at",
    "deleted_at",
    "created",
    "updated",
    "date",
    "time",
];

/// A table, or one of its columns, that a note is about.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Anchor {
    /// As [`Table::qualified_name`].
    pub table: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub column: Option<String>,
    /// The column's fingerprint when the note was saved ([`column_fp`]); none for a table.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fp: Option<String>,
}

/// A short hash of what a column is, without its name: type, nullable, primary key, enum
/// values and the column it references. Two columns with the same fingerprint look alike.
pub fn column_fp(table: &Table, c: &Column) -> String {
    let mut values = c.enum_values.clone().unwrap_or_default();
    values.sort();
    let refs = table
        .foreign_keys
        .iter()
        .find(|f| f.column.eq_ignore_ascii_case(&c.name))
        .map(|f| format!("{}.{}", f.ref_table, f.ref_column).to_lowercase())
        .unwrap_or_default();
    let raw = format!(
        "{}|{}|{}|{}|{}",
        c.data_type.to_lowercase(),
        c.nullable,
        c.primary_key,
        values.join(","),
        refs
    );
    let digest = Sha256::digest(raw.as_bytes());
    digest[..6].iter().map(|b| format!("{b:02x}")).collect()
}

/// What a column is, in words, for reasons shown to people: "integer, not null" or
/// "text, values a|b".
pub fn describe_column(c: &Column) -> String {
    let mut out = c.data_type.to_lowercase();
    if !c.nullable {
        out.push_str(", not null");
    }
    if let Some(v) = &c.enum_values {
        out.push_str(&format!(", values {}", v.join("|")));
    }
    out
}

/// The words of a note that could name a table or column, lowercase, with any quotes taken
/// off: `memberships.role`, `"Orders"`, `` `status` ``.
fn words(text: &str) -> Vec<String> {
    text.split(|c: char| {
        !(c.is_alphanumeric()
            || c == '_'
            || c == '.'
            || c == '"'
            || c == '`'
            || c == '['
            || c == ']')
    })
    .map(|w| {
        w.trim_matches(|c: char| c == '.' || c == '"' || c == '`' || c == '[' || c == ']')
            .replace(['"', '`', '[', ']'], "")
            .to_lowercase()
    })
    .filter(|w| !w.is_empty())
    .collect()
}

fn table_named<'a>(schema: &'a Schema, word: &str) -> Option<&'a Table> {
    schema
        .tables
        .iter()
        .find(|t| t.qualified_name().to_lowercase() == word)
        .or_else(|| schema.tables.iter().find(|t| t.name.to_lowercase() == word))
}

fn column_named<'a>(t: &'a Table, word: &str) -> Option<&'a Column> {
    t.columns.iter().find(|c| c.name.to_lowercase() == word)
}

/// What resolving a note against a schema found.
#[derive(Debug, Default, PartialEq)]
pub struct Resolved {
    pub anchors: Vec<Anchor>,
    /// `table.column` the note names where the table exists but the column doesn't.
    pub dangling: Vec<String>,
}

/// The tables and columns `text` names in `schema`. A column counts when the note names it
/// with its table (`orders.status`), names its table too, or the column's name is distinctive
/// and belongs to exactly one table.
pub fn resolve(text: &str, schema: &Schema) -> Resolved {
    let words = words(text);
    let mut out = Resolved::default();
    let push = |out: &mut Resolved, a: Anchor| {
        if out.anchors.len() < MAX_ANCHORS && !out.anchors.contains(&a) {
            out.anchors.push(a);
        }
    };
    let mut tables: Vec<&Table> = Vec::new();
    // table.column first, then whole tables.
    for w in &words {
        if let Some(t) = table_named(schema, w) {
            if !tables.iter().any(|x| std::ptr::eq(*x, t)) {
                tables.push(t);
            }
            continue;
        }
        let Some((left, col)) = w.rsplit_once('.') else {
            continue;
        };
        let Some(t) = table_named(schema, left) else {
            continue;
        };
        if !tables.iter().any(|x| std::ptr::eq(*x, t)) {
            tables.push(t);
        }
        match column_named(t, col) {
            Some(c) => push(
                &mut out,
                Anchor {
                    table: t.qualified_name(),
                    column: Some(c.name.clone()),
                    fp: Some(column_fp(t, c)),
                },
            ),
            None => out.dangling.push(format!("{}.{col}", t.qualified_name())),
        }
    }
    for t in &tables {
        push(
            &mut out,
            Anchor {
                table: t.qualified_name(),
                column: None,
                fp: None,
            },
        );
    }
    // Bare column names: of a table the note names, or distinctive and in one table only.
    for w in &words {
        if w.contains('.') || table_named(schema, w).is_some() {
            continue;
        }
        let in_named: Vec<(&Table, &Column)> = tables
            .iter()
            .filter_map(|t| column_named(t, w).map(|c| (*t, c)))
            .collect();
        let hits = if !in_named.is_empty() {
            in_named
        } else if w.len() >= 4 && !GENERIC.contains(&w.as_str()) {
            let all: Vec<(&Table, &Column)> = schema
                .tables
                .iter()
                .filter_map(|t| column_named(t, w).map(|c| (t, c)))
                .collect();
            if all.len() == 1 { all } else { Vec::new() }
        } else {
            Vec::new()
        };
        for (t, c) in hits {
            push(
                &mut out,
                Anchor {
                    table: t.qualified_name(),
                    column: Some(c.name.clone()),
                    fp: Some(column_fp(t, c)),
                },
            );
        }
    }
    out
}

/// Why an anchor no longer holds in `schema`, or `None` when it does.
pub fn broken(anchor: &Anchor, schema: &Schema) -> Option<String> {
    let Some(t) = schema
        .tables
        .iter()
        .find(|t| t.qualified_name().eq_ignore_ascii_case(&anchor.table))
    else {
        return Some(format!("the table {} no longer exists", anchor.table));
    };
    let col = anchor.column.as_deref()?;
    let Some(c) = t.columns.iter().find(|c| c.name.eq_ignore_ascii_case(col)) else {
        return Some(format!(
            "the column {}.{col} no longer exists",
            anchor.table
        ));
    };
    match &anchor.fp {
        Some(fp) if *fp != column_fp(t, c) => Some(format!(
            "the column {}.{col} changed (now {})",
            anchor.table,
            describe_column(c)
        )),
        _ => None,
    }
}

/// The tables of `schema` that a rendered (possibly narrowed) schema text describes, by
/// qualified name: each table's line starts with its name (see [`super::render_table`]).
pub fn tables_in(text: &str, schema: &Schema) -> Vec<String> {
    let starts: std::collections::HashSet<&str> =
        text.lines().filter_map(|l| l.split(" [").next()).collect();
    schema
        .tables
        .iter()
        .filter(|t| starts.contains(super::render_name(t, &schema.dialect).as_str()))
        .map(|t| t.qualified_name())
        .collect()
}

/// A schema as kept between loads: per table, its columns' fingerprints.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    /// Qualified table name (lowercase) to column name (lowercase) and fingerprint.
    pub tables: BTreeMap<String, BTreeMap<String, String>>,
}

impl Snapshot {
    pub fn of(schema: &Schema) -> Snapshot {
        Snapshot {
            tables: schema
                .tables
                .iter()
                .map(|t| {
                    let cols = t
                        .columns
                        .iter()
                        .map(|c| (c.name.to_lowercase(), column_fp(t, c)))
                        .collect();
                    (t.qualified_name().to_lowercase(), cols)
                })
                .collect(),
        }
    }
}

/// How a schema changed between two snapshots.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    TableAdded(String),
    TableRemoved(String),
    /// The old table is gone and a new one has exactly its columns.
    TableRenamed {
        from: String,
        to: String,
    },
    ColumnAdded {
        table: String,
        column: String,
    },
    ColumnRemoved {
        table: String,
        column: String,
    },
    /// The only column that disappeared from a table, and the only one that appeared, look alike.
    ColumnRenamed {
        table: String,
        from: String,
        to: String,
    },
    ColumnChanged {
        table: String,
        column: String,
    },
}

impl Change {
    /// The change in words: "orders.status was renamed to state".
    pub fn describe(&self) -> String {
        match self {
            Change::TableAdded(t) => format!("table {t} was added"),
            Change::TableRemoved(t) => format!("table {t} was removed"),
            Change::TableRenamed { from, to } => format!("table {from} was renamed to {to}"),
            Change::ColumnAdded { table, column } => format!("{table}.{column} was added"),
            Change::ColumnRemoved { table, column } => format!("{table}.{column} was removed"),
            Change::ColumnRenamed { table, from, to } => {
                format!("{table}.{from} was renamed to {to}")
            }
            Change::ColumnChanged { table, column } => {
                format!("{table}.{column} changed type or values")
            }
        }
    }
}

/// What changed from `old` to `new`.
pub fn diff(old: &Snapshot, new: &Snapshot) -> Vec<Change> {
    let mut out = Vec::new();
    let gone: Vec<&String> = old
        .tables
        .keys()
        .filter(|t| !new.tables.contains_key(*t))
        .collect();
    let mut added: Vec<&String> = new
        .tables
        .keys()
        .filter(|t| !old.tables.contains_key(*t))
        .collect();
    for t in gone {
        let cols = &old.tables[t];
        match added.iter().position(|a| &new.tables[*a] == cols) {
            Some(i) => {
                let to = added.remove(i);
                out.push(Change::TableRenamed {
                    from: t.clone(),
                    to: to.clone(),
                });
            }
            None => out.push(Change::TableRemoved(t.clone())),
        }
    }
    out.extend(added.into_iter().map(|t| Change::TableAdded(t.clone())));
    for (t, before) in &old.tables {
        let Some(after) = new.tables.get(t) else {
            continue;
        };
        let removed: Vec<(&String, &String)> = before
            .iter()
            .filter(|(c, _)| !after.contains_key(*c))
            .collect();
        let appeared: Vec<(&String, &String)> = after
            .iter()
            .filter(|(c, _)| !before.contains_key(*c))
            .collect();
        if let ([(from, fa)], [(to, fb)]) = (removed.as_slice(), appeared.as_slice())
            && fa == fb
        {
            out.push(Change::ColumnRenamed {
                table: t.clone(),
                from: (*from).clone(),
                to: (*to).clone(),
            });
        } else {
            out.extend(removed.iter().map(|(c, _)| Change::ColumnRemoved {
                table: t.clone(),
                column: (*c).clone(),
            }));
            out.extend(appeared.iter().map(|(c, _)| Change::ColumnAdded {
                table: t.clone(),
                column: (*c).clone(),
            }));
        }
        for (c, fp) in before {
            if after.get(c).is_some_and(|f| f != fp) {
                out.push(Change::ColumnChanged {
                    table: t.clone(),
                    column: c.clone(),
                });
            }
        }
    }
    out
}

/// `text` with the whole-word name `from` replaced by `to`, matching case-insensitively, as a
/// rename would change a note: "orders.state" for "orders.status".
pub fn rename_in(text: &str, from: &str, to: &str) -> String {
    let lower = text.to_lowercase();
    let from_l = from.to_lowercase();
    let is_word = |c: char| c.is_alphanumeric() || c == '_';
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while let Some(off) = lower[i..].find(&from_l) {
        let start = i + off;
        let end = start + from_l.len();
        let before_ok = lower[..start]
            .chars()
            .next_back()
            .is_none_or(|c| !is_word(c));
        let after_ok = lower[end..].chars().next().is_none_or(|c| !is_word(c));
        out.push_str(&text[i..start]);
        if before_ok && after_ok {
            out.push_str(to);
        } else {
            out.push_str(&text[start..end]);
        }
        i = end;
    }
    out.push_str(&text[i..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::ForeignKey;

    fn col(name: &str, ty: &str) -> Column {
        Column {
            name: name.into(),
            data_type: ty.into(),
            nullable: true,
            primary_key: false,
            ..Default::default()
        }
    }

    fn table(name: &str, cols: Vec<Column>) -> Table {
        Table {
            schema: None,
            name: name.into(),
            kind: "table".into(),
            columns: cols,
            foreign_keys: vec![],
            row_estimate: None,
            ..Default::default()
        }
    }

    fn shop() -> Schema {
        let mut orders = table(
            "orders",
            vec![
                col("id", "integer"),
                col("status", "text"),
                col("amount_cents", "integer"),
                col("customer_id", "integer"),
            ],
        );
        orders.foreign_keys.push(ForeignKey {
            column: "customer_id".into(),
            ref_table: "customers".into(),
            ref_column: "id".into(),
        });
        Schema {
            dialect: "PostgreSQL".into(),
            tables: vec![
                orders,
                table(
                    "customers",
                    vec![
                        col("id", "integer"),
                        col("status", "text"),
                        col("tier_code", "text"),
                    ],
                ),
                table(
                    "memberships",
                    vec![col("id", "integer"), col("role", "text")],
                ),
            ],
        }
    }

    fn cols(r: &Resolved) -> Vec<String> {
        r.anchors
            .iter()
            .map(|a| match &a.column {
                Some(c) => format!("{}.{c}", a.table),
                None => a.table.clone(),
            })
            .collect()
    }

    #[test]
    fn finds_what_a_note_is_about() {
        let s = shop();
        assert_eq!(
            cols(&resolve("owner = memberships.role = 'owner'", &s)),
            ["memberships.role", "memberships"]
        );
        assert_eq!(
            cols(&resolve("In orders, status 3 = cancelled", &s)),
            ["orders", "orders.status"],
            "a generic column counts when its table is named"
        );
        assert_eq!(
            cols(&resolve("amounts are in cents (amount_cents)", &s)),
            ["orders.amount_cents"],
            "a distinctive column in one table counts alone"
        );
        assert!(
            resolve("status 3 = cancelled", &s).anchors.is_empty(),
            "status is in two tables"
        );
        assert_eq!(
            cols(&resolve("`Orders`.\"STATUS\" is free text", &s)),
            ["orders.status", "orders"]
        );
        let r = resolve("orders.state replaced status", &s);
        assert_eq!(r.dangling, ["orders.state"]);
    }

    #[test]
    fn says_why_an_anchor_broke() {
        let s = shop();
        let r = resolve("orders.status codes", &s);
        assert!(r.anchors.iter().all(|a| broken(a, &s).is_none()));

        let mut changed = shop();
        changed.tables[0].columns[1].enum_values = Some(vec!["new".into(), "paid".into()]);
        assert_eq!(
            broken(&r.anchors[0], &changed).unwrap(),
            "the column orders.status changed (now text, values new|paid)"
        );
        changed.tables[0].columns.remove(1);
        assert_eq!(
            broken(&r.anchors[0], &changed).unwrap(),
            "the column orders.status no longer exists"
        );
        changed.tables.remove(0);
        assert_eq!(
            broken(&r.anchors[1], &changed).unwrap(),
            "the table orders no longer exists"
        );
    }

    #[test]
    fn tells_a_rename_from_a_drop() {
        let before = Snapshot::of(&shop());
        let mut s = shop();
        s.tables[0].columns[1].name = "state".into();
        s.tables[2].name = "members".into();
        s.tables[1].columns.remove(2);
        s.tables[1].columns.push(col("joined_at", "timestamp"));
        let changes = diff(&before, &Snapshot::of(&s));
        assert!(changes.contains(&Change::ColumnRenamed {
            table: "orders".into(),
            from: "status".into(),
            to: "state".into()
        }));
        assert!(changes.contains(&Change::TableRenamed {
            from: "memberships".into(),
            to: "members".into()
        }));
        assert!(changes.contains(&Change::ColumnRemoved {
            table: "customers".into(),
            column: "tier_code".into()
        }));
        assert!(changes.contains(&Change::ColumnAdded {
            table: "customers".into(),
            column: "joined_at".into()
        }));
        assert!(diff(&before, &before).is_empty());
    }

    #[test]
    fn renames_whole_words_only() {
        assert_eq!(
            rename_in(
                "orders.status 3 = cancelled; see status_log",
                "status",
                "state"
            ),
            "orders.state 3 = cancelled; see status_log"
        );
        assert_eq!(
            rename_in("Memberships.role", "memberships", "members"),
            "members.role"
        );
    }
}
