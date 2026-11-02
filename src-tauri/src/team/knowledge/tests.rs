//! Shared knowledge against the fake team server (see `team::tests`).

use super::*;
use crate::state::{AppState, Target};
use crate::store::agent::render_notes;
use crate::team::tests::{Fake, Reply, client, err, fake, server};
use crate::team::{ServerInfo, TeamAccount};
use serde_json::json;

/// A 0.5.0 server: `/me` lists `knowledge`, and connection c_1 has three notes (one retired).
pub(crate) fn knowledge_server(method: &str, path: &str, body: &Value) -> (u16, Value) {
    match (method, path) {
        ("GET", "/api/v1/me") => (
            200,
            json!({ "user": { "id": "u_1", "email": "ana@acme.test", "name": "Ana", "role": "member" },
                    "server": { "name": "SayQL server", "version": "0.5.0", "api": 1, "licensed": true,
                                "features": ["knowledge"] } }),
        ),
        ("GET", "/api/v1/connections/c_1/knowledge") => (
            200,
            json!({ "can_edit": false, "notes": [
                { "id": "kn_aaaa1111", "ref": "n_aaaa1111", "connection_id": "c_1", "kind": "note",
                  "text": "orders.status 3 means refunded",
                  "anchors": [{ "table": "orders", "column": "status", "fp": "ab12" }],
                  "source": "ana@acme.test (Investigate)", "evidence": "", "status": "active",
                  "suspect_reason": null, "check_sql": "SELECT count(*) = 0 FROM orders WHERE status > 3",
                  "check_last": { "ok": true, "at": "2026-11-01T10:00:00.000Z" },
                  "verified_at": "2026-11-01T10:00:00.000Z", "owner_user_id": null,
                  "created_by": "admin@acme.test", "created_at": "2026-10-30T09:00:00.000Z",
                  "updated_at": "2026-11-01T10:00:00.000Z" },
                { "id": "kn_bbbb2222", "ref": "n_bbbb2222", "connection_id": "c_1", "kind": "glossary",
                  "text": "users.last_login_at is set on every sign-in",
                  "anchors": [{ "table": "users", "column": "last_login_at" }],
                  "source": "admin@acme.test", "status": "suspect",
                  "suspect_reason": "the column users.last_login_at no longer exists",
                  "check_sql": null, "check_last": null, "verified_at": null,
                  "created_at": "2026-10-30T09:00:00.000Z", "updated_at": "2026-11-02T08:00:00.000Z" },
                { "id": "kn_cccc3333", "ref": "n_cccc3333", "connection_id": "c_1", "kind": "note",
                  "text": "invites rows are deleted once accepted", "anchors": [], "status": "retired" }
            ] }),
        ),
        ("POST", "/api/v1/connections/c_1/knowledge/suggestions") if body["text"].is_string() => (
            201,
            json!({ "id": "s_1", "status": "pending", "text": body["text"] }),
        ),
        ("GET", "/api/v1/connections/c_nolic/knowledge") => err(402, "no_licence"),
        _ => server(method, path, body),
    }
}

fn knowledge_reads(f: &Fake) -> usize {
    f.seen
        .lock()
        .unwrap()
        .iter()
        .filter(|s| s.path.ends_with("/knowledge"))
        .count()
}

fn with_knowledge() -> ServerInfo {
    ServerInfo {
        features: vec![FEATURE.into()],
        ..Default::default()
    }
}

/// The server's notes in the app's shape: doubt with its reason, the check and when it last
/// passed, and the tables they're about. Retired notes are left out.
#[tokio::test]
async fn shared_notes_map_into_the_apps_notes() {
    let f = fake(knowledge_server).await;
    let shared = client(&f.url)
        .knowledge("c_1", "team:c_1")
        .await
        .unwrap()
        .unwrap();
    assert!(!shared.can_edit);
    let texts: Vec<&str> = shared.notes.iter().map(|n| n.text.as_str()).collect();
    assert_eq!(
        texts,
        [
            "orders.status 3 means refunded",
            "users.last_login_at is set on every sign-in"
        ]
    );
    let (refunded, login) = (&shared.notes[0], &shared.notes[1]);
    assert_eq!(refunded.id, "kn_aaaa1111");
    assert_eq!(refunded.connection_id, "team:c_1");
    assert_eq!(refunded.source, "ana@acme.test (Investigate)");
    assert!(refunded.suspect.is_none());
    assert!(refunded.evidence.is_none(), "empty evidence is none");
    // 2026-11-01T10:00:00Z
    assert_eq!(refunded.verified_at, Some(1_793_527_200_000));
    let check = refunded.check.as_ref().unwrap();
    assert_eq!(
        check.sql,
        "SELECT count(*) = 0 FROM orders WHERE status > 3"
    );
    assert_eq!(check.last.as_ref().unwrap().ok, Some(true));
    assert_eq!(refunded.anchors[0].table, "orders");
    assert_eq!(refunded.anchors[0].column.as_deref(), Some("status"));
    assert_eq!(
        login.suspect.as_ref().unwrap().reason,
        "the column users.last_login_at no longer exists"
    );
    assert!(login.check.is_none() && login.verified_at.is_none());

    // The prompts mark them as they mark local notes.
    let text = render_notes(&shared.notes, false);
    assert!(
        text.contains("(may be out of date: the column users.last_login_at no longer exists) users.last_login_at"),
        "{text}"
    );
    assert!(text.contains("(checked "), "{text}");

    // The Ask box picks them by table, with or without the default schema's name.
    let picked = about(&shared.notes, &["dbo.orders".into()]);
    assert_eq!(picked.len(), 1);
    assert_eq!(picked[0].id, "kn_aaaa1111");
    assert!(names("orders", "dbo.orders") && names("orders", "ORDERS"));
    assert!(!names("sales.orders", "orders") && !names("orders", "order_items"));
}

