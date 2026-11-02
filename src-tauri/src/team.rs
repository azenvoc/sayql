//! SayQL Team: the client side of a company's SayQL server (docs: sayql-server's api-v1.md).
//!
//! Signing in redeems an invite code for a token, kept in the OS keychain; the server's address
//! and the person's email go in settings. Team connections live on the server: SayQL never sees
//! their host or password, reads their schema from the server, and sends every query there,
//! where the server checks it is read-only and records it in its audit log. The admin's rules
//! (how much agent mode may see, never-send columns) are applied on top of the person's own.
//!
//! Signed out, nothing here runs: no team server is known, so no request can be made.

use crate::db::{CANCELLED, Kind, QueryResult};
use crate::error::{AppError, AppResult};
use crate::schema::{Column, Schema, Table};
use crate::store::agent::{AgentAccess, Results};
use crate::store::{Store, secrets};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::time::Duration;
use tokio_util::sync::CancellationToken;
use url::Url;

/// Team connections' ids in the app: the server's id after this prefix, so they never clash
/// with saved connections.
pub const ID_PREFIX: &str = "team:";

pub const SIGNED_OUT: &str = "Signed out of your team, sign in again.";
pub const ENTER_SERVER: &str = "Enter your server's address, for example https://sayql.example.com";
pub const AGENT_OFF: &str = "Investigate is turned off for this connection by your team admin.";
pub const BAD_INVITE: &str =
    "That invite code isn't valid or has expired. Ask your team admin for a new one.";
pub const SSO_EXPIRED: &str = "This sign-in expired, was already used, or was started somewhere else. Sign in again from SayQL.";
pub const WRITE_REFUSED: &str =
    "Your team's SayQL server runs only read-only queries, so this statement was not run.";

/// How long a sign-in, schema or list request may take. Queries have no limit of their own:
/// Stop, or agent mode's time per query, ends them.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Who is signed in, and where. Saved in settings; the token is in the keychain.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TeamAccount {
    pub server: String,
    pub email: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub role: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct TeamUser {
    #[serde(default)]
    pub email: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub role: String,
}

/// `/info`, also inside `/me`: the server and its licence.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all(serialize = "camelCase", deserialize = "snake_case"))]
pub struct ServerInfo {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub licensed: bool,
    #[serde(default)]
    pub plan: Option<String>,
    #[serde(default)]
    pub licensee: Option<String>,
    #[serde(default)]
    pub expires: Option<String>,
    #[serde(default)]
    pub users: Option<u32>,
    #[serde(default)]
    pub user_limit: Option<u32>,
    /// What the server can do beyond queries, such as `knowledge` (0.5.0). Older servers omit it.
    #[serde(default)]
    pub features: Vec<String>,
}

#[derive(Deserialize)]
struct Me {
    user: TeamUser,
    #[serde(default)]
    server: ServerInfo,
}

#[derive(Deserialize)]
struct Redeemed {
    token: String,
    user: TeamUser,
}

/// Whether the server offers "Sign in with your company" (`GET /api/v1/auth/sso`).
#[derive(Debug, Default, Deserialize)]
pub struct SsoInfo {
    #[serde(default)]
    pub enabled: bool,
}

/// The signed-in account with the server's licence, for Settings.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TeamStatus {
    pub account: TeamAccount,
    pub server: ServerInfo,
    /// Team AI and this month's usage; `None` if the server couldn't say.
    pub ai: Option<ai::AiStatus>,
}

/// One shared connection as the server lists it: never its host, user or password.
#[derive(Debug, Clone, Deserialize)]
pub struct TeamConnection {
    pub id: String,
    pub name: String,
    pub engine: String,
    #[serde(default)]
    pub database: String,
    /// The most agent mode may see: `off`, `counts` or `masked_rows`.
    #[serde(default)]
    pub agent_access: String,
    /// `table.column`, never sent to the AI and withheld from results by the server.
    #[serde(default)]
    pub never_send: Vec<String>,
    #[serde(default)]
    pub notes: String,
}

impl TeamConnection {
    pub fn local_id(&self) -> String {
        format!("{ID_PREFIX}{}", self.id)
    }

