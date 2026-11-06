//! The team client against a small fake SayQL server that follows api-v1.md.

use super::*;
use crate::state::{AppState, Target};
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// One request the fake server saw.
#[derive(Debug, Clone)]
pub(crate) struct Seen {
    pub method: String,
    pub path: String,
    pub auth: Option<String>,
    pub body: Value,
}

/// A fake team server on a free local port. `reply(method, path, body)` gives the status and
/// JSON to answer with.
pub(crate) struct Fake {
    pub url: String,
    pub seen: Arc<Mutex<Vec<Seen>>>,
}

pub(crate) type Reply = fn(&str, &str, &Value) -> (u16, Value);

pub(crate) async fn fake(reply: Reply) -> Fake {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else {
                return;
            };
            let log = log.clone();
            tokio::spawn(async move {
                let mut buf = Vec::new();
                let mut chunk = [0u8; 4096];
                let head_end = loop {
                    let n = sock.read(&mut chunk).await.unwrap_or(0);
                    if n == 0 {
                        return;
                    }
                    buf.extend_from_slice(&chunk[..n]);
                    if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                        break i + 4;
                    }
                };
                let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
                let mut lines = head.lines();
                let mut first = lines.next().unwrap_or("").split(' ');
                let method = first.next().unwrap_or("").to_string();
                let path = first.next().unwrap_or("").to_string();
                let header = |name: &str| {
                    head.lines().find_map(|l| {
                        let (k, v) = l.split_once(':')?;
                        k.eq_ignore_ascii_case(name).then(|| v.trim().to_string())
                    })
                };
                let len: usize = header("content-length")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(0);
                while buf.len() < head_end + len {
                    let n = sock.read(&mut chunk).await.unwrap_or(0);
                    if n == 0 {
                        break;
                    }
                    buf.extend_from_slice(&chunk[..n]);
                }
                let body: Value = serde_json::from_slice(&buf[head_end..]).unwrap_or(Value::Null);
                log.lock().unwrap().push(Seen {
                    method: method.clone(),
                    path: path.clone(),
                    auth: header("authorization"),
                    body: body.clone(),
                });
                let (status, json) = reply(&method, &path, &body);
                let text = if status == 204 {
                    String::new()
                } else {
                    json.to_string()
                };
                let out = format!(
                    "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{text}",
                    text.len()
                );
                let _ = sock.write_all(out.as_bytes()).await;
            });
        }
    });
    Fake { url, seen }
}

pub(crate) fn err(status: u16, code: &str) -> (u16, Value) {
    (
        status,
        json!({ "error": { "code": code, "message": format!("The server says {code}.") } }),
    )
}

/// The contract's happy path, with the auth checked on every route but the open two.
pub(crate) fn server(method: &str, path: &str, body: &Value) -> (u16, Value) {
    let sql = body["sql"].as_str().unwrap_or("").to_ascii_lowercase();
    match (method, path) {
        // The server, not the app, refuses writes, so the attempt is in its audit log.
        ("POST", p) if p.ends_with("/query") && !sql.starts_with("select") => {
            err(422, "write_refused")
        }
        ("POST", "/api/v1/auth/redeem") => (
            200,
            json!({ "token": "tok-1", "user": { "id": "u_1", "email": "ana@acme.test", "name": "Ana", "role": "member" } }),
        ),
        ("GET", "/api/v1/me") => (
            200,
            json!({ "user": { "id": "u_1", "email": "ana@acme.test", "name": "Ana", "role": "member" },
                    "server": { "name": "SayQL server", "version": "0.1.0", "api": 1, "licensed": true, "plan": "team",
                                "licensee": "Acme", "expires": "2027-04-30", "users": 3, "user_limit": 10 } }),
        ),
        ("POST", "/api/v1/auth/signout") => (204, Value::Null),
        ("GET", "/api/v1/connections") => (
            200,
            json!([{ "id": "c_1", "name": "Production", "engine": "postgres", "database": "app", "agent_access": "counts",
                     "never_send": ["patients.ssn"], "notes": "Read replica" },
                   { "id": "c_2", "name": "Mainframe", "engine": "db2", "database": "x", "agent_access": "off", "never_send": [] }]),
        ),
        ("GET", "/api/v1/connections/c_1/schema") => (
            200,
            json!({ "tables": [{ "schema": "public", "name": "patients",
                                 "columns": [{ "name": "id", "type": "uuid", "nullable": false }, { "name": "ssn", "type": "text", "nullable": true }] }] }),
        ),
        ("POST", "/api/v1/connections/c_1/query") => (
            200,
            json!({ "columns": [{ "name": "id", "type": "int4" }, { "name": "ssn", "type": "text" }], "rows": [[1], [2]], "row_count": 2,
                    "truncated": false, "elapsed_ms": 12, "withheld": ["ssn"], "audit_id": "a_1" }),
        ),
        ("POST", "/api/v1/connections/c_never/query") => err(400, "never_send"),
        ("POST", "/api/v1/connections/c_nolic/query") => err(402, "no_licence"),
        ("POST", "/api/v1/connections/c_expired/query") => err(402, "licence_expired"),
        ("POST", "/api/v1/connections/c_db/query") => err(422, "db_error"),
        ("POST", "/api/v1/connections/c_noagent/query") => err(403, "forbidden"),
        ("GET", "/api/v1/revoked") => err(401, "unauthorized"),
        _ => err(404, "not_found"),
    }
}

