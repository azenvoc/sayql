//! Handoffs from AI apps to the SayQL window: `sayql-mcp` asks the app to open a prefilled
//! New connection dialog, a new SQL tab or a saved script, and the person finishes the job there.
//!
//! The MCP process opens a `sayql://` deep link with the OS opener, so it works whether SayQL
//! is running (the single-instance plugin routes the link to it) or not (the OS starts it).
//! Any website can open a `sayql://` link too, so every link is untrusted: the fields are
//! validated here, and a link is only acted on with a one-time token that `sayql-mcp` wrote
//! into the app's config folder, which only the same user's local processes can do.
//! Nothing is ever run or saved: the person reviews the dialog or tab and acts on it.
//! Passwords and other secrets are never part of a handoff.

use crate::db::Kind;
use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};
use url::Url;

pub const SCHEME: &str = "sayql";
/// Longest SQL accepted in a handoff, in characters.
pub const MAX_SQL_CHARS: usize = 10_000;
/// Longest link handed to the OS opener (Windows command lines stop at 32,767 characters).
const MAX_URL_BYTES: usize = 30_000;
/// How long a token stays valid: enough for SayQL to start when it was closed.
const TOKEN_TTL: Duration = Duration::from_secs(120);
const TOKEN_DIR: &str = "handoffs";

/// Fields that look like secrets. A handoff never carries one.
const SECRET_WORDS: [&str; 6] = ["password", "passwd", "pwd", "secret", "token", "key"];

/// The New connection form's fields, minus the password.
#[derive(Debug, Clone, PartialEq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionDraft {
    pub name: String,
    pub kind: Kind,
    pub host: String,
    pub port: Option<u16>,
    pub database: String,
    pub user: String,
    /// SQLite database file path.
    pub file: String,
    /// "Require SSL" / "Encrypt connection"; `None` keeps the form's default for the kind.
    pub ssl: Option<bool>,
    /// SQL Server: "Trust server certificate".
    pub trust_cert: Option<bool>,
    /// The workspace to save it in: the one the asking AI app serves. `None`: the open one.
    pub workspace: Option<String>,
    /// The group to save it in: the one the asking AI app is limited to.
    pub group: Option<String>,
}

/// SQL to open, unrun, in a new tab.
#[derive(Debug, Clone, PartialEq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct QueryDraft {
    pub connection_id: String,
    pub database: Option<String>,
    pub sql: String,
    pub title: Option<String>,
    pub question: Option<String>,
}

/// A saved script to open, unrun.
#[derive(Debug, Clone, PartialEq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ScriptRef {
    pub script_id: String,
}

/// A SayQL Team invite: the "Sign in to your team" dialog, filled in. The person confirms it.
#[derive(Debug, Clone, PartialEq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct TeamJoin {
    pub server: String,
    pub code: String,
}

/// The end of a sign-in with the company's identity provider: the server's one-time code.
#[derive(Debug, Clone, PartialEq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct TeamSso {
    pub server: String,
    pub code: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, specta::Type)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Action {
    NewConnection(ConnectionDraft),
    OpenQuery(QueryDraft),
    OpenScript(ScriptRef),
    TeamJoin(TeamJoin),
    TeamSso(TeamSso),
}

impl Action {
    fn path(&self) -> &'static str {
        match self {
            Action::NewConnection(_) => "new-connection",
            Action::OpenQuery(_) => "open-query",
            Action::OpenScript(_) => "open-script",
            Action::TeamJoin(_) => "team-join",
            Action::TeamSso(_) => "team-sso",
        }
    }
}

/// What the app shows, and which AI app asked (its MCP client name), if known.
#[derive(Debug, Clone, PartialEq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct Handoff {
    pub source: Option<String>,
    #[serde(flatten)]
    pub action: Action,
}

/// True when a field name looks like it holds a password or other secret.
pub fn is_secret_field(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    SECRET_WORDS.iter().any(|w| n.contains(w))
}

pub fn secret_refused() -> String {
    "SayQL never takes passwords or other secrets over MCP. Leave it out: the person types the \
     password in SayQL's New connection dialog."
        .into()
}