    pub fn kind(&self) -> Option<Kind> {
        match self.engine.to_ascii_lowercase().as_str() {
            "postgres" | "postgresql" => Some(Kind::Postgres),
            "mysql" | "mariadb" => Some(Kind::Mysql),
            "sqlite" => Some(Kind::Sqlite),
            "mssql" | "sqlserver" => Some(Kind::Mssql),
            _ => None,
        }
    }

    /// The admin turned agent mode off here: the server refuses agent queries.
    pub fn agent_off(&self) -> bool {
        self.agent_access == "off"
    }

    pub fn rules(&self) -> TeamRules {
        TeamRules {
            max_results: max_results(&self.agent_access),
            never_send: self.never_send.clone(),
        }
    }
}

/// What the team admin set for agent mode on a connection. The person can't loosen it.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TeamRules {
    pub max_results: Results,
    pub never_send: Vec<String>,
}

/// The server's `agent_access` as agent mode's results setting. Anything unknown is Off.
fn max_results(agent_access: &str) -> Results {
    match agent_access {
        "counts" => Results::Counts,
        "masked_rows" => Results::Rows,
        _ => Results::Off,
    }
}

fn rank(r: Results) -> u8 {
    match r {
        Results::Off => 0,
        Results::Counts => 1,
        Results::Rows => 2,
    }
}

/// The person's agent access with the admin's rules on top: results no higher than the
/// admin's, and the admin's never-send columns added.
pub fn apply_rules(access: AgentAccess, rules: &TeamRules) -> AgentAccess {
    let results = if rank(access.results) > rank(rules.max_results) {
        rules.max_results
    } else {
        access.results
    };
    let mut never_send = access.never_send;
    for col in &rules.never_send {
        if !never_send.iter().any(|c| c.eq_ignore_ascii_case(col)) {
            never_send.push(col.clone());
        }
    }
    AgentAccess {
        results,
        never_send,
        ..access
    }
}

/// What the person saves: their own choices, without the admin's columns (added back when read).
pub fn own_access(access: AgentAccess, rules: &TeamRules) -> AgentAccess {
    let never_send = access
        .never_send
        .into_iter()
        .filter(|c| !rules.never_send.iter().any(|r| r.eq_ignore_ascii_case(c)))
        .collect();
    apply_rules(
        AgentAccess {
            never_send,
            ..access
        },
        &TeamRules {
            never_send: vec![],
            ..rules.clone()
        },
    )
}

/// The server address as typed, cleaned up: `https://` added when missing, no trailing slash.
/// Plain `http://` is only accepted on this computer, so the token never crosses a network
/// unencrypted.
pub fn clean_server(input: &str) -> Result<String, String> {
    let bad = || ENTER_SERVER.to_string();
    let input = input.trim();
    // Half-typed schemes ("http:/", "https:") aren't hosts: don't turn them into https://http.
    let schemeless = !input.contains("://");
    if input.is_empty()
        || (schemeless
            && input.contains(':')
            && input
                .split(':')
                .next()
                .is_some_and(|s| matches!(s.to_ascii_lowercase().as_str(), "http" | "https")))
    {
        return Err(bad());
    }
    let input = input.trim_end_matches('/');
    let with_scheme = if schemeless {
        format!("https://{input}")
    } else {
        input.to_string()
    };
    let url = Url::parse(&with_scheme).map_err(|_| bad())?;
    let host = url.host_str().filter(|h| !h.is_empty()).ok_or_else(bad)?;
    if schemeless && matches!(host.to_ascii_lowercase().as_str(), "http" | "https") {
        return Err(bad());
    }
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(bad());
    }
    let local = matches!(host, "localhost" | "127.0.0.1" | "[::1]") || host.ends_with(".localhost");
    match url.scheme() {
        "https" => {}
        "http" if local => {}
        "http" => return Err(
            "Use the server's https:// address: SayQL doesn't send your team sign-in unencrypted."
                .into(),
        ),
        _ => return Err(bad()),
    }
    Ok(url.as_str().trim_end_matches('/').to_string())
}

/// An invite code as typed: trimmed, one line, not absurdly long.
pub fn clean_code(input: &str) -> Result<String, String> {
    let code = input.trim();
    if code.is_empty() {
        return Err("Enter the invite code from your team admin.".into());
    }
    if code.len() > 200 || code.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return Err("That doesn't look like an invite code. Copy it again from the invite.".into());
    }
    Ok(code.to_string())
}