pub(crate) fn client(url: &str) -> TeamClient {
    TeamClient::new(
        reqwest::Client::new(),
        url.to_string(),
        "tok-1".into(),
        None,
    )
}

fn pool(url: &str, id: &str) -> TeamPool {
    TeamPool {
        client: client(url),
        id: id.into(),
        kind: Kind::Postgres,
        source: Source::Editor,
        question: None,
    }
}

#[tokio::test]
async fn sign_in_status_connections_schema_and_query_follow_the_contract() {
    let f = fake(server).await;
    let (token, account) = redeem(
        &reqwest::Client::new(),
        &f.url,
        "INVITE-1",
        "SayQL on a Mac",
    )
    .await
    .unwrap();
    assert_eq!(token, "tok-1");
    assert_eq!(
        (account.email.as_str(), account.server.as_str()),
        ("ana@acme.test", f.url.as_str())
    );

    let c = client(&f.url);
    let (user, info) = c.status().await.unwrap();
    assert_eq!(
        (user.email.as_str(), info.plan.as_deref(), info.user_limit),
        ("ana@acme.test", Some("team"), Some(10))
    );

    let list = c.connections().await.unwrap();
    assert_eq!(list.len(), 2);
    assert_eq!(
        (list[0].local_id(), list[0].kind()),
        ("team:c_1".to_string(), Some(Kind::Postgres))
    );
    assert_eq!(
        list[1].kind(),
        None,
        "unknown engines are left out of the navigator"
    );
    assert_eq!(
        list[0].rules(),
        TeamRules {
            max_results: Results::Counts,
            never_send: vec!["patients.ssn".into()]
        }
    );

    let p = pool(&f.url, "c_1");
    let schema = p.schema().await.unwrap();
    assert_eq!(schema.dialect, "PostgreSQL");
    assert_eq!(schema.tables[0].qualified_name(), "patients");
    assert_eq!(schema.tables[0].columns[0].data_type, "uuid");

    let asked = TeamPool {
        source: Source::Ask,
        question: Some("How many patients?".into()),
        ..p.clone()
    };
    let r = asked
        .query(
            "select id, ssn from patients",
            5000,
            &CancellationToken::new(),
        )
        .await
        .unwrap();
    assert_eq!(r.columns, ["id"], "a withheld column leaves the header too");
    assert_eq!(
        (r.rows.len(), r.withheld.clone()),
        (2, vec!["ssn".to_string()])
    );

    let seen = f.seen.lock().unwrap().clone();
    let redeem_req = &seen[0];
    assert_eq!(
        redeem_req.body,
        json!({ "code": "INVITE-1", "device": "SayQL on a Mac" })
    );
    assert_eq!(redeem_req.auth, None, "redeeming needs no token");
    assert!(
        seen[1..]
            .iter()
            .all(|s| s.auth.as_deref() == Some("Bearer tok-1")),
        "{seen:?}"
    );
    let q = seen.iter().find(|s| s.path.ends_with("/query")).unwrap();
    assert_eq!(
        q.body,
        json!({ "sql": "select id, ssn from patients", "source": "ask", "question": "How many patients?", "max_rows": 5000 })
    );

    // The editor sends no question; agent mode tags its queries; max_rows stays within the server's cap.
    p.query("select 1", 50_000, &CancellationToken::new())
        .await
        .unwrap();
    let last = f.seen.lock().unwrap().last().cloned().unwrap();
    assert_eq!(
        last.body,
        json!({ "sql": "select 1", "source": "editor", "question": null, "max_rows": 10_000 })
    );

    c.sign_out().await.unwrap();
    let out = f.seen.lock().unwrap().last().cloned().unwrap();
    assert_eq!(
        (out.method.as_str(), out.path.as_str()),
        ("POST", "/api/v1/auth/signout")
    );
}