/// One line of text: trimmed, no control characters, at most `max` characters.
pub fn text(field: &str, value: &str, max: usize) -> Result<String, String> {
    let v = value.trim();
    if v.chars().any(char::is_control) {
        return Err(format!("`{field}` has control characters."));
    }
    if v.chars().count() > max {
        return Err(format!("`{field}` is longer than {max} characters."));
    }
    Ok(v.to_string())
}

fn optional_text(field: &str, value: Option<&str>, max: usize) -> Result<Option<String>, String> {
    Ok(value
        .map(|v| text(field, v, max))
        .transpose()?
        .filter(|v| !v.is_empty()))
}

pub fn sql(value: &str) -> Result<String, String> {
    if value.trim().is_empty() {
        return Err("`sql` is empty.".into());
    }
    if value
        .chars()
        .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
    {
        return Err("`sql` has control characters.".into());
    }
    if value.chars().count() > MAX_SQL_CHARS {
        return Err(format!("`sql` is longer than {MAX_SQL_CHARS} characters."));
    }
    Ok(value.to_string())
}

pub fn kind(value: &str) -> Result<Kind, String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "postgres" | "postgresql" => Ok(Kind::Postgres),
        "mysql" | "mariadb" => Ok(Kind::Mysql),
        "sqlite" => Ok(Kind::Sqlite),
        "sqlserver" | "mssql" | "azuresql" => Ok(Kind::Mssql),
        other => Err(format!(
            "Unknown `type` {other:?}: use postgres, mysql, sqlite or sqlserver."
        )),
    }
}

fn kind_name(k: Kind) -> &'static str {
    match k {
        Kind::Postgres => "postgres",
        Kind::Mysql => "mysql",
        Kind::Sqlite => "sqlite",
        Kind::Mssql => "sqlserver",
    }
}

pub fn port(value: i64) -> Result<u16, String> {
    u16::try_from(value)
        .ok()
        .filter(|p| *p > 0)
        .ok_or_else(|| format!("`port` {value} is not between 1 and 65535."))
}

/// A connection id from list_connections: letters, digits, `-` and `_`.
pub fn connection_id(value: &str) -> Result<String, String> {
    saved_id(value).ok_or_else(|| {
        format!(
            "`connection_id` {value:?} is not a SayQL connection id; use one from list_connections."
        )
    })
}

/// A script id from list_scripts.
pub fn script_id(value: &str) -> Result<String, String> {
    saved_id(value).ok_or_else(|| {
        format!("`script_id` {value:?} is not a SayQL script id; use one from list_scripts.")
    })
}

fn saved_id(value: &str) -> Option<String> {
    let v = value.trim();
    let ok = !v.is_empty()
        && v.len() <= 64
        && v.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    ok.then(|| v.to_string())
}

/// The AI app's name as shown in SayQL ("From Claude (MCP)").
pub fn source(value: Option<&str>) -> Option<String> {
    let v: String = value?
        .chars()
        .filter(|c| !c.is_control())
        .take(40)
        .collect();
    Some(v.trim().to_string()).filter(|v| !v.is_empty())
}

/// The connection fields, as given by the AI app (`get` returns a field's raw value).
pub fn connection_draft(
    get: impl Fn(&str) -> Option<String>,
    port_value: Option<i64>,
    flag: impl Fn(&str) -> Result<Option<bool>, String>,
) -> Result<ConnectionDraft, String> {
    let kind =
        kind(&get("type").ok_or("`type` is required: postgres, mysql, sqlite or sqlserver.")?)?;
    let field = |name: &str, max: usize| text(name, &get(name).unwrap_or_default(), max);
    let draft = ConnectionDraft {
        name: field("name", 100)?,
        kind,
        host: field("host", 255)?,
        port: port_value.map(port).transpose()?,
        database: field("database", 128)?,
        user: field("user", 128)?,
        file: field("file", 1024)?,
        ssl: flag("ssl")?,
        trust_cert: if kind == Kind::Mssql {
            flag("trust_server_certificate")?
        } else {
            None
        },
        workspace: get("workspace").filter(|w| !w.is_empty()),
        group: get("group").filter(|g| !g.is_empty()),
    };
    if kind == Kind::Sqlite && draft.file.is_empty() {
        return Err("`file` (the SQLite database file path) is required for sqlite.".into());
    }
    // A URL or `user:password@host` in a field would carry a password into the link.
    if draft.host.contains("://")
        || draft.host.contains('@')
        || draft.host.contains('/')
        || draft.host.chars().any(char::is_whitespace)
    {
        return Err(
            "`host` is only the server's name or IP address (for SQL Server, `host,port` also works): not a URL, \
             and never a user or password."
                .into(),
        );
    }
    if draft.user.contains(':') || draft.user.contains("://") || draft.database.contains("://") {
        return Err(
            "`user` and `database` are plain names: never a URL or a password. The user types the password in SayQL."
                .into(),
        );
    }
    Ok(draft)
}