fn endpoint(server: &str, segments: &[&str]) -> AppResult<Url> {
    let mut url = Url::parse(server)
        .map_err(|_| AppError::User(format!("{server} isn't a server address.")))?;
    url.path_segments_mut()
        .map_err(|_| AppError::User(format!("{server} isn't a server address.")))?
        .pop_if_empty()
        .extend(["api", "v1"])
        .extend(segments);
    Ok(url)
}

fn unreachable(server: &str, e: reqwest::Error) -> AppError {
    if e.is_timeout() {
        return AppError::User(format!(
            "Your team's SayQL server at {server} didn't answer in time."
        ));
    }
    AppError::User(format!(
        "Couldn't reach your team's SayQL server at {server}. Check the address and your network."
    ))
}

#[derive(Deserialize)]
struct ErrorBody {
    error: ErrorDetail,
}

#[derive(Deserialize)]
struct ErrorDetail {
    code: String,
    #[serde(default)]
    message: String,
}

/// A server error code as the message the person sees.
pub fn error_text(code: &str, message: &str) -> String {
    let or = |fallback: &str| {
        if message.trim().is_empty() {
            fallback.to_string()
        } else {
            message.trim().to_string()
        }
    };
    match code {
        "unauthorized" => SIGNED_OUT.into(),
        "write_refused" => WRITE_REFUSED.into(),
        "never_send" => {
            let base = "Your team admin marked a column in this query never send, so it was not run. Leave that column out.";
            if message.trim().is_empty() { base.into() } else { format!("{base}\n{}", message.trim()) }
        }
        "no_licence" => "Your team's SayQL server has no licence, so it can't run queries. Ask your team admin.".into(),
        "licence_expired" => "Your team's SayQL licence has expired, so queries can't run. Ask your team admin to renew it.".into(),
        "seat_limit" => "Your team has no free seats on its SayQL licence. Ask your team admin to free one or add more.".into(),
        "forbidden" => or("Your team admin hasn't given you access to this."),
        "not_found" => or("Your team's server doesn't have this any more. It may have been removed by your team admin."),
        "timeout" => or("The query took too long on your team's server, so it was stopped."),
        _ => or("Your team's SayQL server couldn't do that."),
    }
}

/// Codes for a team server refusing by its rules, as opposed to the database failing.
const REFUSALS: [&str; 7] = [
    "write_refused",
    "never_send",
    "forbidden",
    "no_licence",
    "licence_expired",
    "seat_limit",
    "unauthorized",
];

/// The error for a server error code: a refusal, or a plain message (`db_error`, `timeout`, …).
fn coded_error(code: Option<&str>, msg: String) -> AppError {
    match code {
        Some(c) if REFUSALS.contains(&c) => AppError::TeamRefused(msg),
        _ => AppError::User(msg),
    }
}

/// The body of a reply as `T`, or the server's error as a message. `None` for `204`.
async fn decode<T: DeserializeOwned>(
    resp: reqwest::Response,
) -> Result<T, (Option<String>, String)> {
    let status = resp.status();
    let bytes = resp.bytes().await.map_err(|e| {
        (
            None,
            format!("Your team's SayQL server stopped answering: {e}"),
        )
    })?;
    if status.is_success() {
        let body: &[u8] = if bytes.is_empty() { b"null" } else { &bytes };
        return serde_json::from_slice(body)
            .map_err(|_| (None, "Your team's SayQL server sent an answer this version of SayQL can't read. Update SayQL, or ask your team admin.".into()));
    }
    match serde_json::from_slice::<ErrorBody>(&bytes) {
        Ok(b) => Err((
            Some(b.error.code.clone()),
            error_text(&b.error.code, &b.error.message),
        )),
        Err(_) if status == reqwest::StatusCode::UNAUTHORIZED => {
            Err((Some("unauthorized".into()), SIGNED_OUT.into()))
        }
        Err(_) => Err((
            None,
            format!("Your team's SayQL server answered with an error ({status})."),
        )),
    }
}