#[tokio::test]
async fn server_errors_become_clear_messages() {
    let f = fake(server).await;
    let msg = |id: &'static str| {
        let p = pool(&f.url, id);
        async move {
            p.query("select 1", 10, &CancellationToken::new())
                .await
                .unwrap_err()
                .to_string()
        }
    };
    // Refusals by the server's rules are marked as such (the UI offers no AI fix); database errors aren't.
    let refused = |e: AppError| matches!(e, AppError::TeamRefused(_));
    let q = |id: &'static str| {
        let p = pool(&f.url, id);
        async move {
            p.query("select 1", 10, &CancellationToken::new())
                .await
                .unwrap_err()
        }
    };
    assert!(
        refused(q("c_never").await) && refused(q("c_nolic").await) && refused(q("c_noagent").await)
    );
    assert!(!refused(q("c_db").await));
    assert_eq!(
        serde_json::to_value(q("c_nolic").await).unwrap()["teamRefused"],
        true
    );
    assert!(serde_json::to_value(q("c_db").await).unwrap().is_string());
    assert!(
        msg("c_never")
            .await
            .starts_with("Your team admin marked a column in this query never send")
    );
    assert!(msg("c_nolic").await.contains("has no licence"));
    assert!(msg("c_expired").await.contains("licence has expired"));
    assert_eq!(
        msg("c_db").await,
        "The server says db_error.",
        "database errors keep the server's message"
    );
    assert_eq!(msg("c_noagent").await, "The server says forbidden.");
    let agent = TeamPool {
        source: Source::Agent,
        ..pool(&f.url, "c_noagent")
    };
    assert_eq!(
        agent
            .query("select 1", 10, &CancellationToken::new())
            .await
            .unwrap_err()
            .to_string(),
        AGENT_OFF
    );
    let revoked: AppResult<Value> = client(&f.url).get(&["revoked"]).await;
    assert_eq!(revoked.unwrap_err().to_string(), SIGNED_OUT);
    assert!(error_text("seat_limit", "").contains("no free seats"));

    // A bad invite code reads as one, whatever code the server picks for it.
    let bad = |_: &str, _: &str, _: &Value| err(401, "unauthorized");
    let f2 = fake(bad).await;
    let e = redeem(&reqwest::Client::new(), &f2.url, "nope", "d")
        .await
        .unwrap_err()
        .to_string();
    assert_eq!(e, BAD_INVITE);
    let full = |_: &str, _: &str, _: &Value| err(403, "seat_limit");
    let f3 = fake(full).await;
    let e = redeem(&reqwest::Client::new(), &f3.url, "x", "d")
        .await
        .unwrap_err()
        .to_string();
    assert!(e.contains("no free seats"), "{e}");
}

/// A write on a team connection isn't stopped in the app: it goes to the server, which refuses
/// it and records the attempt. The editor and Ask box use this path (no confirm dialog first).
#[tokio::test]
async fn writes_go_to_the_server_to_be_refused_and_audited() {
    let f = fake(server).await;
    let p = crate::db::DbPool::Team(pool(&f.url, "c_1"));
    let e = crate::db::run_query_with(
        &p,
        "delete from patients",
        true,
        10,
        &CancellationToken::new(),
    )
    .await
    .unwrap_err();
    assert!(
        matches!(&e, AppError::TeamRefused(m) if m == WRITE_REFUSED),
        "{e}"
    );
    let seen = f.seen.lock().unwrap().clone();
    assert_eq!(seen.last().unwrap().body["sql"], "delete from patients");
    // Agent mode's read-only runner sends it too.
    let agent = p.with_context(Source::Agent, None);
    let e = crate::safety::readonly::run_limited(
        &agent,
        "update patients set ssn = null",
        10,
        Duration::from_secs(5),
    )
    .await
    .unwrap_err();
    assert_eq!(e, WRITE_REFUSED);
    assert_eq!(
        f.seen.lock().unwrap().last().unwrap().body["source"],
        "agent"
    );
}

