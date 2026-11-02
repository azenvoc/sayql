//! The schema model sent to the UI and, rendered compactly, to the AI.
//!
//! Unlike the rest of the app's JSON, these structs serialize with their snake_case field names
//! (`foreign_keys`, `data_type`, `primary_key`, `ref_table`, …). The UI is the only JSON consumer
//! (MCP clients and the AI get rendered text), nothing is persisted, and `src/api/types.ts` plus the
//! code reading it use these names, so they stay as they are. See docs/ARCHITECTURE.md.

pub mod anchors;
mod render;

pub use render::{
    quote_ident, relevant_tables, render_full, render_name, render_table, schema_for_prompt,
    schema_with_focus,
};

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct Schema {
    pub dialect: String,
    pub tables: Vec<Table>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Table {
    /// Postgres schema / MySQL database. `None` for SQLite.
    pub schema: Option<String>,
    pub name: String,
    /// "table" or "view"
    pub kind: String,
    pub columns: Vec<Column>,
    pub foreign_keys: Vec<ForeignKey>,
    pub row_estimate: Option<i64>,
    /// The database's own description of the table (`COMMENT ON TABLE`, MS_Description, …).
    pub comment: Option<String>,
    /// CHECK constraints as the database writes them, each starting with `CHECK`.
    pub checks: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Column {
    pub name: String,
    pub data_type: String,
    pub nullable: bool,
    pub primary_key: bool,
    /// Allowed values for enum-typed columns, when the database exposes them.
    pub enum_values: Option<Vec<String>>,
    /// The database's own description of the column.
    pub comment: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ForeignKey {
    pub column: String,
    /// Qualified like the referencing table (e.g. `public.customers`).
    pub ref_table: String,
    pub ref_column: String,
}

impl Table {
    pub fn qualified_name(&self) -> String {
        match &self.schema {
            Some(s) if s != "public" => format!("{s}.{}", self.name),
            _ => self.name.clone(),
        }
    }
}