/// Redeems an invite code. Returns the token and who it belongs to.
pub async fn redeem(
    http: &reqwest::Client,
    server: &str,
    code: &str,
    device: &str,
) -> AppResult<(String, TeamAccount)> {
    let resp = http
        .post(endpoint(server, &["auth", "redeem"])?)
        .timeout(REQUEST_TIMEOUT)
        .json(&json!({ "code": code, "device": device }))
        .send()
        .await
        .map_err(|e| unreachable(server, e))?;
    let r: Redeemed = decode(resp)
        .await
        .map_err(|(code, msg)| match code.as_deref() {
            Some("unauthorized" | "not_found" | "bad_request") => AppError::User(BAD_INVITE.into()),
            // Such as: this team signs in with the company's account, not invite codes.
            Some("forbidden") => AppError::User(error_text("forbidden", &msg)),
            c => coded_error(c, msg),
        })?;
    signed_in(server, r)
}

fn signed_in(server: &str, r: Redeemed) -> AppResult<(String, TeamAccount)> {
    if r.token.is_empty() {
        return Err("Your team's SayQL server didn't return a sign-in.".into());
    }
    Ok((
        r.token,
        TeamAccount {
            server: server.to_string(),
            email: r.user.email,
            name: r.user.name,
            role: r.user.role,
        },
    ))
}

/// Whether `server` offers sign-in with the company's identity provider. Servers before 0.4.0
/// don't have the route, and don't.
pub async fn sso_info(http: &reqwest::Client, server: &str) -> AppResult<SsoInfo> {
    let resp = http
        .get(endpoint(server, &["auth", "sso"])?)
        .timeout(REQUEST_TIMEOUT)
        .send()
        .await
        .map_err(|e| unreachable(server, e))?;
    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(SsoInfo::default());
    }
    decode(resp)
        .await
        .map_err(|(c, msg)| coded_error(c.as_deref(), msg))
}

/// A PKCE pair (RFC 7636): the verifier SayQL keeps, and the challenge sent with the sign-in.
/// The server only hands out a sign-in for the code it sends back with the matching verifier.
pub fn pkce_pair() -> (String, String) {
    use base64::Engine;
    use sha2::{Digest, Sha256};
    let verifier = format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(Sha256::digest(verifier.as_bytes()));
    (verifier, challenge)
}

/// The page that starts a sign-in at the company's identity provider, opened in the browser.
pub fn sso_start_url(server: &str, challenge: &str, device: &str) -> AppResult<String> {
    let mut url = Url::parse(server)
        .map_err(|_| AppError::User(format!("{server} isn't a server address.")))?;
    url.path_segments_mut()
        .map_err(|_| AppError::User(format!("{server} isn't a server address.")))?
        .pop_if_empty()
        .extend(["auth", "sso", "start"]);
    url.query_pairs_mut()
        .append_pair("target", "app")
        .append_pair("challenge", challenge)
        .append_pair("device", device);
    Ok(url.into())
}

/// The one-time code from `sayql://team-sso`: letters, digits, `-` and `_`.
pub fn clean_sso_code(input: &str) -> Result<String, String> {
    let code = input.trim();
    let ok = (16..=128).contains(&code.len())
        && code
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    ok.then(|| code.to_string())
        .ok_or_else(|| "That sign-in link isn't valid.".to_string())
}

/// Ends a sign-in with the company's identity provider: the code from the browser, with the
/// verifier only this app has. Returns the token and who it belongs to.
pub async fn sso_redeem(
    http: &reqwest::Client,
    server: &str,
    code: &str,
    verifier: &str,
    device: &str,
) -> AppResult<(String, TeamAccount)> {
    let resp = http
        .post(endpoint(server, &["auth", "sso", "redeem"])?)
        .timeout(REQUEST_TIMEOUT)
        .json(&json!({ "code": code, "verifier": verifier, "device": device }))
        .send()
        .await
        .map_err(|e| unreachable(server, e))?;
    let r: Redeemed = decode(resp)
        .await
        .map_err(|(code, msg)| match code.as_deref() {
            Some("unauthorized") => AppError::User(SSO_EXPIRED.into()),
            c => coded_error(c, msg),
        })?;
    signed_in(server, r)
}