#[tokio::test]
async fn stop_ends_the_wait_for_a_team_query() {
    // A server that never answers.
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    tokio::spawn(async move {
        let mut held = Vec::new();
        while let Ok((s, _)) = listener.accept().await {
            held.push(s);
        }
    });
    let cancel = CancellationToken::new();
    let stop = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(100)).await;
        stop.cancel();
    });
    let e = pool(&url, "c_1")
        .query("select 1", 10, &cancel)
        .await
        .unwrap_err();
    assert_eq!(e.to_string(), CANCELLED);
}

#[test]
fn server_addresses_are_cleaned_and_https_is_required_off_this_computer() {
    assert_eq!(
        clean_server(" sayql.acme.internal/ ").unwrap(),
        "https://sayql.acme.internal"
    );
    assert_eq!(
        clean_server("https://acme.test/sayql/").unwrap(),
        "https://acme.test/sayql"
    );
    assert_eq!(
        clean_server("http://localhost:8080").unwrap(),
        "http://localhost:8080"
    );
    assert_eq!(
        clean_server("http://127.0.0.1:8080").unwrap(),
        "http://127.0.0.1:8080"
    );
    assert!(
        clean_server("http://sayql.acme.internal")
            .unwrap_err()
            .contains("https://")
    );
    // Half-typed addresses are caught before anything is sent.
    for half in [
        "http:/", "http:", "https:/", "http", "https://", "http://", ":",
    ] {
        assert_eq!(clean_server(half).unwrap_err(), ENTER_SERVER, "{half}");
    }
    for bad in [
        "",
        "ftp://acme.test",
        "https://user:pw@acme.test",
        "https://acme.test/?x=1",
        "not a url at all",
    ] {
        assert!(clean_server(bad).is_err(), "{bad}");
    }
    assert_eq!(clean_code("  ABCD-1234 ").unwrap(), "ABCD-1234");
    assert!(clean_code("").is_err() && clean_code("two words").is_err());
    let url = endpoint("https://acme.test/sayql", &["connections", "c/1", "query"]).unwrap();
    assert_eq!(
        url.as_str(),
        "https://acme.test/sayql/api/v1/connections/c%2F1/query"
    );
}

#[test]
fn the_admin_rules_win() {
    let rules = TeamRules {
        max_results: Results::Counts,
        never_send: vec!["patients.ssn".into()],
    };
    let mine = AgentAccess {
        enabled: true,
        results: Results::Rows,
        ask_before_send: false,
        never_send: vec!["notes".into()],
        ..Default::default()
    };
    let applied = apply_rules(mine.clone(), &rules);
    assert_eq!(
        applied.results,
        Results::Counts,
        "can't be set above the admin's"
    );
    assert_eq!(applied.never_send, ["notes", "patients.ssn"]);
    assert!(
        applied.enabled && !applied.ask_before_send,
        "the person's other choices stay"
    );
    // Lower than the admin's is fine.
    assert_eq!(
        apply_rules(
            AgentAccess {
                results: Results::Off,
                ..mine.clone()
            },
            &rules
        )
        .results,
        Results::Off
    );
    // Off on the server means off, and anything unknown is Off too.
    assert_eq!(max_results("off"), Results::Off);
    assert_eq!(max_results("everything"), Results::Off);
    assert_eq!(max_results("masked_rows"), Results::Rows);

    // Saving keeps only the person's own columns, and an admin column can't be removed.
    let saved = own_access(
        AgentAccess {
            never_send: vec!["PATIENTS.SSN".into(), "notes".into()],
            ..mine
        },
        &rules,
    );
    assert_eq!(
        (saved.results, saved.never_send.clone()),
        (Results::Counts, vec!["notes".to_string()])
    );
    assert!(
        apply_rules(
            AgentAccess {
                never_send: vec![],
                ..saved
            },
            &rules
        )
        .never_send
        .contains(&"patients.ssn".to_string())
    );
}