/// The server must list `knowledge`; an older one, or one answering 404 or 402, keeps the
/// local notes. Notes are read about once a minute, and again after a suggestion.
#[tokio::test]
async fn shared_notes_need_the_feature_and_are_cached() {
    // A 0.4 server: no features in /me, so its knowledge route is never asked.
    let old = fake(server).await;
    let cache = Cache::default();
    assert!(cache.notes(&client(&old.url), "team:c_1").await.is_none());
    assert_eq!(knowledge_reads(&old), 0);
    // Personal connections never reach the server.
    assert!(cache.notes(&client(&old.url), "c_1").await.is_none());
    assert!(
        old.seen
            .lock()
            .unwrap()
            .iter()
            .all(|s| s.path == "/api/v1/me")
    );

    // A server that lists the feature but answers 404 or 402 from the route: local notes.
    let cache = Cache::default();
    cache.set_server(&with_knowledge());
    assert!(cache.notes(&client(&old.url), "team:c_1").await.is_none());
    let new = fake(knowledge_server).await;
    assert!(
        cache
            .notes(&client(&new.url), "team:c_nolic")
            .await
            .is_none()
    );

    // A 0.5.0 server: found from /me, read once, then kept for a minute.
    let cache = Cache::default();
    let c = client(&new.url);
    let first = cache.notes(&c, "team:c_1").await.unwrap();
    assert_eq!(first.notes.len(), 2);
    let reads = knowledge_reads(&new);
    assert_eq!(cache.notes(&c, "team:c_1").await.unwrap(), first);
    assert_eq!(knowledge_reads(&new), reads, "kept");
    cache.forget("team:c_1");
    cache.notes(&c, "team:c_1").await.unwrap();
    assert_eq!(
        knowledge_reads(&new),
        reads + 1,
        "read again after a suggestion"
    );

    // The server can't be reached: the last notes read stand in; with none, the local notes.
    let gone = client("http://127.0.0.1:9");
    cache.forget("team:c_1");
    assert!(cache.notes(&gone, "team:c_1").await.is_none());
    let cache = Cache::default();
    cache.set_server(&with_knowledge());
    cache.notes(&c, "team:c_1").await.unwrap();
    {
        // A minute later.
        let mut kept = lock(&cache.notes);
        let at = &mut kept.get_mut("team:c_1").unwrap().0;
        *at = at.checked_sub(FRESH_FOR).unwrap_or(*at);
    }
    assert_eq!(cache.notes(&gone, "team:c_1").await.unwrap(), first);

    // Signing out forgets what the server can do.
    cache.clear();
    assert!(lock(&cache.knowledge).is_none() && lock(&cache.notes).is_empty());
}

#[tokio::test]
async fn suggestions_go_to_the_server() {
    let f = fake(knowledge_server).await;
    let s = Suggestion {
        text: "orders.status 4 means disputed".into(),
        evidence: "Step 2".into(),
        check: Some("SELECT count(*) >= 0 FROM orders".into()),
        replaces: Some("kn_aaaa1111".into()),
        source: Some("Investigate".into()),
        ..Default::default()
    };
    client(&f.url).suggest_knowledge("c_1", &s).await.unwrap();
    let seen = f.seen.lock().unwrap().clone();
    let post = seen.last().unwrap();
    assert_eq!(
        (post.method.as_str(), post.path.as_str()),
        ("POST", "/api/v1/connections/c_1/knowledge/suggestions")
    );
    assert_eq!(post.auth.as_deref(), Some("Bearer tok-1"));
    assert_eq!(
        post.body,
        json!({ "text": "orders.status 4 means disputed", "evidence": "Step 2",
                "check": "SELECT count(*) >= 0 FROM orders", "replaces": "kn_aaaa1111",
                "source": "Investigate" })
    );
    // A person's note sends only its text.
    let mine = Suggestion {
        text: "x is y".into(),
        ..Default::default()
    };
    client(&f.url)
        .suggest_knowledge("c_1", &mine)
        .await
        .unwrap();
    assert_eq!(
        f.seen.lock().unwrap().last().unwrap().body,
        json!({ "text": "x is y" })
    );
    // An older server says so plainly.
    let old = fake(server).await;
    assert!(
        client(&old.url)
            .suggest_knowledge("c_1", &mine)
            .await
            .is_err()
    );
}