/// This computer's name as the server lists it among the person's devices.
pub fn device_name() -> String {
    let os = match std::env::consts::OS {
        "macos" => "Mac",
        "windows" => "Windows PC",
        "linux" => "Linux PC",
        other => other,
    };
    let host = std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_default();
    let host: String = host.chars().filter(|c| !c.is_control()).take(60).collect();
    if host.trim().is_empty() {
        format!("SayQL on a {os}")
    } else {
        format!("SayQL on {}", host.trim())
    }
}

/// Requests to the signed-in team server.
#[derive(Clone)]
pub struct TeamClient {
    http: reqwest::Client,
    server: String,
    token: String,
    /// Where the sign-in is saved, to forget it when the server says the token is no good.
    store: Option<Store>,
}

impl TeamClient {
    pub fn new(http: reqwest::Client, server: String, token: String, store: Option<Store>) -> Self {
        TeamClient {
            http,
            server,
            token,
            store,
        }
    }

    async fn send<T: DeserializeOwned>(&self, req: reqwest::RequestBuilder) -> AppResult<T> {
        self.send_coded(req).await.map_err(|(_, e)| e)
    }

    /// `send`, also giving the server's error code (errors are mapped by code, never by status).
    async fn send_coded<T: DeserializeOwned>(
        &self,
        req: reqwest::RequestBuilder,
    ) -> Result<T, (Option<String>, AppError)> {
        let resp = req
            .bearer_auth(&self.token)
            .send()
            .await
            .map_err(|e| (None, unreachable(&self.server, e)))?;
        self.read(resp).await
    }

    /// A reply's body as `T`, or the server's error with its code. A revoked token is forgotten.
    async fn read<T: DeserializeOwned>(
        &self,
        resp: reqwest::Response,
    ) -> Result<T, (Option<String>, AppError)> {
        decode(resp).await.map_err(|(code, msg)| {
            if code.as_deref() == Some("unauthorized")
                && let Some(store) = &self.store
            {
                // The token was revoked (or the person removed): it's no use any more.
                forget_sign_in(store);
            }
            let e = coded_error(code.as_deref(), msg);
            (code, e)
        })
    }

    async fn get<T: DeserializeOwned>(&self, segments: &[&str]) -> AppResult<T> {
        self.send(
            self.http
                .get(endpoint(&self.server, segments)?)
                .timeout(REQUEST_TIMEOUT),
        )
        .await
    }

    pub async fn status(&self) -> AppResult<(TeamUser, ServerInfo)> {
        let me: Me = self.get(&["me"]).await?;
        Ok((me.user, me.server))
    }

    pub async fn sign_out(&self) -> AppResult<()> {
        self.send::<Value>(
            self.http
                .post(endpoint(&self.server, &["auth", "signout"])?)
                .timeout(REQUEST_TIMEOUT),
        )
        .await
        .map(|_| ())
    }

    /// Team AI: whether the company key is set up, its models and this month's usage.
    pub async fn ai_status(&self) -> AppResult<ai::AiStatus> {
        self.get(&["ai"]).await
    }

    pub fn server(&self) -> &str {
        &self.server
    }

    pub fn token(&self) -> &str {
        &self.token
    }

    pub async fn connections(&self) -> AppResult<Vec<TeamConnection>> {
        self.get(&["connections"]).await
    }

    pub async fn schema(&self, id: &str, kind: Kind) -> AppResult<Schema> {
        let s: SchemaReply = self.get(&["connections", id, "schema"]).await?;
        Ok(Schema {
            dialect: kind.dialect().to_string(),
            tables: s.tables.into_iter().map(Into::into).collect(),
        })
    }

    pub async fn query(
        &self,
        id: &str,
        body: Value,
        cancel: &CancellationToken,
    ) -> AppResult<QueryResult> {
        let agent = body["source"] == "agent";
        let req = self.send_coded::<QueryReply>(
            self.http
                .post(endpoint(&self.server, &["connections", id, "query"])?)
                .json(&body),
        );
        // Stop ends the wait; the server finishes (and audits) the query on its side.
        let r = tokio::select! {
            r = req => r.map_err(|(code, e)| match code.as_deref() {
                // The server refuses agent mode's queries where the admin turned it off.
                Some("forbidden") if agent => AppError::TeamRefused(AGENT_OFF.into()),
                _ => e,
            })?,
            _ = cancel.cancelled() => return Err(CANCELLED.into()),
        };
        Ok(r.into())
    }
}