#[test]
fn team_invite_links_fill_the_dialog_and_nothing_else() {
    let h = crate::handoff::parse_team_join(
        "sayql://team-join?server=https%3A%2F%2Fsayql.acme.internal&code=ABCD",
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        h.action,
        crate::handoff::Action::TeamJoin(crate::handoff::TeamJoin {
            server: "https://sayql.acme.internal".into(),
            code: "ABCD".into()
        })
    );
    assert!(
        crate::handoff::parse_team_join("sayql://open-query?token=x").is_none(),
        "other links go the usual way"
    );
    for bad in [
        "sayql://team-join?server=http://evil.example&code=ABCD",
        "sayql://team-join?server=https://a.test&code=ABCD&sql=drop",
        "sayql://team-join?server=https://a.test&code=A&code=B",
        "sayql://team-join?server=https://a.test",
    ] {
        assert!(
            crate::handoff::parse_team_join(bad).unwrap().is_err(),
            "{bad}"
        );
    }
}

#[test]
fn company_sign_in_links_carry_only_a_server_and_a_code() {
    let code = "sqt_0123456789abcdefABCDEF-_xyz";
    let h = crate::handoff::parse_team_sso(&format!(
        "sayql://team-sso?server=https%3A%2F%2Fsayql.acme.internal&code={code}"
    ))
    .unwrap()
    .unwrap();
    assert_eq!(
        h.action,
        crate::handoff::Action::TeamSso(crate::handoff::TeamSso {
            server: "https://sayql.acme.internal".into(),
            code: code.into()
        })
    );
    assert!(crate::handoff::parse_team_sso("sayql://team-join?server=a&code=b").is_none());
    for bad in [
        "sayql://team-sso?server=http://evil.example&code=sqt_0123456789abcdef",
        "sayql://team-sso?server=https://a.test&code=short",
        "sayql://team-sso?server=https://a.test&code=sqt_0123456789abcdef%20x",
        "sayql://team-sso?server=https://a.test&code=sqt_0123456789abcdef&token=x",
        "sayql://team-sso?server=https://a.test&code=sqt_0123456789abcdef#x",
    ] {
        assert!(
            crate::handoff::parse_team_sso(bad).unwrap().is_err(),
            "{bad}"
        );
    }
}

#[test]
fn company_sign_in_uses_pkce() {
    use base64::Engine;
    use sha2::{Digest, Sha256};
    let (verifier, challenge) = pkce_pair();
    assert_eq!(verifier.len(), 64);
    assert_eq!(challenge.len(), 43);
    assert_eq!(
        challenge,
        base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(Sha256::digest(verifier.as_bytes()))
    );
    assert_ne!(pkce_pair().0, verifier, "a new verifier each time");
    let url = sso_start_url(
        "https://sayql.acme.internal/team",
        &challenge,
        "SayQL on Ana's Mac",
    )
    .unwrap();
    assert_eq!(
        url,
        format!(
            "https://sayql.acme.internal/team/auth/sso/start?target=app&challenge={challenge}&device=SayQL+on+Ana%27s+Mac"
        )
    );
}

#[tokio::test]
async fn company_sign_in_redeems_with_the_verifier() {
    let reply = |method: &str, path: &str, body: &Value| match (method, path) {
        ("GET", "/api/v1/auth/sso") => (
            200,
            json!({ "enabled": true, "provider": "okta", "require_sso": false }),
        ),
        ("POST", "/api/v1/auth/sso/redeem") if body["verifier"] == "v1" && body["code"] == "c1" => {
            (
                200,
                json!({ "token": "tok-sso", "user": { "id": "u_1", "email": "ana@acme.test", "name": "Ana", "role": "admin" }, "expires_in_days": 7 }),
            )
        }
        ("POST", "/api/v1/auth/sso/redeem") => (
            401,
            json!({ "error": { "code": "unauthorized", "message": "This sign-in was started somewhere else. Sign in again from SayQL." } }),
        ),
        ("POST", "/api/v1/auth/redeem") => (
            403,
            json!({ "error": { "code": "forbidden", "message": "This team signs in with your company account." } }),
        ),
        _ => err(404, "not_found"),
    };
    let f = fake(reply).await;
    let http = reqwest::Client::new();
    assert!(sso_info(&http, &f.url).await.unwrap().enabled);
    let (token, account) = sso_redeem(&http, &f.url, "c1", "v1", "d").await.unwrap();
    assert_eq!(
        (token.as_str(), account.role.as_str()),
        ("tok-sso", "admin")
    );
    assert_eq!(
        sso_redeem(&http, &f.url, "c1", "stolen", "d")
            .await
            .unwrap_err()
            .to_string(),
        SSO_EXPIRED
    );
    // With the company sign-in required, an invite says so instead of "isn't valid".
    assert_eq!(
        redeem(&http, &f.url, "ABCD", "d")
            .await
            .unwrap_err()
            .to_string(),
        "This team signs in with your company account."
    );
    // A server before 0.4.0 has no company sign-in.
    let old = fake(|_: &str, _: &str, _: &Value| err(404, "not_found")).await;
    assert!(!sso_info(&http, &old.url).await.unwrap().enabled);
}

