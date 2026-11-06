//! SayQL's safety code: what keeps a database's data from an AI that may not see it, in one
//! place to review. Everything here is shared by the MCP server, agent mode and note checks.
//!
//! - `sql_lex`: the SQL lexer every check shares (strings, quoted names and comments per dialect).
//! - `classify`: `is_read_only`, the per-dialect write check (re-exported as `db::is_read_only`).
//! - `never_send`: refuses SQL that could read a never-send column, and functions that read
//!   server files.
//! - `readonly`: read-only access for AI clients: schema search, queries stopped on the server
//!   after a time limit, row caps, the write refusals.
//! - `redact`: database error messages in the fixed form an AI that may not see values gets.
//! - `pii`: likely personal data in text.
//! - `mask`: masks likely personal data in results as per-run tokens and puts real values back.
//! - `gate`: the AI query gate, which every query run for an AI goes through.

pub mod classify;
pub mod databases;
pub mod gate;
pub mod mask;
pub mod never_send;
pub mod pii;
pub mod readonly;
pub(crate) mod redact;
pub mod sql_lex;