/// Forgets the sign-in on this computer: the token and the saved server.
pub fn forget_sign_in(store: &Store) {
    if let Err(e) = secrets::delete_team_token() {
        eprintln!("sayql: couldn't remove the team sign-in from the keychain: {e}");
    }
    let _ = store.update_settings(|s| {
        s.team = None;
        Ok(())
    });
}

#[derive(Deserialize)]
struct SchemaReply {
    #[serde(default)]
    tables: Vec<SchemaTable>,
}

#[derive(Deserialize)]
struct SchemaTable {
    #[serde(default)]
    schema: Option<String>,
    name: String,
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    columns: Vec<SchemaColumn>,
    // Empty until the server sends the database's comments and checks.
    #[serde(default)]
    comment: Option<String>,
    #[serde(default)]
    checks: Vec<String>,
}

#[derive(Deserialize)]
struct SchemaColumn {
    name: String,
    #[serde(default, rename = "type")]
    data_type: String,
    #[serde(default = "yes")]
    nullable: bool,
    #[serde(default)]
    comment: Option<String>,
}

fn yes() -> bool {
    true
}

impl From<SchemaTable> for Table {
    fn from(t: SchemaTable) -> Self {
        Table {
            schema: t.schema.filter(|s| !s.is_empty()),
            name: t.name,
            kind: match t.kind.as_deref() {
                Some("view") => "view".into(),
                Some("materialized view") => "materialized view".into(),
                _ => "table".into(),
            },
            columns: t
                .columns
                .into_iter()
                .map(|c| Column {
                    name: c.name,
                    data_type: c.data_type,
                    nullable: c.nullable,
                    comment: c.comment,
                    ..Default::default()
                })
                .collect(),
            comment: t.comment,
            checks: t.checks,
            ..Default::default()
        }
    }
}

#[derive(Deserialize)]
struct QueryReply {
    #[serde(default)]
    columns: Vec<ReplyColumn>,
    #[serde(default)]
    rows: Vec<Vec<Value>>,
    #[serde(default)]
    truncated: bool,
    #[serde(default)]
    elapsed_ms: u64,
    #[serde(default)]
    withheld: Vec<String>,
}

#[derive(Deserialize)]
struct ReplyColumn {
    name: String,
}

impl From<QueryReply> for QueryResult {
    fn from(r: QueryReply) -> Self {
        let mut columns: Vec<String> = r.columns.into_iter().map(|c| c.name).collect();
        // Withheld columns are left out of the rows; drop them from the header too if the server kept them there.
        let width = r.rows.first().map(Vec::len);
        if width.is_some_and(|w| w < columns.len()) {
            columns.retain(|c| !r.withheld.iter().any(|w| w.eq_ignore_ascii_case(c)));
        }
        QueryResult {
            columns,
            rows: r.rows,
            rows_affected: None,
            truncated: r.truncated,
            elapsed_ms: r.elapsed_ms as u128,
            withheld: r.withheld,
        }
    }
}

/// Where a query on a team connection came from, for the server's audit log.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    Ask,
    Agent,
    #[default]
    Editor,
}

/// A team connection in place of a database pool: every query goes to the team server.
#[derive(Clone)]
pub struct TeamPool {
    pub client: TeamClient,
    /// The server's id for the connection.
    pub id: String,
    pub kind: Kind,
    pub source: Source,
    pub question: Option<String>,
}

impl TeamPool {
    pub async fn query(
        &self,
        sql: &str,
        cap: usize,
        cancel: &CancellationToken,
    ) -> AppResult<QueryResult> {
        let body = json!({
            "sql": sql,
            "source": self.source,
            "question": self.question.as_deref().map(str::trim).filter(|q| !q.is_empty()),
            "max_rows": cap.min(10_000),
        });
        self.client.query(&self.id, body, cancel).await
    }

    pub async fn schema(&self) -> AppResult<Schema> {
        self.client.schema(&self.id, self.kind).await
    }
}

pub mod ai;
pub mod knowledge;

#[cfg(test)]
pub(crate) mod tests;