pub fn query_draft(
    connection: &str,
    database: Option<&str>,
    sql_text: &str,
    title: Option<&str>,
    question: Option<&str>,
) -> Result<QueryDraft, String> {
    Ok(QueryDraft {
        connection_id: connection_id(connection)?,
        database: optional_text("database", database, 128)?,
        sql: sql(sql_text)?,
        title: optional_text("title", title, 100)?,
        question: optional_text("question", question, 1000)?,
    })
}

/// The `sayql://` link for `handoff`, carrying `token`.
pub fn to_url(handoff: &Handoff, token: &str) -> Result<String, String> {
    let mut url =
        Url::parse(&format!("{SCHEME}://{}", handoff.action.path())).map_err(|e| e.to_string())?;
    {
        let mut q = url.query_pairs_mut();
        q.append_pair("token", token);
        if let Some(s) = &handoff.source {
            q.append_pair("from", s);
        }
        let mut put = |k: &str, v: &str| {
            if !v.is_empty() {
                q.append_pair(k, v);
            }
        };
        match &handoff.action {
            Action::NewConnection(c) => {
                put("type", kind_name(c.kind));
                put("name", &c.name);
                put("host", &c.host);
                put("port", &c.port.map(|p| p.to_string()).unwrap_or_default());
                put("database", &c.database);
                put("user", &c.user);
                put("file", &c.file);
                put("ssl", &c.ssl.map(|b| b.to_string()).unwrap_or_default());
                put("workspace", c.workspace.as_deref().unwrap_or_default());
                put("group", c.group.as_deref().unwrap_or_default());
                put(
                    "trust_server_certificate",
                    &c.trust_cert.map(|b| b.to_string()).unwrap_or_default(),
                );
            }
            Action::OpenQuery(qd) => {
                put("connection_id", &qd.connection_id);
                put("database", qd.database.as_deref().unwrap_or_default());
                put("sql", &qd.sql);
                put("title", qd.title.as_deref().unwrap_or_default());
                put("question", qd.question.as_deref().unwrap_or_default());
            }
            Action::OpenScript(s) => put("script_id", &s.script_id),
            Action::TeamJoin(t) => {
                put("server", &t.server);
                put("code", &t.code);
            }
            Action::TeamSso(t) => {
                put("server", &t.server);
                put("code", &t.code);
            }
        }
    }
    let url = String::from(url);
    if url.len() > MAX_URL_BYTES {
        return Err("This is too long to hand to SayQL. Shorten the SQL.".into());
    }
    Ok(url)
}

