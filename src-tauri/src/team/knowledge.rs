//! Shared knowledge (team server 0.5.0): the team's notes about each connection, kept true on
//! the server. On a team connection whose server has them, the Ask box and Investigate read
//! these instead of the notes saved on this computer, and new notes go to the server as
//! suggestions for the connection's knowledge owners to approve there.
//!
//! An older server (no `knowledge` feature, or a 404 or 402 from the route) keeps the local
//! notes, as before.

use super::{ID_PREFIX, REQUEST_TIMEOUT, ServerInfo, TeamClient, endpoint};
use crate::error::AppResult;
use crate::schema::anchors::Anchor;
use crate::state::lock;
use crate::store::agent::{CheckRun, Note, NoteCheck, Suspect};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// The feature a server lists in `/info` when it keeps shared knowledge.
pub const FEATURE: &str = "knowledge";

/// What the person sees after suggesting a note on a team connection.
pub const SENT: &str = "Sent to your team's knowledge owners for approval.";

/// How long the team's notes are used before they're asked for again.
const FRESH_FOR: Duration = Duration::from_secs(60);

/// How long reading the notes may take before the prompt goes on with the local ones.
const READ_TIMEOUT: Duration = Duration::from_secs(5);

/// A note as the server sends it (api-v1.md, "Shared knowledge").
#[derive(Debug, Clone, Deserialize)]
pub struct ServerNote {
    pub id: String,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub anchors: Vec<Anchor>,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub evidence: Option<String>,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub suspect_reason: Option<String>,
    #[serde(default)]
    pub check_sql: Option<String>,
    #[serde(default)]
    pub check_last: Option<ServerCheck>,
    #[serde(default)]
    pub verified_at: Option<String>,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub updated_at: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ServerCheck {
    #[serde(default)]
    pub ok: Option<bool>,
    #[serde(default)]
    pub at: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Deserialize)]
struct Reply {
    #[serde(default)]
    notes: Vec<ServerNote>,
    #[serde(default)]
    can_edit: bool,
}

/// The team's notes on one connection, in the app's shape.
#[derive(Debug, Clone, PartialEq)]
pub struct Shared {
    pub notes: Vec<Note>,
    /// The person is an owner or admin, who approve changes on the server's admin page.
    pub can_edit: bool,
}

/// A time from the server as milliseconds since the Unix epoch; 0 when it can't be read.
fn ms(at: Option<&str>) -> u64 {
    at.and_then(|s| chrono::DateTime::parse_from_rfc3339(s.trim()).ok())
        .map(|d| d.timestamp_millis().max(0) as u64)
        .unwrap_or(0)
}

impl ServerNote {
    /// The note as the app keeps one, so the prompts treat it like a local note. Retired
    /// notes are no longer true: `None`.
    pub fn into_note(self, connection_id: &str) -> Option<Note> {
        if self.status == "retired" || self.text.trim().is_empty() {
            return None;
        }
        let updated = ms(self.updated_at.as_deref());
        let suspect = (self.status == "suspect").then(|| Suspect {
            reason: self
                .suspect_reason
                .filter(|r| !r.trim().is_empty())
                .unwrap_or_else(|| "something changed since it was written".into()),
            since: updated,
        });
        let check = self
            .check_sql
            .filter(|c| !c.trim().is_empty())
            .map(|sql| NoteCheck {
                sql,
                last: self.check_last.map(|c| CheckRun {
                    ok: c.ok,
                    at: ms(c.at.as_deref()),
                    error: c.error,
                }),
            });
        Some(Note {
            id: self.id,
            connection_id: connection_id.to_string(),
            database: None,
            text: self.text,
            updated_at: updated,
            created_at: ms(self.created_at.as_deref()),
            source: self.source,
            evidence: self.evidence.filter(|e| !e.trim().is_empty()),
            anchors: self.anchors,
            suspect,
            check,
            verified_at: self.verified_at.as_deref().map(|v| ms(Some(v))),
            history: Vec::new(),
        })
    }
}

/// A suggested note (or fix to one) for the server's knowledge owners.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Suggestion {
    pub text: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub evidence: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub check: Option<String>,
    /// The note it rewrites or removes: its `id` or `ref` on the server.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub replaces: Option<String>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub remove: bool,
    /// Where in the app it came from ("Investigate"), shown after the person's email.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

impl ServerInfo {
    /// Whether the server lists `feature` (such as [`FEATURE`]) in `/info`.
    pub fn has(&self, feature: &str) -> bool {
        self.features.iter().any(|f| f == feature)
    }
}