/// Without sign-in, the app behaves as before: no team server is known, so nothing reads the
/// keychain for a token or sends anything anywhere, and a `team:` id can't reach the network.
#[tokio::test]
async fn signed_out_nothing_is_sent_anywhere() {
    let dir = std::env::temp_dir().join(format!("sayql-team-{}", uuid::Uuid::new_v4()));
    let state = AppState::new(dir.join("config"), dir.join("data")).unwrap();
    assert!(state.store.settings().unwrap().team.is_none());
    // `team_client` checks settings before the keychain; `None` means no request can be built.
    assert!(state.team_client().unwrap().is_none());
    let e = state
        .target_pool(&Target::default_of("team:c_1"))
        .await
        .err()
        .unwrap();
    assert_eq!(e.to_string(), SIGNED_OUT);
    assert!(state.check_connection("team:c_1").is_err());
    assert!(
        state
            .load_schema(&Target::default_of("team:c_1"))
            .await
            .is_err()
    );
    // Settings saved by a signed-out app have no team entry at all.
    state
        .store
        .update_settings(|s| {
            s.onboarded = true;
            Ok(())
        })
        .unwrap();
    let saved = std::fs::read_to_string(dir.join("config/settings.json")).unwrap();
    assert!(!saved.contains("team"), "{saved}");
    let _ = std::fs::remove_dir_all(dir);
}

/// Answers every query with rows that don't line up with their one column.
fn misshapen(_: &str, path: &str, _: &Value) -> (u16, Value) {
    match path {
        "/api/v1/connections/c_short/query" => {
            (200, json!({ "columns": [{ "name": "x" }], "rows": [[]] }))
        }
        "/api/v1/connections/c_long/query" => (
            200,
            json!({ "columns": [{ "name": "x" }], "rows": [[true, 1]] }),
        ),
        "/api/v1/connections/c_mixed/query" => (
            200,
            json!({ "columns": [{ "name": "a" }, { "name": "b" }], "rows": [[1, 2], [3]] }),
        ),
        _ => err(404, "not_found"),
    }
}

/// Every caller of the AI query gate gets the database limit, on any pool, and nothing reaches
/// the server.
#[tokio::test]
async fn the_gate_refuses_a_query_outside_the_database_limit_before_the_server_sees_it() {
    use crate::safety::databases::DbLimit;
    use crate::safety::gate::{self, Refusal};
    let f = fake(misshapen).await;
    let team = crate::db::DbPool::Team(pool(&f.url, "c_short"));
    let limit = DbLimit::new(Some(vec!["acme".into()]), vec![]);
    for opts in [gate::MCP, gate::AGENT, gate::NOTE_CHECK] {
        let refused = gate::prepare(&team, "SELECT datname FROM pg_database", &[], &limit, opts);
        assert!(matches!(refused.await, Err(Refusal::Database(_))));
    }
    let check = crate::notes::run_check(&team, "SELECT count(*) > 0 FROM pg_database", &[], &limit);
    assert_eq!(
        check.await.error.as_deref(),
        Some(crate::safety::databases::REFUSED)
    );
}

/// Code downstream reads cells by position (a note's check reads `rows[0][0]`), so a reply
/// whose rows are wider or narrower than its columns used to abort the app.
#[tokio::test]
async fn replies_whose_rows_dont_match_their_columns_are_refused() {
    let f = fake(misshapen).await;
    for id in ["c_short", "c_long", "c_mixed"] {
        let e = pool(&f.url, id)
            .query("select x", 10, &CancellationToken::new())
            .await
            .unwrap_err();
        assert_eq!(e.to_string(), MISSHAPEN, "{id}");
    }
    let check = crate::notes::run_check(
        &crate::db::DbPool::Team(pool(&f.url, "c_short")),
        "select true",
        &[],
        &Default::default(),
    )
    .await;
    assert!(
        check.ok.is_none() && check.error.as_deref() == Some(MISSHAPEN),
        "{check:?}"
    );
}