/// Reads a `sayql://` link: the handoff and its token. Rejects anything unexpected.
pub fn parse_url(link: &str) -> Result<(Handoff, String), String> {
    if link.len() > MAX_URL_BYTES {
        return Err("Link too long.".into());
    }
    let url = Url::parse(link).map_err(|e| format!("Not a link: {e}"))?;
    if url.scheme() != SCHEME {
        return Err(format!("Not a {SCHEME}:// link."));
    }
    let mut params: HashMap<String, String> = HashMap::new();
    for (k, v) in url.query_pairs() {
        if is_secret_field(&k) && k != "token" {
            return Err(secret_refused());
        }
        if params.insert(k.to_string(), v.to_string()).is_some() {
            return Err(format!("`{k}` is given twice."));
        }
    }
    let allowed: &[&str] = match url.host_str() {
        Some("new-connection") => &[
            "token",
            "from",
            "type",
            "name",
            "host",
            "port",
            "database",
            "user",
            "file",
            "ssl",
            "trust_server_certificate",
            "workspace",
            "group",
        ],
        Some("open-query") => &[
            "token",
            "from",
            "connection_id",
            "database",
            "sql",
            "title",
            "question",
        ],
        Some("open-script") => &["token", "from", "script_id"],
        _ => return Err("Unknown SayQL link.".into()),
    };
    if !matches!(url.path(), "" | "/") || url.fragment().is_some() {
        return Err("Unknown SayQL link.".into());
    }
    if let Some(k) = params.keys().find(|k| !allowed.contains(&k.as_str())) {
        return Err(format!("Unexpected `{k}` in the link."));
    }
    let token = params
        .get("token")
        .cloned()
        .ok_or("The link has no token.")?;
    let get = |k: &str| params.get(k).cloned();
    let action = if url.host_str() == Some("open-script") {
        Action::OpenScript(ScriptRef {
            script_id: script_id(&get("script_id").unwrap_or_default())?,
        })
    } else if url.host_str() == Some("new-connection") {
        let port_value = match get("port") {
            Some(p) => Some(
                p.parse::<i64>()
                    .map_err(|_| format!("`port` {p:?} is not a number."))?,
            ),
            None => None,
        };
        let flag = |k: &str| match get(k).as_deref() {
            None => Ok(None),
            Some("true") => Ok(Some(true)),
            Some("false") => Ok(Some(false)),
            Some(v) => Err(format!("`{k}` must be true or false, not {v:?}.")),
        };
        Action::NewConnection(connection_draft(get, port_value, flag)?)
    } else {
        let p = |k: &str| params.get(k).map(String::as_str);
        Action::OpenQuery(query_draft(
            p("connection_id").unwrap_or(""),
            p("database"),
            p("sql").unwrap_or(""),
            p("title"),
            p("question"),
        )?)
    };
    Ok((
        Handoff {
            source: source(get("from").as_deref()),
            action,
        },
        token,
    ))
}

/// Reads a `sayql://team-join?server=…&code=…` invite link, sent by a team admin. `None` for
/// any other link. Unlike the other links it needs no token: it only fills in the sign-in
/// dialog, where the person sees the server and decides; nothing is sent until they confirm.
pub fn parse_team_join(link: &str) -> Option<Result<Handoff, String>> {
    let (server, code) = match team_link(link, "team-join")? {
        Ok(p) => p,
        Err(e) => return Some(Err(e)),
    };
    Some((|| {
        Ok(Handoff {
            source: None,
            action: Action::TeamJoin(TeamJoin {
                server: crate::team::clean_server(&server)?,
                code: crate::team::clean_code(&code)?,
            }),
        })
    })())
}

/// Reads a `sayql://team-sso?server=…&code=…` link, where the team server's sign-in page
/// sends the browser back. `None` for any other link. Anyone can make one, so it's only acted
/// on while SayQL is waiting for a sign-in it started, and the code works only with that
/// sign-in's PKCE verifier.
pub fn parse_team_sso(link: &str) -> Option<Result<Handoff, String>> {
    let (server, code) = match team_link(link, "team-sso")? {
        Ok(p) => p,
        Err(e) => return Some(Err(e)),
    };
    Some((|| {
        Ok(Handoff {
            source: None,
            action: Action::TeamSso(TeamSso {
                server: crate::team::clean_server(&server)?,
                code: crate::team::clean_sso_code(&code)?,
            }),
        })
    })())
}