impl TeamClient {
    /// The team's live notes on connection `id` (the server's id), mapped for the app connection
    /// `local_id`. `None` when the server doesn't keep them: a server before 0.5.0 (404) or
    /// one without a licence (402).
    pub async fn knowledge(&self, id: &str, local_id: &str) -> AppResult<Option<Shared>> {
        let resp = self
            .http
            .get(endpoint(&self.server, &["connections", id, "knowledge"])?)
            .timeout(READ_TIMEOUT)
            .bearer_auth(&self.token)
            .send()
            .await
            .map_err(|e| super::unreachable(&self.server, e))?;
        if matches!(resp.status().as_u16(), 402 | 404) {
            return Ok(None);
        }
        let r: Reply = self.read(resp).await.map_err(|(_, e)| e)?;
        Ok(Some(Shared {
            notes: r
                .notes
                .into_iter()
                .filter_map(|n| n.into_note(local_id))
                .collect(),
            can_edit: r.can_edit,
        }))
    }

    /// Suggests a note on connection `id` (the server's id) for its owners to approve.
    pub async fn suggest_knowledge(&self, id: &str, s: &Suggestion) -> AppResult<()> {
        self.send::<Value>(
            self.http
                .post(endpoint(
                    &self.server,
                    &["connections", id, "knowledge", "suggestions"],
                )?)
                .timeout(REQUEST_TIMEOUT)
                .json(s),
        )
        .await
        .map(|_| ())
    }
}

/// The team's notes per team connection, read at most about once a minute, and whether the
/// signed-in server keeps them at all.
#[derive(Default)]
pub struct Cache {
    /// From the server's `/info` features; `None` until it's known.
    knowledge: Mutex<Option<bool>>,
    /// By the app's connection id: when they were read, and what (`None`: use local notes).
    notes: Mutex<HashMap<String, (Instant, Option<Shared>)>>,
}

impl Cache {
    /// Remembers what the server can do, from `/info` or `/me`.
    pub fn set_server(&self, info: &ServerInfo) {
        let has = info.has(FEATURE);
        let before = lock(&self.knowledge).replace(has);
        if before != Some(has) {
            lock(&self.notes).clear();
        }
    }

    /// Reads the connection's notes again next time (after a suggestion, say).
    pub fn forget(&self, local_id: &str) {
        lock(&self.notes).remove(local_id);
    }

    /// Forgets everything, after signing in or out.
    pub fn clear(&self) {
        *lock(&self.knowledge) = None;
        lock(&self.notes).clear();
    }

    /// The team's notes for the app connection `local_id` (`team:…`), or `None` to use the
    /// notes saved on this computer: not a team connection, a server without shared knowledge,
    /// or one that can't be reached right now (then the last notes read, if any).
    pub async fn notes(&self, client: &TeamClient, local_id: &str) -> Option<Shared> {
        let server_id = local_id.strip_prefix(ID_PREFIX)?;
        let known = *lock(&self.knowledge);
        let has = match known {
            Some(has) => has,
            None => match client.status().await {
                Ok((_, info)) => {
                    self.set_server(&info);
                    info.has(FEATURE)
                }
                Err(_) => return self.last(local_id),
            },
        };
        if !has {
            return None;
        }
        if let Some((at, shared)) = lock(&self.notes).get(local_id)
            && at.elapsed() < FRESH_FOR
        {
            return shared.clone();
        }
        let shared = match client.knowledge(server_id, local_id).await {
            Ok(shared) => shared,
            Err(e) => {
                // The local notes stand in; try again in a minute.
                eprintln!("sayql: couldn't read the team's notes: {e}");
                self.last(local_id)
            }
        };
        lock(&self.notes).insert(local_id.to_string(), (Instant::now(), shared.clone()));
        shared
    }

    fn last(&self, local_id: &str) -> Option<Shared> {
        lock(&self.notes).get(local_id).and_then(|(_, s)| s.clone())
    }
}

/// The notes about any of `tables` (qualified names), plus those not tied to a table, as the
/// Ask box shows them. The server names a table in the default schema without its schema
/// (`orders`, not `dbo.orders`), so a bare name matches too.
pub fn about(notes: &[Note], tables: &[String]) -> Vec<Note> {
    notes
        .iter()
        .filter(|n| {
            n.anchors.is_empty()
                || n.anchors
                    .iter()
                    .any(|a| tables.iter().any(|t| names(&a.table, t)))
        })
        .cloned()
        .collect()
}

/// Whether a note's anchor `table` names the table `qualified`: the same name, or the same
/// table without its schema.
pub fn names(table: &str, qualified: &str) -> bool {
    qualified.eq_ignore_ascii_case(table)
        || (!table.contains('.')
            && qualified
                .rsplit('.')
                .next()
                .is_some_and(|b| b.eq_ignore_ascii_case(table)))
}

#[cfg(test)]
pub(crate) mod tests;