/// Against a real SayQL server: `SAYQL_TEST_TEAM_URL=http://localhost:8080 SAYQL_TEST_TEAM_CODE=…
/// cargo test --lib team::tests::real_server -- --ignored`. The server needs a licence, an unused
/// invite, a SQLite connection named Clinic over a `patients(id, name, ssn)` table with
/// `agent_access counts` and never-send `patients.ssn`, and one named NoAgent with agent access off.
#[tokio::test]
#[ignore]
async fn real_server() {
    use crate::db::{DbPool, introspect, run_query_with};
    let (Ok(url), Ok(code)) = (
        std::env::var("SAYQL_TEST_TEAM_URL"),
        std::env::var("SAYQL_TEST_TEAM_CODE"),
    ) else {
        return;
    };
    let url = clean_server(&url).unwrap();
    let http = reqwest::Client::new();
    let (token, account) = redeem(&http, &url, &code, &device_name()).await.unwrap();
    assert!(!account.email.is_empty());
    assert_eq!(
        redeem(&http, &url, &code, "again")
            .await
            .unwrap_err()
            .to_string(),
        BAD_INVITE,
        "an invite works once"
    );
    let c = TeamClient::new(http, url.clone(), token, None);
    let (_, info) = c.status().await.unwrap();
    assert!(info.licensed, "{info:?}");

    let list = c.connections().await.unwrap();
    let clinic = list
        .iter()
        .find(|x| x.name == "Clinic")
        .expect("a connection named Clinic");
    assert_eq!(
        clinic.rules(),
        TeamRules {
            max_results: Results::Counts,
            never_send: vec!["patients.ssn".into()]
        }
    );
    let team = TeamPool {
        client: c.clone(),
        id: clinic.id.clone(),
        kind: clinic.kind().unwrap(),
        source: Source::Editor,
        question: None,
    };
    let pool = DbPool::Team(team.clone());
    let schema = introspect(&pool, pool.kind()).await.unwrap();
    assert!(
        schema.tables.iter().any(|t| t.name == "patients"),
        "{schema:?}"
    );

    // The editor path, and Ask with its question.
    let none = CancellationToken::new();
    let r = run_query_with(
        &pool,
        "select id, name from patients order by id",
        true,
        100,
        &none,
    )
    .await
    .unwrap();
    assert_eq!(
        (r.columns.clone(), r.rows.len()),
        (vec!["id".to_string(), "name".to_string()], 2)
    );
    let asked = pool
        .clone()
        .with_context(Source::Ask, Some("Who are the patients?".into()));
    let r = run_query_with(&asked, "select * from patients", true, 100, &none)
        .await
        .unwrap();
    assert_eq!(
        r.withheld,
        ["ssn"],
        "SELECT * withholds the never-send column"
    );
    assert!(
        !r.columns.iter().any(|c| c == "ssn")
            && r.rows.iter().all(|row| row.len() == r.columns.len()),
        "{r:?}"
    );
    let e = run_query_with(&pool, "select ssn from patients", true, 100, &none)
        .await
        .unwrap_err()
        .to_string();
    assert!(e.starts_with("Your team admin marked a column"), "{e}");
    // A write never runs: the server refuses it too, if it gets past the app.
    let e = team
        .query("delete from patients", 10, &none)
        .await
        .unwrap_err();
    assert_eq!(e.to_string(), WRITE_REFUSED);

    // Agent mode's path; and where the admin turned agent mode off, the reason.
    let agent = pool
        .clone()
        .with_context(Source::Agent, Some("How many patients?".into()));
    let r = crate::safety::readonly::run_limited(
        &agent,
        "select count(*) as n from patients",
        10,
        Duration::from_secs(10),
    )
    .await
    .unwrap();
    assert_eq!(r.rows.len(), 1);
    if let Some(off) = list.iter().find(|x| x.name == "NoAgent") {
        assert!(off.agent_off());
        let p = TeamPool {
            client: c.clone(),
            id: off.id.clone(),
            kind: off.kind().unwrap(),
            source: Source::Agent,
            question: None,
        };
        assert_eq!(
            p.query("select 1", 10, &none)
                .await
                .unwrap_err()
                .to_string(),
            AGENT_OFF
        );
    }

    c.sign_out().await.unwrap();
    assert_eq!(
        c.connections().await.unwrap_err().to_string(),
        SIGNED_OUT,
        "the token is revoked"
    );
}