/// The `server` and `code` of a `sayql://{host}?server=…&code=…` link.
fn team_link(link: &str, host: &str) -> Option<Result<(String, String), String>> {
    let url = Url::parse(link).ok()?;
    if url.scheme() != SCHEME || url.host_str() != Some(host) {
        return None;
    }
    Some((|| {
        if link.len() > 2048 || !matches!(url.path(), "" | "/") || url.fragment().is_some() {
            return Err("Invalid team link.".to_string());
        }
        let mut server = None;
        let mut code = None;
        for (k, v) in url.query_pairs() {
            let slot = match k.as_ref() {
                "server" => &mut server,
                "code" => &mut code,
                other => return Err(format!("Unexpected `{other}` in the link.")),
            };
            if slot.replace(v.to_string()).is_some() {
                return Err(format!("`{k}` is given twice."));
            }
        }
        Ok((server.unwrap_or_default(), code.unwrap_or_default()))
    })())
}

fn token_dir(config_dir: &Path) -> PathBuf {
    config_dir.join(TOKEN_DIR)
}

fn valid_token(token: &str) -> bool {
    token.len() == 32
        && token
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
}

/// Writes a one-time token into the app's config folder (the MCP side).
pub fn issue_token(config_dir: &Path) -> Result<String, String> {
    let dir = token_dir(config_dir);
    std::fs::create_dir_all(&dir).map_err(|e| format!("Could not reach SayQL's folder: {e}"))?;
    let token = uuid::Uuid::new_v4().simple().to_string();
    std::fs::write(dir.join(&token), b"")
        .map_err(|e| format!("Could not reach SayQL's folder: {e}"))?;
    Ok(token)
}