/// The Ask box: on a team connection whose server keeps shared notes, the team's notes about
/// the tables shown; on an older server, the notes saved here. (Tests keep the keychain in memory.)
#[tokio::test]
async fn the_ask_box_uses_the_teams_notes() {
    use crate::store::agent::Note;
    for (reply, team) in [(knowledge_server as Reply, true), (server as Reply, false)] {
        let f = fake(reply).await;
        let dir = std::env::temp_dir().join(format!("sayql-team-notes-{}", uuid::Uuid::new_v4()));
        let state = AppState::new(dir.join("config"), dir.join("data")).unwrap();
        let mut settings = state.store.settings().unwrap();
        settings.team = Some(TeamAccount {
            server: f.url.clone(),
            email: "ana@acme.test".into(),
            name: String::new(),
            role: "member".into(),
        });
        state.store.save_settings(&settings).unwrap();
        crate::store::secrets::set_team_token("tok-1").unwrap();
        state
            .store
            .save_note(Note {
                connection_id: "team:c_1".into(),
                ..Note::from("a note saved on this computer")
            })
            .unwrap();
        let notes = state
            .prompt_notes(&Target::default_of("team:c_1"), &["public.orders".into()])
            .await;
        let texts: Vec<&str> = notes.iter().map(|n| n.text.as_str()).collect();
        if team {
            assert_eq!(texts, ["orders.status 3 means refunded"]);
        } else {
            assert_eq!(texts, ["a note saved on this computer"]);
        }
        let _ = std::fs::remove_dir_all(dir);
    }
}

/// Against a real SayQL server 0.5.0: `SAYQL_TEST_TEAM_URL=http://127.0.0.1:8095
/// SAYQL_TEST_TEAM_CODE=… SAYQL_TEST_TEAM_ADMIN_CODE=… cargo test --lib
/// team::knowledge::tests::real_server -- --ignored`. The server needs a SQLite connection named
/// Shop over `orders(id, status)`, added with the notes "orders.status 3 means refunded", and
/// unused invites for a member and an admin.
#[tokio::test]
#[ignore]
async fn real_server() {
    use crate::team::{clean_server, redeem};
    let (Ok(url), Ok(code), Ok(admin_code)) = (
        std::env::var("SAYQL_TEST_TEAM_URL"),
        std::env::var("SAYQL_TEST_TEAM_CODE"),
        std::env::var("SAYQL_TEST_TEAM_ADMIN_CODE"),
    ) else {
        return;
    };
    let url = clean_server(&url).unwrap();
    let http = reqwest::Client::new();
    let (token, _) = redeem(&http, &url, &code, "test").await.unwrap();
    let ana = TeamClient::new(http.clone(), url.clone(), token, None);
    let shop = ana
        .connections()
        .await
        .unwrap()
        .into_iter()
        .find(|c| c.name == "Shop")
        .expect("a connection named Shop");
    let local = shop.local_id();

    // The feature comes from /me; the connection's notes became shared notes.
    let cache = Cache::default();
    let shared = cache.notes(&ana, &local).await.expect("shared notes");
    assert!(!shared.can_edit, "a member doesn't edit");
    let refunded = shared
        .notes
        .iter()
        .find(|n| n.text == "orders.status 3 means refunded")
        .expect("the connection's notes are shared notes");
    assert_eq!(refunded.connection_id, local);
    assert!(refunded.id.starts_with("kn_"), "{}", refunded.id);
    assert!(refunded.suspect.is_none());

    // A person's suggestion waits for the owners; once accepted, everyone's prompts have it.
    let text = format!(
        "orders.status 4 means disputed ({})",
        uuid::Uuid::new_v4().simple()
    );
    ana.suggest_knowledge(
        &shop.id,
        &Suggestion {
            text: text.clone(),
            source: Some("Investigate".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    cache.forget(&local);
    let (admin_token, _) = redeem(&http, &url, &admin_code, "test").await.unwrap();
    let list: Value = http
        .get(format!("{url}/api/v1/knowledge/connections/{}", shop.id))
        .bearer_auth(&admin_token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let waiting = list["suggestions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["text"] == text.as_str())
        .unwrap_or_else(|| panic!("the suggestion waits: {list}"));
    assert!(
        cache
            .notes(&ana, &local)
            .await
            .unwrap()
            .notes
            .iter()
            .all(|n| n.text != text),
        "not before it's approved"
    );
    let accepted = http
        .post(format!(
            "{url}/api/v1/knowledge/suggestions/{}/accept",
            waiting["id"].as_str().unwrap()
        ))
        .bearer_auth(&admin_token)
        .send()
        .await
        .unwrap();
    assert!(accepted.status().is_success(), "{}", accepted.status());
    cache.forget(&local);
    let after = cache.notes(&ana, &local).await.unwrap();
    assert!(after.notes.iter().any(|n| n.text == text), "{after:?}");
    let prompt = render_notes(&about(&after.notes, &["orders".into()]), false);
    assert!(prompt.contains(&text) && prompt.contains("orders.status 3 means refunded"));
}
