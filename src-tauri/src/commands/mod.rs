//! Tauri commands, grouped by the part of the UI they serve.
//! Each has a typed wrapper in the generated src/api/bindings.ts (see `bindings`).

pub mod agent;
pub mod app;
pub mod audit;
pub mod connections;
pub mod generation;
pub mod handoff;
pub mod query;
pub mod scripts;
pub mod settings;
pub mod skills;
pub mod team;
pub mod updates;
pub mod voice;
pub mod workspaces;

#[cfg(test)]
mod bindings;