/// Uses up `token` (the app side): true only for a fresh token issued by `issue_token`.
/// Also clears out expired ones.
pub fn redeem_token(config_dir: &Path, token: &str) -> bool {
    let dir = token_dir(config_dir);
    let fresh = |p: &Path| {
        std::fs::metadata(p)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| SystemTime::now().duration_since(t).ok())
            .is_some_and(|age| age <= TOKEN_TTL)
    };
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for e in entries.flatten() {
            if !fresh(&e.path()) {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
    if !valid_token(token) {
        return false;
    }
    let path = dir.join(token);
    // Only the first redeem succeeds, even if the OS delivers the link twice.
    fresh(&path) && std::fs::remove_file(&path).is_ok()
}

/// Hands the link to the OS, which opens SayQL (starting it if needed).
pub fn open_in_app(link: &str) -> Result<(), String> {
    open::that_detached(link)
        .map_err(|e| format!("Could not open SayQL: {e}. Is the SayQL app installed?"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conn_args(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let m: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |k| m.get(k).cloned()
    }

    fn no_flags(_: &str) -> Result<Option<bool>, String> {
        Ok(None)
    }

    fn pg() -> Handoff {
        let get = conn_args(&[
            ("type", "postgresql"),
            ("name", "Prod"),
            ("host", "db.example.com"),
            ("database", "shop"),
            ("user", "reader"),
        ]);
        let draft =
            connection_draft(get, Some(5433), |k| Ok((k == "ssl").then_some(true))).unwrap();
        Handoff {
            source: Some("Claude".into()),
            action: Action::NewConnection(draft),
        }
    }

    #[test]
    fn validates_connection_fields() {
        let Action::NewConnection(c) = pg().action else {
            panic!()
        };
        assert_eq!(
            (c.kind, c.port, c.ssl, c.trust_cert),
            (Kind::Postgres, Some(5433), Some(true), None)
        );
        assert!(kind("oracle").is_err());
        assert!(port(0).is_err() && port(70_000).is_err() && port(1433).is_ok());
        assert!(
            connection_draft(conn_args(&[]), None, no_flags)
                .unwrap_err()
                .contains("`type` is required")
        );
        assert!(
            connection_draft(conn_args(&[("type", "sqlite")]), None, no_flags)
                .unwrap_err()
                .contains("`file`")
        );
        assert!(
            connection_draft(
                conn_args(&[("type", "mysql"), ("host", "a\u{7}b")]),
                None,
                no_flags
            )
            .is_err()
        );
        assert!(
            connection_draft(
                conn_args(&[("type", "mysql"), ("host", &"h".repeat(300))]),
                None,
                no_flags
            )
            .is_err()
        );
    }

    #[test]
    fn validates_query_fields() {
        assert!(query_draft("demo", None, "select 1", Some("Top"), None).is_ok());
        assert!(query_draft("../etc", None, "select 1", None, None).is_err());
        assert!(query_draft("demo", None, "  ", None, None).is_err());
        assert!(query_draft("demo", None, "select 1\0", None, None).is_err());
        assert!(query_draft("demo", None, &"x".repeat(MAX_SQL_CHARS + 1), None, None).is_err());
        let q = query_draft("demo", Some(" "), "select\n\t1", Some(""), None).unwrap();
        assert_eq!(
            (q.database, q.title, q.sql.as_str()),
            (None, None, "select\n\t1")
        );
    }

    #[test]
    fn spots_secret_fields() {
        for k in [
            "password",
            "PASSWORD",
            "db_password",
            "pwd",
            "api_key",
            "secret",
            "access_token",
        ] {
            assert!(is_secret_field(k), "{k}");
        }
        for k in ["host", "user", "database", "ssl"] {
            assert!(!is_secret_field(k), "{k}");
        }
    }

    #[test]
    fn links_round_trip() {
        let token = "0123456789abcdef0123456789abcdef";
        let link = to_url(&pg(), token).unwrap();
        assert!(link.starts_with("sayql://new-connection?token="), "{link}");
        assert_eq!(parse_url(&link).unwrap(), (pg(), token.to_string()));

        let query = Handoff {
            source: None,
            action: Action::OpenQuery(
                query_draft(
                    "demo",
                    None,
                    "select * from orders\nwhere total > 10 & 1=1 -- #?",
                    Some("Big"),
                    None,
                )
                .unwrap(),
            ),
        };
        let link = to_url(&query, token).unwrap();
        assert_eq!(parse_url(&link).unwrap(), (query, token.to_string()));

        let script = Handoff {
            source: Some("Cursor".into()),
            action: Action::OpenScript(ScriptRef {
                script_id: "s-1".into(),
            }),
        };
        let link = to_url(&script, token).unwrap();
        assert!(link.starts_with("sayql://open-script?"), "{link}");
        assert_eq!(parse_url(&link).unwrap(), (script, token.to_string()));
    }

    #[test]
    fn rejects_bad_links() {
        let ok = "sayql://open-query?token=t&connection_id=demo&sql=select+1";
        assert!(parse_url(ok).is_ok());
        for bad in [
            "https://open-query?token=t&connection_id=demo&sql=select+1",
            "sayql://run-query?token=t&connection_id=demo&sql=select+1",
            "sayql://open-query/x?token=t&connection_id=demo&sql=select+1",
            "sayql://open-query?connection_id=demo&sql=select+1",
            "sayql://open-query?token=t&connection_id=demo&sql=select+1&sql=drop+table+x",
            "sayql://open-query?token=t&connection_id=demo&sql=select+1&autorun=true",
            "sayql://open-query?token=t&connection_id=demo&sql=",
            "sayql://new-connection?token=t&type=postgres&host=h&password=hunter2",
            "sayql://new-connection?token=t&type=postgres&port=abc",
            "sayql://new-connection?token=t&type=postgres&ssl=yes",
            "sayql://open-script?token=t",
            "sayql://open-script?token=t&script_id=../x",
            "sayql://open-script?token=t&script_id=s1&sql=select+1",
        ] {
            assert!(parse_url(bad).is_err(), "{bad}");
        }
        assert_eq!(
            parse_url("sayql://new-connection?token=t&type=mysql&pwd=x").unwrap_err(),
            secret_refused()
        );
    }

    #[test]
    fn tokens_are_single_use() {
        let dir = std::env::temp_dir().join(format!("sayql-handoff-{}", uuid::Uuid::new_v4()));
        let token = issue_token(&dir).unwrap();
        assert!(valid_token(&token));
        assert!(!redeem_token(&dir, "0123456789abcdef0123456789abcdef"));
        assert!(!redeem_token(&dir, "../../etc/passwd"));
        assert!(redeem_token(&dir, &token));
        assert!(!redeem_token(&dir, &token));
        let _ = std::fs::remove_dir_all(dir);
    }
}
