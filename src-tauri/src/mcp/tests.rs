use super::export::{EXPORT_OFF, EXPORT_ROWS, new_export_file, to_csv};
use super::shape::{failed, page, shape, shown_error, withhold};
use super::*;
use tokio_util::sync::CancellationToken;

/// The audit run ("Run audit") allows or denies every tool this server offers by name, so a
/// new tool can't slip into the run unlisted.
#[test]
fn the_audit_allows_or_denies_every_tool() {
    use crate::audit::runner::{CLAUDE_DENIED, SAYQL_TOOLS};
    let offered: Vec<String> = SayqlServer::tool_router()
        .list_all()
        .into_iter()
        .map(|t| format!("mcp__sayql__{}", t.name))
        .collect();
    assert!(offered.len() >= 10, "{offered:?}");
    let allowed: Vec<String> = SAYQL_TOOLS
        .iter()
        .map(|t| format!("mcp__sayql__{t}"))
        .collect();
    for tool in &offered {
        let a = allowed.contains(tool);
        let d = CLAUDE_DENIED.contains(&tool.as_str());
        assert!(a ^ d, "{tool}: allowed {a}, denied {d}");
    }
    for listed in allowed.iter().map(String::as_str).chain(
        CLAUDE_DENIED
            .iter()
            .copied()
            .filter(|t| t.starts_with("mcp__")),
    ) {
        assert!(offered.iter().any(|t| t == listed), "{listed} isn't a tool");
    }
}

fn add_params(args: Value) -> Result<AddConnectionParams, serde_json::Error> {
    serde_json::from_value(args)
}

#[test]
fn add_connection_refuses_passwords() {
    let p = add_params(json!({ "type": "postgres", "host": "db", "password": "hunter2" })).unwrap();
    assert_eq!(connection_draft(p).unwrap_err(), handoff::secret_refused());
    let p = add_params(json!({ "type": "mysql", "apiKey": "x" })).unwrap();
    assert_eq!(connection_draft(p).unwrap_err(), handoff::secret_refused());
    let p = add_params(json!({ "type": "mysql", "colour": "red" })).unwrap();
    assert!(connection_draft(p).unwrap_err().contains("colour"));
}

#[test]
fn add_connection_validates_fields() {
    let p = add_params(json!({ "type": "sqlserver", "host": "sql.local", "port": 1433, "ssl": true, "trust_server_certificate": true })).unwrap();
    let d = connection_draft(p).unwrap();
    assert_eq!(
        (d.kind, d.port, d.ssl, d.trust_cert),
        (Kind::Mssql, Some(1433), Some(true), Some(true))
    );
    assert!(
        connection_draft(add_params(json!({ "type": "postgres", "port": 0 })).unwrap()).is_err()
    );
    assert!(connection_draft(add_params(json!({ "type": "sqlite" })).unwrap()).is_err());
    assert!(
        add_params(json!({ "host": "db" })).is_err(),
        "type is required"
    );
    assert!(add_params(json!({ "type": "postgres", "port": "5432" })).is_err());
}

#[test]
fn withholds_never_send_columns() {
    let mut never = Masker::new(["patients.notes"]);
    never.learn_aliases("SELECT * FROM patients");
    let mut rows = vec![
        vec![json!(1), json!("private"), json!("x")],
        vec![json!(2), Value::Null, json!("y")],
    ];
    withhold(
        &never,
        &["id".into(), "NOTES".into(), "name".into()],
        &mut rows,
    );
    assert_eq!(
        rows[0],
        vec![json!(1), json!("(withheld: never sent to AI)"), json!("x")]
    );
    assert_eq!(rows[1][1], Value::Null);
    // `patients.notes` is about patients: a `notes` column of another table is sent.
    never.learn_aliases("SELECT * FROM visits");
    let mut rows = vec![vec![json!(1), json!("follow up"), json!("x")]];
    withhold(
        &never,
        &["id".into(), "notes".into(), "name".into()],
        &mut rows,
    );
    assert_eq!(rows[0][1], json!("follow up"));
    // A position from the Postgres plan is withheld whatever its name.
    never.learn_aliases("SELECT t.c6 FROM (SELECT * FROM patients) t(c1, c2, c3, c4, c5, c6)");
    never.withhold_positions(&[0]);
    let mut rows = vec![vec![json!("123-45-6789")]];
    withhold(&never, &["c6".into()], &mut rows);
    assert_eq!(rows[0][0], json!("(withheld: never sent to AI)"));
}

fn result(columns: &[&str], rows: Vec<Vec<Value>>) -> QueryResult {
    QueryResult {
        columns: columns.iter().map(|c| c.to_string()).collect(),
        rows,
        rows_affected: None,
        truncated: false,
        elapsed_ms: 3,
        withheld: vec![],
    }
}

#[test]
fn results_follow_what_ai_apps_may_see() {
    let rows = || {
        result(
            &["id", "email", "notes", "total"],
            vec![
                vec![
                    json!(1),
                    json!("ana@example.com"),
                    json!("allergic to penicillin"),
                    json!(40),
                ],
                vec![json!(2), json!("raj@example.com"), json!("vip"), json!(60)],
            ],
        )
    };
    let sql = "SELECT id, email, notes, total FROM customers";
    let never = || Masker::new(["notes".to_string()]);
    let text = |v: &Value| v.to_string();

    // Off: the row count and column names, no values at all.
    let off = shape(AppResults::Off, &mut never(), sql, &[], rows(), 0, 100);
    assert_eq!(
        (off["row_count"].clone(), off["columns"].clone()),
        (json!(2), json!(["id", "email", "notes", "total"]))
    );
    assert!(
        off.get("rows").is_none() && !text(&off).contains("ana@") && !text(&off).contains("vip")
    );

    // Counts: per-column summaries; no row values or labels.
    let counts = shape(AppResults::Counts, &mut never(), sql, &[], rows(), 0, 100);
    assert_eq!(counts["summary"]["rowCount"], json!(2));
    assert_eq!(counts["summary"]["columns"][3]["max"], json!(60));
    for value in ["ana@", "raj@", "penicillin", "vip"] {
        assert!(!text(&counts).contains(value), "{value} in {counts}");
    }

    // Masked: rows, with personal data and never-send columns as tokens.
    let masked = shape(AppResults::Masked, &mut never(), sql, &[], rows(), 0, 100);
    assert_eq!(masked["rows"][0][0], json!(1));
    assert_eq!(masked["rows"][1][3], json!(60));
    for value in ["ana@", "raj@", "penicillin", "vip"] {
        assert!(!text(&masked).contains(value), "{value} in {masked}");
    }
    assert!(masked["note"].as_str().unwrap().contains("tokens"));

    // All: rows as they are, never-send columns still withheld, paged.
    let all = shape(AppResults::All, &mut never(), sql, &[], rows(), 1, 100);
    assert_eq!(
        all["rows"],
        json!([[2, "raj@example.com", "(withheld: never sent to AI)", 60]])
    );
    assert_eq!(
        (all["offset"].clone(), all["next_offset"].clone()),
        (json!(1), Value::Null)
    );
}

#[test]
fn errors_show_only_their_safe_form_below_all_or_near_never_send_columns() {
    let e = || QueryFailure {
        full: "unrecognized configuration parameter \"chandra@example.in\"".into(),
        safe: "Something the query names does not exist.".into(),
    };
    let open = Guard::default();
    let protected = Guard {
        protected: true,
        withheld: vec![],
    };
    for level in [AppResults::Off, AppResults::Counts, AppResults::Masked] {
        assert!(
            !shown_error(level, &open, e()).contains("chandra"),
            "{level:?}"
        );
    }
    assert!(shown_error(AppResults::All, &open, e()).contains("chandra"));
    assert!(!shown_error(AppResults::All, &protected, e()).contains("chandra"));
    // The log keeps the safe form, at every level.
    let mut log = Activity::new("x", "run_query");
    let shown = failed(&mut log, AppResults::All, &open, e());
    assert!(shown.contains("chandra") && !log.detail.unwrap().contains("chandra"));
    assert_eq!(log.status, "error");
}

/// H1 and M1 of the governance test (2026-10-28): renamed personal columns get no min/max
/// at Counts, and neither do computed numbers (which can carry text).
#[test]
fn counts_give_min_and_max_only_for_plain_columns_and_aggregates() {
    let r = |cols: &[&str], v: Vec<Value>| result(cols, vec![v.clone(), v]);
    let summary = |sql: &str, res: QueryResult| {
        shape(
            AppResults::Counts,
            &mut Masker::new(Vec::<String>::new()),
            sql,
            &[],
            res,
            0,
            100,
        )
    };
    for sql in [
        "SELECT date_of_birth AS d FROM customers",
        "SELECT date_of_birth + 0 AS d FROM customers",
        "SELECT extract(year FROM date_of_birth) AS d FROM customers",
        "SELECT ('x'||encode(substr(email,1,7)::bytea,'hex'))::bit(56)::bigint AS d FROM customers WHERE id=1",
        "SELECT ascii(substr(email, 1, 1)) AS d FROM customers",
        "SELECT d FROM (SELECT ascii(substr(email, 1, 1)) AS d FROM customers) t",
        "SELECT sum(ascii(substr(email, 1, 1))) AS d FROM customers",
        "SELECT 1 AS d UNION ALL SELECT ascii(email) FROM customers",
    ] {
        let s = summary(sql, r(&["d"], vec![json!(97)]));
        assert!(
            s["summary"]["columns"][0].get("max").is_none(),
            "{sql}: {s}"
        );
    }
    for sql in [
        "SELECT count(*) AS d FROM customers WHERE email LIKE 'a%'",
        "SELECT max(total) AS d FROM orders",
        "SELECT o.total AS d FROM orders o",
        "SELECT round(avg(total), 2) AS d FROM orders",
        "SELECT count(DISTINCT email) d FROM customers",
    ] {
        let s = summary(sql, r(&["d"], vec![json!(97)]));
        assert_eq!(s["summary"]["columns"][0]["max"], json!(97), "{sql}: {s}");
    }
    let s = summary(
        "SELECT * FROM orders",
        r(&["id", "total"], vec![json!(1), json!(5)]),
    );
    assert_eq!(s["summary"]["columns"][1]["max"], json!(5), "{s}");
}

/// H2 of the governance test (2026-10-28): at Masked, names that reach the result without
/// their column's name are masked too.
#[test]
fn masked_rows_hide_names_that_arrive_under_other_names() {
    for sql in [
        "SELECT t.a FROM (SELECT name FROM customers) t(a)",
        "SELECT a FROM customers c, LATERAL (VALUES (c.name)) v(a)",
        "SELECT (c).name AS q FROM customers c",
        "SELECT x FROM unnest(ARRAY(SELECT name FROM customers)) x",
        "SELECT to_jsonb(c)->>'name' AS v FROM customers c",
        "SELECT 'x' AS a UNION ALL SELECT name FROM customers",
        "SELECT 1 AS id, 'x' AS label UNION ALL SELECT id, name FROM customers",
        "WITH x(a) AS (SELECT name FROM customers) SELECT a FROM x",
        "SELECT U&\"n\\0061me\" AS v FROM customers",
    ] {
        let res = result(
            &["v"],
            vec![vec![json!("Alice Smith")], vec![json!("Bob Jones")]],
        );
        let out = shape(
            AppResults::Masked,
            &mut Masker::new(Vec::<String>::new()),
            sql,
            &[],
            res,
            0,
            100,
        );
        assert!(!out.to_string().contains("Alice"), "{sql}: {out}");
    }
    // Plain queries keep ordinary values.
    let res = result(&["status"], vec![vec![json!("paid")]]);
    let out = shape(
        AppResults::Masked,
        &mut Masker::new(Vec::<String>::new()),
        "SELECT o.status FROM orders o JOIN customers c ON c.id = o.customer_id",
        &[],
        res,
        0,
        100,
    );
    assert_eq!(out["rows"][0][0], json!("paid"), "{out}");
}

#[test]
fn scripts_show_as_much_sql_as_the_level_allows() {
    assert_eq!(
        mask_literals(
            "SELECT * FROM c WHERE email = 'alice@x.com' AND n = 'it''s'",
            None
        ),
        "SELECT * FROM c WHERE email = '…' AND n = '…'"
    );
    // The 2026-10-29 audit: MySQL "…" strings, backslash escapes and comments leaked.
    let masked = mask_literals(
        "SELECT * FROM c WHERE email = \"bob@x.com\" AND note = 'a\\' carol' -- eve@x.com",
        Some(crate::db::Kind::Mysql),
    );
    for value in ["bob", "carol", "eve"] {
        assert!(!masked.contains(value), "{value} in {masked}");
    }
}

#[test]
fn describe_marks_never_send_columns() {
    use crate::schema::{Column, Table};
    let col = |n: &str| Column {
        name: n.into(),
        data_type: "text".into(),
        nullable: true,
        primary_key: false,
        ..Default::default()
    };
    let table = |n: &str| Table {
        schema: None,
        name: n.into(),
        kind: "table".into(),
        columns: vec![col("email"), col("ssn")],
        foreign_keys: vec![],
        row_estimate: None,
        ..Default::default()
    };
    let mut s = Schema {
        dialect: "PostgreSQL".into(),
        tables: vec![table("customers"), table("staff")],
    };
    mark_never_send(&mut s, &never_send::entries(&["customers.email", "ssn"]));
    let types: Vec<Vec<&str>> = s
        .tables
        .iter()
        .map(|t| t.columns.iter().map(|c| c.data_type.as_str()).collect())
        .collect();
    assert_eq!(
        types,
        [
            ["text [never sent to AI]", "text [never sent to AI]"],
            ["text", "text [never sent to AI]"]
        ]
    );
}

#[test]
fn unreadable_settings_refuse_everything_and_stay_put() {
    let dir = std::env::temp_dir().join(format!("sayql-mcp-corrupt-{}", uuid::Uuid::new_v4()));
    let mut state = AppState::new(dir.clone(), dir.clone()).unwrap();
    state.store = state.store.without_repair();
    let bad = r#"{"pg-off": {"apps": "off", "neverSend": ["ssn"]}"#;
    std::fs::write(dir.join("agent.json"), bad).unwrap();
    let server = SayqlServer::new(state);
    assert_eq!(server.access("pg-off").unwrap_err(), SETTINGS_UNREADABLE);
    assert!(server.state.store.agent_access_readable().is_err());
    assert_eq!(
        std::fs::read_to_string(dir.join("agent.json")).unwrap(),
        bad
    );
    let others: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().contains("corrupt"))
        .collect();
    assert!(others.is_empty(), "nothing moved aside");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_gate_lets_in_only_what_the_user_allows() {
    let dir = std::env::temp_dir().join(format!("sayql-mcp-gate-{}", uuid::Uuid::new_v4()));
    let mut state = AppState::new(dir.clone(), dir.clone()).unwrap();
    state.store = state.store.without_repair();
    let server = SayqlServer::new(state);
    let store = &server.state.store;
    let gate = |app: &str| server.gate_app(app.to_string(), "run_query");

    // Off (the default): anything not blocked.
    assert_eq!(gate("Zed").unwrap(), "Zed");
    store
        .log_app_activity(Activity::new("Zed", "run_query"))
        .unwrap();
    store.set_app_blocked("Cursor", true).unwrap();
    assert!(gate("Cursor").unwrap_err().contains("blocked Cursor"));

    // On: Zed was seen, so it's approved; a new app waits.
    store.set_only_approved_apps(true).unwrap();
    assert_eq!(gate("Zed").unwrap(), "Zed");
    assert_eq!(
        gate("Windsurf").unwrap_err(),
        "SayQL only lets in AI apps the user has approved. Ask them to approve Windsurf in SayQL under Settings → AI apps → Access."
    );
    let last = &store.app_activity(1).unwrap()[0];
    assert_eq!(
        (
            last.app.as_str(),
            last.status.as_str(),
            last.detail.as_deref()
        ),
        ("Windsurf", "refused", Some("not approved"))
    );
    let list = store.app_summaries().unwrap();
    let w = list.apps.iter().find(|a| a.name == "Windsurf").unwrap();
    assert!(list.only_approved && !w.approved && !w.blocked);
    store.approve_app("Windsurf").unwrap();
    assert_eq!(gate("Windsurf").unwrap(), "Windsurf");

    // Blocked wins over approved.
    store.approve_app("Cursor").unwrap();
    assert!(gate("Cursor").unwrap_err().contains("blocked Cursor"));

    // A corrupt apps file still refuses everything, approved or not, and stays put.
    std::fs::write(dir.join("mcp-apps.json"), r#"{"only_approved": tr"#).unwrap();
    assert_eq!(gate("Zed").unwrap_err(), SETTINGS_UNREADABLE);
    assert_eq!(
        std::fs::read_to_string(dir.join("mcp-apps.json")).unwrap(),
        r#"{"only_approved": tr"#
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn ai_apps_see_counts_until_the_user_chooses() {
    assert_eq!(AgentAccess::default().apps, AppResults::Counts);
    let saved: AgentAccess =
        serde_json::from_value(json!({ "enabled": true, "results": "rows" })).unwrap();
    assert_eq!(
        saved.apps,
        AppResults::Counts,
        "settings saved before this setting existed"
    );
    assert_eq!(
        serde_json::to_value(AppResults::Masked).unwrap(),
        json!("masked")
    );
}

#[test]
fn pages_through_rows() {
    let rows = |n: usize| (0..n).map(|i| vec![json!(i)]).collect::<Vec<_>>();
    let p = page(rows(250), false, 0, 100);
    assert_eq!(
        (p.rows.len(), p.next_offset, p.more, p.past_cap),
        (100, Some(100), true, false)
    );
    let p = page(rows(250), false, 200, 100);
    assert_eq!(
        (p.rows[0][0].clone(), p.rows.len(), p.next_offset, p.more),
        (json!(200), 50, None, false)
    );
    let p = page(rows(250), false, 900, 100);
    assert_eq!((p.rows.len(), p.next_offset, p.more), (0, None, false));
    let p = page(rows(5000), true, 4950, 100);
    assert_eq!(
        (p.rows.len(), p.next_offset, p.more, p.past_cap),
        (50, None, true, true)
    );
}

#[test]
fn writes_csv_like_the_app() {
    let csv = to_csv(
        &["id".into(), "note".into()],
        &[
            vec![json!(1), json!("a, \"b\"")],
            vec![json!(2), Value::Null],
        ],
    );
    assert_eq!(csv, "id,note\n1,\"a, \"\"b\"\"\"\n2,\n");
}

#[test]
fn export_files_never_overwrite() {
    let dir = std::env::temp_dir().join(format!("sayql-export-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let (a, _) = new_export_file(&dir, Some("../../etc/orders.csv")).unwrap();
    let (b, _) = new_export_file(&dir, Some("....etcorders")).unwrap();
    let (c, _) = new_export_file(&dir, None).unwrap();
    assert_eq!(a, dir.join("etcorders.csv"));
    assert_eq!(b, dir.join("etcorders (2).csv"));
    assert_eq!(c, dir.join("sayql-export.csv"));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn labels_known_clients() {
    let info = |name: &str| Implementation::new(name, "1");
    assert_eq!(client_label(&info("claude-ai")).as_deref(), Some("Claude"));
    assert_eq!(
        client_label(&info("claude-code")).as_deref(),
        Some("Claude Code")
    );
    assert_eq!(
        client_label(&info("cursor-vscode")).as_deref(),
        Some("Cursor")
    );
    assert_eq!(client_label(&info("Zed")).as_deref(), Some("Zed"));
    assert_eq!(
        client_label(&info("opencode")).as_deref(),
        Some(crate::audit::opencode::APP)
    );
    // The title an app sends is not used, and an unknown name is marked unverified.
    let mut spoof = info("x-agent");
    spoof.title = Some("SayQL Support".into());
    assert_eq!(client_label(&spoof).as_deref(), Some("x-agent"));
    assert_eq!(
        handoff_label(&spoof).as_deref(),
        Some("x-agent (unverified)")
    );
    assert_eq!(
        handoff_label(&info("claude-code")).as_deref(),
        Some("Claude Code")
    );
}

#[test]
fn suggested_notes_wait_for_the_user_and_are_logged() {
    let dir = std::env::temp_dir().join(format!("sayql-mcp-suggest-{}", uuid::Uuid::new_v4()));
    let state = AppState::new(dir.clone(), dir.clone()).unwrap();
    let cfg = db::ConnConfig {
        id: "c1".into(),
        name: "Shop".into(),
        kind: Kind::Sqlite,
        host: String::new(),
        port: None,
        database: String::new(),
        user: String::new(),
        file: "/tmp/none.sqlite".into(),
        ssl: false,
        trust_cert: false,
        production: false,
    };
    let pg = db::ConnConfig {
        id: "pg".into(),
        kind: Kind::Postgres,
        database: "shop".into(),
        file: String::new(),
        ..cfg.clone()
    };
    state.store.save_connections(&[cfg, pg]).unwrap();
    let server = SayqlServer::new(state);
    // Without `database`, notes go under the connection's default database, where the app keeps them.
    let on_pg = SuggestNoteParams {
        connection_id: "pg".into(),
        database: None,
        text: "status 3 = cancelled".into(),
        evidence: String::new(),
        replaces: None,
        remove: None,
        check: None,
    };
    server.suggest("Cursor", on_pg).unwrap();
    let s = server.state.store.note_suggestions().unwrap().remove(0);
    assert_eq!(s.database.as_deref(), Some("shop"));
    server.state.store.dismiss_note_suggestion(&s.id).unwrap();
    server.state.store.clear_app_activity().unwrap();
    let p = |text: &str, evidence: &str| SuggestNoteParams {
        connection_id: "c1".into(),
        database: None,
        text: text.into(),
        evidence: evidence.into(),
        replaces: None,
        remove: None,
        check: None,
    };
    let ok = server
        .suggest(
            "Claude Code",
            p(
                "orders_legacy is unused since March; use orders_v2",
                "0 writes since 2026-01-01",
            ),
        )
        .unwrap();
    assert!(ok.contains("until the user accepts"), "{ok}");
    let store = &server.state.store;
    assert!(store.notes_text("c1", None).is_empty(), "not a note yet");
    let waiting = store.note_suggestions().unwrap();
    assert_eq!((waiting.len(), waiting[0].app.as_str()), (1, "Claude Code"));

    assert!(
        server
            .suggest(
                "Claude Code",
                p("orders_legacy is unused since March; use orders_v2", "")
            )
            .unwrap_err()
            .contains("already suggested")
    );
    assert!(
        server
            .suggest("Cursor", p("the owner is dana@example.com", ""))
            .is_err()
    );
    assert!(
        server
            .suggest(
                "Cursor",
                SuggestNoteParams {
                    connection_id: "nope".into(),
                    ..p("x", "")
                }
            )
            .unwrap_err()
            .contains("list_connections")
    );

    let log = store.app_activity(10).unwrap();
    assert_eq!(log.len(), 4, "an unknown connection is logged too");
    assert_eq!(
        (
            log[3].tool.as_str(),
            log[3].status.as_str(),
            log[3].connection.as_str()
        ),
        ("suggest_note", "ok", "Shop")
    );
    assert_eq!(
        log[3].detail.as_deref(),
        Some("suggested a note"),
        "the note text stays out of the log"
    );
    assert_eq!(log[0].status, "refused");

    store.accept_note_suggestion(&waiting[0].id).unwrap();
    assert!(
        store
            .notes_text("c1", None)
            .contains("orders_legacy is unused"),
        "accepted notes reach describe_schema"
    );
    let _ = std::fs::remove_dir_all(dir);
}

fn sqlite_conn(id: &str, name: &str) -> db::ConnConfig {
    db::ConnConfig {
        id: id.into(),
        name: name.into(),
        kind: Kind::Sqlite,
        host: String::new(),
        port: None,
        database: String::new(),
        user: String::new(),
        file: "/tmp/none.sqlite".into(),
        ssl: false,
        trust_cert: false,
        production: false,
    }
}

#[test]
fn the_audit_prompt_names_the_connection_and_is_recorded() {
    let dir = std::env::temp_dir().join(format!("sayql-mcp-audit-{}", uuid::Uuid::new_v4()));
    let state = AppState::new(dir.clone(), dir.clone()).unwrap();
    let pg = db::ConnConfig {
        kind: Kind::Postgres,
        database: "shop".into(),
        file: String::new(),
        ..sqlite_conn("pg", "Warehouse")
    };
    state
        .store
        .save_connections(&[sqlite_conn("c1", "Shop"), pg])
        .unwrap();
    let server = SayqlServer::new(state);
    let store = &server.state.store;
    let args = |v: Value| v.as_object().cloned();

    // By name, any case; a SQLite file has no database, whatever the app passes.
    let text = server
        .audit_prompt(
            "Claude Code",
            args(json!({ "connection": "shop", "database": "x" })).as_ref(),
        )
        .unwrap();
    assert!(
        text.contains("connection_id \"c1\"") && !text.contains("\"x\""),
        "{text}"
    );
    // By id, on the server's default database.
    let text = server
        .audit_prompt("Cursor", args(json!({ "connection": "pg" })).as_ref())
        .unwrap();
    assert!(text.contains("database \"shop\""), "{text}");
    let mut audits = store.audits().unwrap();
    audits.sort_by(|a, b| a.connection_id.cmp(&b.connection_id));
    assert_eq!(
        audits
            .iter()
            .map(|a| (
                a.connection_id.as_str(),
                a.database.as_deref(),
                a.app.as_str()
            ))
            .collect::<Vec<_>>(),
        [("c1", None, "Claude Code"), ("pg", Some("shop"), "Cursor")]
    );
    // No connection named with two saved: the app picks one; nothing recorded.
    let open = server.audit_prompt("Cursor", None).unwrap();
    assert!(open.contains("call list_connections"), "{open}");
    assert_eq!(store.audits().unwrap().len(), 2);
    let e = server
        .audit_prompt("Cursor", args(json!({ "connection": "crm" })).as_ref())
        .unwrap_err();
    assert!(e.contains("\"Warehouse\""), "{e}");

    let log = store.app_activity(10).unwrap();
    assert_eq!(
        log.iter()
            .map(|l| (l.tool.as_str(), l.status.as_str(), l.connection.as_str()))
            .collect::<Vec<_>>(),
        [
            ("audit", "error", ""),
            ("audit", "ok", ""),
            ("audit", "ok", "Warehouse"),
            ("audit", "ok", "Shop"),
        ]
    );
    // The gate applies: a blocked app gets nothing.
    store.set_app_blocked("Cursor", true).unwrap();
    assert!(
        server
            .gate_app("Cursor".into(), audit::PROMPT)
            .unwrap_err()
            .contains("blocked Cursor")
    );
    assert_eq!(audit_prompt_info().name, "audit");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn suggest_note_can_replace_or_remove_a_saved_note() {
    let dir = std::env::temp_dir().join(format!("sayql-mcp-replace-{}", uuid::Uuid::new_v4()));
    let state = AppState::new(dir.clone(), dir.clone()).unwrap();
    state
        .store
        .save_connections(&[sqlite_conn("c1", "Shop")])
        .unwrap();
    let server = SayqlServer::new(state);
    let store = &server.state.store;
    let note = store
        .save_note(crate::store::agent::Note {
            id: "1A2B3C4D-0000-4000-8000-000000000000".into(),
            connection_id: "c1".into(),
            database: None,
            text: "orders_v1 is the live orders table".into(),
            updated_at: 0,
            ..Default::default()
        })
        .unwrap();
    assert!(
        store
            .notes_text_with_refs("c1", None)
            .contains("[n_1a2b3c4d] orders_v1 is the live orders table")
    );
    let p = |text: &str, replaces: Option<&str>, remove: Option<bool>| SuggestNoteParams {
        connection_id: "c1".into(),
        database: None,
        text: text.into(),
        evidence: "api/orders.ts reads orders_v2 only".into(),
        replaces: replaces.map(str::to_string),
        remove,
        check: None,
    };
    let ok = server
        .suggest(
            "Claude Code",
            p(
                "orders_v2 is the live orders table",
                Some("n_1a2b3c4d"),
                None,
            ),
        )
        .unwrap();
    assert!(ok.contains("Suggested replacing the note"), "{ok}");
    assert!(
        server
            .suggest("Claude Code", p("gone", None, Some(true)))
            .unwrap_err()
            .contains("set `replaces`")
    );
    assert!(
        server
            .suggest("Claude Code", p("gone", Some("n_ffffffff"), Some(true)))
            .unwrap_err()
            .contains("No saved note")
    );
    server
        .suggest(
            "Claude Code",
            p("orders_v1 was dropped", Some("[n_1a2b3c4d]"), Some(true)),
        )
        .unwrap();
    let waiting = store.note_suggestions().unwrap();
    assert_eq!(
        waiting
            .iter()
            .map(|s| (s.replaces.as_deref(), s.remove))
            .collect::<Vec<_>>(),
        [
            (Some(note.id.as_str()), false),
            (Some(note.id.as_str()), true)
        ]
    );
    let details: Vec<_> = store
        .app_activity(10)
        .unwrap()
        .into_iter()
        .filter_map(|l| l.detail)
        .collect();
    assert!(details.contains(&"suggested replacing a note".to_string()));
    assert!(details.contains(&"suggested removing a note".to_string()));
    let _ = std::fs::remove_dir_all(dir);
}

/// 1000 rows, 0..999, in SQL every supported server runs.
const THOUSAND: &str = "SELECT a.d * 100 + b.d * 10 + c.d AS v FROM \
        (SELECT 0 AS d UNION ALL SELECT 1 UNION ALL SELECT 2 UNION ALL SELECT 3 UNION ALL SELECT 4 UNION ALL SELECT 5 UNION ALL SELECT 6 UNION ALL SELECT 7 UNION ALL SELECT 8 UNION ALL SELECT 9) a CROSS JOIN \
        (SELECT 0 AS d UNION ALL SELECT 1 UNION ALL SELECT 2 UNION ALL SELECT 3 UNION ALL SELECT 4 UNION ALL SELECT 5 UNION ALL SELECT 6 UNION ALL SELECT 7 UNION ALL SELECT 8 UNION ALL SELECT 9) b CROSS JOIN \
        (SELECT 0 AS d UNION ALL SELECT 1 UNION ALL SELECT 2 UNION ALL SELECT 3 UNION ALL SELECT 4 UNION ALL SELECT 5 UNION ALL SELECT 6 UNION ALL SELECT 7 UNION ALL SELECT 8 UNION ALL SELECT 9) c \
        ORDER BY v";

/// The tools against the demo SQLite store, and against real servers when the
/// SAYQL_TEST_*_URL variables are set (see db/integration_tests.rs). The connection is saved
/// without a password and its pool opened directly, so the test never touches the keychain.
#[tokio::test]
async fn tools_work_against_real_databases() {
    let demo_dir = std::env::temp_dir().join(format!("sayql-mcp-demo-{}", uuid::Uuid::new_v4()));
    let demo = demo_dir.join("demo.sqlite");
    db::ensure_demo(&demo).await.unwrap();
    let sqlite = db::ConnConfig {
        id: "demo".into(),
        name: "Demo".into(),
        kind: Kind::Sqlite,
        host: String::new(),
        port: None,
        database: String::new(),
        user: String::new(),
        file: demo.to_string_lossy().into_owned(),
        ssl: false,
        trust_cert: false,
        production: false,
    };
    let servers = [
        ("SAYQL_TEST_PG_URL", Kind::Postgres),
        ("SAYQL_TEST_MYSQL_URL", Kind::Mysql),
        ("SAYQL_TEST_MSSQL_URL", Kind::Mssql),
    ];
    let targets = std::iter::once(Some((sqlite, None)))
        .chain(servers.map(|(var, kind)| db::env_config(var, kind)));
    for (cfg, password) in targets.flatten() {
        let kind = cfg.kind;
        let dir = std::env::temp_dir().join(format!("sayql-mcp-{}", uuid::Uuid::new_v4()));
        let state = AppState::new(dir.clone(), dir.clone()).unwrap();
        state
            .store
            .save_connections(std::slice::from_ref(&cfg))
            .unwrap();
        state.set_pool(
            Target::default_of(&cfg.id),
            db::connect(&cfg, password).await.unwrap(),
        );
        let server = SayqlServer::new(state);
        let json = |s: String| serde_json::from_str::<Value>(&s).unwrap();
        let conn = || ConnectionParams {
            connection_id: cfg.id.clone(),
        };

        let list = json(server.connections().unwrap());
        assert_eq!(list[0]["dialect"], json!(kind.dialect()), "{kind:?}");
        assert_eq!(
            list[0]["ai_may_see"],
            json!("counts"),
            "{kind:?}: counts until the user chooses"
        );
        let dbs = server.databases(conn()).await.unwrap();
        if kind == Kind::Sqlite {
            assert!(dbs.contains("single database"), "{dbs}");
        } else {
            assert!(json(dbs.clone())["databases"].is_array(), "{kind:?}: {dbs}");
        }
        let schema = server
            .describe(SchemaParams {
                connection_id: cfg.id.clone(),
                database: None,
                search: None,
            })
            .await
            .unwrap();
        assert!(
            schema.starts_with(&format!("Dialect: {}.", kind.dialect())),
            "{schema}"
        );
        // Usage from the database's statistics: nothing for SQLite; for servers, the period or why not.
        if kind == Kind::Sqlite {
            assert!(
                !schema.contains("usage") && !schema.contains("Usage"),
                "{schema}"
            );
        } else {
            assert!(schema.contains("statistics"), "{kind:?}: {schema}");
        }
        if kind == Kind::Sqlite {
            // A table added while the server runs shows up in the next describe_schema.
            let pool = server
                .state
                .target_pool(&Target::default_of(&cfg.id))
                .await
                .unwrap();
            db::run_query(
                &pool,
                "CREATE TABLE added_later (id INTEGER PRIMARY KEY)",
                false,
            )
            .await
            .unwrap();
            let again = server
                .describe(SchemaParams {
                    connection_id: cfg.id.clone(),
                    database: None,
                    search: None,
                })
                .await
                .unwrap();
            assert!(
                !schema.contains("added_later") && again.contains("added_later"),
                "{again}"
            );
        }

        let run = |sql: &str, offset: Option<usize>| {
            let p = QueryParams {
                connection_id: cfg.id.clone(),
                database: None,
                sql: sql.into(),
                max_rows: Some(300),
                offset,
            };
            async {
                server
                    .query(p, &mut Activity::new("test", "run_query"))
                    .await
            }
        };
        // By default the AI app gets counts and summaries, not rows, and can't export.
        let counts = json(run(THOUSAND, None).await.unwrap());
        assert_eq!(
            counts["summary"]["rowCount"],
            json!(1000),
            "{kind:?}: {counts}"
        );
        assert!(counts.get("rows").is_none(), "{kind:?}: {counts}");
        let export = |sql: &str| {
            let p = ExportParams {
                connection_id: cfg.id.clone(),
                database: None,
                sql: sql.into(),
                file_name: None,
            };
            async {
                server
                    .export(p, &mut Activity::new("test", "export_csv"))
                    .await
            }
        };
        assert_eq!(export(THOUSAND).await.unwrap_err(), EXPORT_OFF, "{kind:?}");
        // Rows once the user allows them.
        let all = AgentAccess {
            apps: AppResults::All,
            ..Default::default()
        };
        server.state.store.save_agent_access(&cfg.id, all).unwrap();

        // Pages through the rows, and says where the next page starts.
        let first = json(run(THOUSAND, None).await.unwrap());
        assert_eq!(
            (first["row_count"].clone(), first["next_offset"].clone()),
            (json!(300), json!(300)),
            "{kind:?}"
        );
        assert_eq!(first["rows"][0], json!([0]));
        let last = json(run(THOUSAND, Some(900)).await.unwrap());
        assert_eq!(
            (last["row_count"].clone(), last["next_offset"].clone()),
            (json!(100), Value::Null),
            "{kind:?}"
        );
        assert_eq!(last["rows"][99], json!([999]));
        assert_eq!(last["truncated"], json!(false));
        // Anything that could change data is refused, however it is dressed up.
        for sql in [
            "DELETE FROM t",
            "SELECT 1; DROP TABLE t",
            "WITH x AS (SELECT 1 AS a) UPDATE t SET a = 1",
        ] {
            assert_eq!(
                run(sql, None).await.unwrap_err(),
                WRITE_REFUSED,
                "{kind:?}: {sql}"
            );
        }
        // What an AI app ran is in the activity log, under its name, with why it was refused.
        for sql in ["SELECT 1 AS n", "DELETE FROM t"] {
            let mut log = server.activity("Claude Code", "run_query", &cfg.id, sql);
            let p = QueryParams {
                connection_id: cfg.id.clone(),
                database: None,
                sql: sql.into(),
                max_rows: None,
                offset: None,
            };
            let result = server.query(p, &mut log).await;
            server.record(log, &result);
        }
        let log = server.state.store.app_activity(2).unwrap();
        assert_eq!(
            (log[0].status.as_str(), log[0].detail.as_deref()),
            ("refused", Some("could change data")),
            "{kind:?}"
        );
        assert_eq!(
            (log[1].app.as_str(), log[1].status.as_str(), log[1].rows),
            ("Claude Code", "ok", Some(1)),
            "{kind:?}"
        );
        assert_eq!(log[1].connection, cfg.name);

        // export_csv writes to Downloads; check what it writes without writing there.
        let pool = server
            .state
            .target_pool(&Target::default_of(&cfg.id))
            .await
            .unwrap();
        let r = db::run_query_with(
            &pool,
            THOUSAND,
            true,
            EXPORT_ROWS,
            &CancellationToken::new(),
        )
        .await
        .unwrap();
        let csv = to_csv(&r.columns, &r.rows);
        assert_eq!(csv.lines().count(), 1001, "{kind:?}");
        assert!(
            csv.starts_with("v\n0\n1\n") && csv.ends_with("999\n"),
            "{kind:?}"
        );
        let _ = std::fs::remove_dir_all(dir);
    }
    let _ = std::fs::remove_dir_all(demo_dir);
}

/// Every repro from the governance test (2026-10-28) against a real PostgreSQL, when
/// SAYQL_TEST_PG_URL is set: refused, or answered without the values.
#[tokio::test]
async fn governance_repros_on_postgres() {
    let Some((cfg, password)) = db::env_config("SAYQL_TEST_PG_URL", Kind::Postgres) else {
        return;
    };
    let dir = std::env::temp_dir().join(format!("sayql-mcp-gov-{}", uuid::Uuid::new_v4()));
    let state = AppState::new(dir.clone(), dir.clone()).unwrap();
    state
        .store
        .save_connections(std::slice::from_ref(&cfg))
        .unwrap();
    let pool = db::connect(&cfg, password).await.unwrap();
    for sql in [
        "DROP VIEW IF EXISTS gov_contacts",
        "DROP TABLE IF EXISTS gov_customers",
        "CREATE TABLE gov_customers (id int PRIMARY KEY, name text, email text, ssn text, salary int, \
             date_of_birth date, secret_code text)",
        "INSERT INTO gov_customers VALUES (1, 'Alice Smith', 'alice@example.com', '123-45-6789', 91000, '1985-03-14', 'zebra'), \
             (2, 'Bob Jones', 'bob@example.com', '987-65-4321', 72000, '1990-07-01', 'tiger'), \
             (3, 'Chandra Rao', 'chandra@example.in', '111-22-3333', 55000, '1979-12-25', 'koala')",
        "CREATE VIEW gov_contacts AS SELECT id, ssn AS ref, salary AS pay FROM gov_customers",
    ] {
        db::run_query(&pool, sql, false).await.unwrap();
    }
    state.set_pool(Target::default_of(&cfg.id), pool.clone());
    let server = SayqlServer::new(state);
    let set = |apps: AppResults| {
        let a = AgentAccess {
            apps,
            never_send: vec![
                "ssn".into(),
                "gov_customers.salary".into(),
                "secret_code".into(),
            ],
            ..Default::default()
        };
        server.state.store.save_agent_access(&cfg.id, a).unwrap();
    };
    let run = |sql: &str| {
        let p = QueryParams {
            connection_id: cfg.id.clone(),
            database: None,
            sql: sql.into(),
            max_rows: None,
            offset: None,
        };
        let server = &server;
        async move {
            let mut log = Activity::new("test", "run_query");
            let r = server.query(p, &mut log).await;
            let text = match &r {
                Ok(t) | Err(t) => t.clone(),
            };
            (r.is_ok(), text, serde_json::to_string(&log).unwrap())
        }
    };
    let never_values = [
        "123-45-6789",
        "987-65",
        "91000",
        "72000",
        "zebra",
        "tiger",
        "koala",
    ];

    // C2: never-send values in any form, at every level, and in export_csv.
    for level in [AppResults::All, AppResults::Masked, AppResults::Counts] {
        set(level);
        for sql in [
            "SELECT row_to_json(c) AS r FROM gov_customers c WHERE id=1",
            "SELECT c::text AS r FROM gov_customers c WHERE id=1",
            "SELECT json_agg(c) AS j FROM gov_customers c",
            "SELECT to_jsonb(c)->>('sec'||'ret_code') AS v FROM gov_customers c",
            r#"SELECT U&"\0073sn" AS v FROM gov_customers"#,
            "SELECT t.c4 FROM (SELECT * FROM gov_customers) AS t(c1,c2,c3,c4,c5,c6,c7)",
            "SELECT sal FROM gov_customers c, LATERAL (SELECT c.*) x(i,n,e,s,sal,dob,sc)",
            "SELECT key, value FROM gov_customers c, jsonb_each_text(to_jsonb(c)) WHERE c.id=1",
            "SELECT id FROM gov_customers c WHERE c::text LIKE '%123-45%'",
            "SELECT (to_jsonb(c)->>('sal'||'ary'))::int AS v FROM gov_customers c",
            "SELECT max(pay) AS v FROM gov_contacts",
            "SELECT id FROM gov_contacts WHERE ref LIKE '1%'",
            "WITH x AS (SELECT * FROM gov_contacts) SELECT * FROM x",
            "SELECT 'x' AS a UNION ALL SELECT ref FROM gov_contacts",
        ] {
            let (_, text, log) = run(sql).await;
            for v in never_values {
                assert!(
                    !text.contains(v) && !log.contains(v),
                    "{level:?} {sql}: {text}"
                );
            }
        }
        // A never-send column passed straight through a view is withheld, not shown.
        let (ok, text, _) = run("SELECT id, ref, pay FROM gov_contacts").await;
        assert!(ok, "{level:?}: {text}");
        for v in never_values {
            assert!(!text.contains(v), "{level:?}: {text}");
        }
        let (ok, text, _) = run("SELECT * FROM gov_customers").await;
        assert!(ok, "{level:?}: {text}");
        for v in never_values {
            assert!(!text.contains(v), "{level:?}: {text}");
        }
    }
    set(AppResults::All);
    let p = ExportParams {
        connection_id: cfg.id.clone(),
        database: None,
        sql: "SELECT row_to_json(c) AS r FROM gov_customers c".into(),
        file_name: None,
    };
    assert!(
        server
            .export(p, &mut Activity::new("test", "export_csv"))
            .await
            .is_err()
    );
    // L7: server files, whatever the login.
    assert!(!run("SELECT pg_read_file('/etc/hosts')").await.0);
    assert!(!run("SELECT * FROM pg_ls_dir('.') d").await.0);
    // Ordinary queries still run.
    for sql in [
        "SELECT count(*) FROM gov_customers",
        "SELECT c.id, c.name, c.email FROM gov_customers c WHERE c.id > 1 ORDER BY c.name",
        "SELECT count(*) FROM gov_customers WHERE id IN (SELECT id FROM gov_customers WHERE name LIKE 'A%')",
    ] {
        let (ok, text, _) = run(sql).await;
        assert!(ok, "{sql}: {text}");
    }

    // C1: errors don't carry values below all (nor at all near never-send columns), nor does the log.
    for level in [
        AppResults::Off,
        AppResults::Counts,
        AppResults::Masked,
        AppResults::All,
    ] {
        set(level);
        for sql in [
            "SELECT current_setting(email) FROM gov_customers WHERE id=3",
            "SELECT convert_from(name::bytea, name) FROM gov_customers",
            "SELECT xmlparse(document name) FROM gov_customers",
            "SELECT email::regclass FROM gov_customers",
            "SELECT name::regtype FROM gov_customers",
            "SELECT email::json FROM gov_customers",
            "SELECT email::int FROM gov_customers",
        ] {
            let (ok, text, log) = run(sql).await;
            assert!(!ok, "{sql}");
            for v in ["alice", "Alice", "Smith", "chandra", "Chandra"] {
                assert!(!log.contains(v), "{level:?} {sql}: log {log}");
                if level != AppResults::All {
                    assert!(!text.contains(v), "{level:?} {sql}: {text}");
                }
            }
        }
        let (_, text, _) = run("SELECT nme FROM gov_customers").await;
        assert!(
            text.contains("nme") && text.contains("does not exist"),
            "{text}"
        );
    }

    // H1, M1: no min/max for renamed personal columns or computed numbers at counts.
    set(AppResults::Counts);
    for sql in [
        "SELECT date_of_birth AS d FROM gov_customers",
        "SELECT date_of_birth + 0 AS d FROM gov_customers",
        "SELECT extract(year FROM date_of_birth) AS d FROM gov_customers",
        "SELECT ('x'||encode(substr(email,1,7)::bytea,'hex'))::bit(56)::bigint AS d FROM gov_customers WHERE id=1",
    ] {
        let (ok, text, _) = run(sql).await;
        assert!(ok, "{sql}: {text}");
        let v: Value = serde_json::from_str(&text).unwrap();
        assert!(
            v["summary"]["columns"][0].get("max").is_none(),
            "{sql}: {text}"
        );
    }
    let (_, text, _) = run("SELECT max(id) AS top FROM gov_customers").await;
    let v: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(v["summary"]["columns"][0]["max"], json!(3), "{text}");

    // H2: names at masked.
    set(AppResults::Masked);
    for sql in [
        "SELECT t.a FROM (SELECT name FROM gov_customers) t(a)",
        "SELECT a FROM gov_customers c, LATERAL (VALUES (c.name)) v(a)",
        "SELECT (c).name AS q FROM gov_customers c",
        "SELECT x FROM unnest(ARRAY(SELECT name FROM gov_customers)) x",
        "SELECT to_jsonb(c)->>'name' AS v FROM gov_customers c",
        "SELECT 'x' AS a UNION ALL SELECT name FROM gov_customers",
        "SELECT 1 AS id, 'x' AS label UNION ALL SELECT id, name FROM gov_customers",
    ] {
        let (_, text, _) = run(sql).await;
        for v in ["Alice", "Bob", "Chandra"] {
            assert!(!text.contains(v), "{sql}: {text}");
        }
    }

    for sql in ["DROP VIEW gov_contacts", "DROP TABLE gov_customers"] {
        db::run_query(&pool, sql, false).await.unwrap();
    }
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_server_uses_the_app_id_it_is_given_when_it_names_a_folder() {
    assert_eq!(app_id(None), "com.sayql.app");
    assert_eq!(
        app_id(Some("com.sayql.e2etest".into())),
        "com.sayql.e2etest"
    );
    for bad in [
        "",
        "..",
        ".hidden",
        "../com.sayql.app",
        "a/b",
        "a\\b",
        "com sayql",
    ] {
        assert_eq!(app_id(Some(bad.into())), "com.sayql.app", "{bad}");
    }
}

/// Two companies saved side by side: `acme` in Acme, in its group Portside, and `harbor`, with a
/// note, a script and a list of allowed databases, in Harbor. A server is limited to one of them.
fn two_companies(scope: Scope) -> SayqlServer {
    let dir = std::env::temp_dir().join(format!("sayql-mcp-scope-{}", uuid::Uuid::new_v4()));
    let state = AppState::new(dir.clone(), dir).unwrap();
    let mut server = sqlite_conn("server", "shared server");
    server.kind = Kind::Mssql;
    server.host = "sql.example.com".into();
    state
        .store
        .save_connections(&[
            sqlite_conn("old", "From before workspaces"),
            sqlite_conn("acme", "Acme billing"),
            sqlite_conn("harbor", "Harbor claims"),
            server,
        ])
        .unwrap();
    state
        .store
        .update_layout(|l| {
            l.create_workspace("Acme")?;
            l.create_workspace("Harbor")?;
            l.create_group("acme", "Portside")?;
            l.create_group("acme", "Meridian")?;
            l.move_connection("acme", "acme", Some("portside"))?;
            l.move_connection("harbor", "harbor", None)?;
            l.move_connection("server", "acme", Some("meridian"))?;
            l.set_allowed_databases("server", Some(vec!["acme_orders".into()]));
            Ok(())
        })
        .unwrap();
    state
        .store
        .update_scripts(|list| {
            list.push(crate::store::Script {
                id: "harbor-script".into(),
                name: "Claims by payer".into(),
                connection_id: "harbor".into(),
                database: None,
                question: String::new(),
                sql: "SELECT 1".into(),
                updated_at: 0,
            });
            list.push(crate::store::Script {
                id: "orphan".into(),
                name: "From a deleted connection".into(),
                connection_id: "deleted".into(),
                database: None,
                question: String::new(),
                sql: "SELECT 2".into(),
                updated_at: 0,
            });
            Ok(())
        })
        .unwrap();
    SayqlServer::new(state).with_scope(scope)
}

fn acme_scope(group: Option<&str>) -> Scope {
    Scope {
        workspace: "acme".into(),
        group: group.map(str::to_string),
    }
}

/// Everything a tool replies about a connection: the reply for one in another workspace must be
/// the reply for one that isn't saved, so an app can't learn it exists.
async fn replies_about(server: &SayqlServer, id: &str) -> Vec<String> {
    let show = |r: Result<String, String>| match r {
        Ok(text) => format!("ok: {text}"),
        Err(text) => format!("err: {text}"),
    };
    let connection_id = id.to_string();
    let mut log = Activity::new("Test", "run_query");
    let mut out = vec![
        show(
            server
                .databases(ConnectionParams {
                    connection_id: connection_id.clone(),
                })
                .await,
        ),
        show(
            server
                .describe(SchemaParams {
                    connection_id: connection_id.clone(),
                    database: None,
                    search: None,
                })
                .await,
        ),
        show(
            server
                .query(
                    QueryParams {
                        connection_id: connection_id.clone(),
                        database: None,
                        sql: "SELECT 1".into(),
                        max_rows: None,
                        offset: None,
                    },
                    &mut log,
                )
                .await,
        ),
        show(
            server
                .export(
                    ExportParams {
                        connection_id: connection_id.clone(),
                        database: None,
                        sql: "SELECT 1".into(),
                        file_name: None,
                    },
                    &mut log,
                )
                .await,
        ),
        show(server.suggest(
            "Test",
            SuggestNoteParams {
                connection_id: connection_id.clone(),
                database: None,
                text: "orders is unused".into(),
                evidence: "no reads in api/".into(),
                replaces: None,
                remove: None,
                check: None,
            },
        )),
        show(server.scripts(Some(id))),
        show(server.query_connection(id, None).map(|c| c.name)),
    ];
    let mut args = Map::new();
    args.insert("connection".into(), json!(id));
    out.push(show(server.audit_prompt("Test", Some(&args))));
    // Some replies quote the id they were given; compare them with it taken out.
    out.iter_mut()
        .for_each(|reply| *reply = reply.replace(id, "<id>"));
    out
}

#[tokio::test]
async fn a_connection_in_another_workspace_replies_exactly_like_one_that_does_not_exist() {
    for scope in [
        Scope::default(),
        acme_scope(None),
        acme_scope(Some("portside")),
    ] {
        let server = two_companies(scope.clone());
        assert_eq!(
            replies_about(&server, "harbor").await,
            replies_about(&server, "nope").await,
            "{scope:?}"
        );
    }
}

#[tokio::test]
async fn a_connection_in_another_group_replies_exactly_like_one_that_does_not_exist() {
    let server = two_companies(acme_scope(Some("portside")));
    assert_eq!(
        replies_about(&server, "server").await,
        replies_about(&server, "nope").await
    );
}

#[test]
fn an_app_set_up_without_a_workspace_sees_personal_only() {
    let list = two_companies(Scope::default()).connections().unwrap();
    assert!(list.contains("From before workspaces"), "{list}");
    assert!(!list.contains("Acme"), "{list}");
    assert!(!list.contains("Harbor"), "{list}");
}

#[test]
fn a_workspace_sees_its_own_connections_and_says_where_they_are() {
    let list = two_companies(acme_scope(None)).connections().unwrap();
    let list: Value = serde_json::from_str(&list).unwrap();
    let names: Vec<&str> = list
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["Acme billing", "shared server"]);
    assert_eq!(list[0]["workspace"], json!("Acme"));
    assert_eq!(list[0]["group"], json!("Portside"));
    // A list's first database stands in for a default that isn't on it.
    assert_eq!(list[1]["default_database"], json!("acme_orders"));
}

#[test]
fn a_group_sees_only_its_own_connections() {
    let list = two_companies(acme_scope(Some("portside")))
        .connections()
        .unwrap();
    assert!(list.contains("Acme billing"), "{list}");
    assert!(!list.contains("shared server"), "{list}");
}

#[test]
fn personal_listing_keeps_its_old_shape() {
    let list = two_companies(Scope::default()).connections().unwrap();
    let list: Value = serde_json::from_str(&list).unwrap();
    assert!(list[0].get("workspace").is_none());
    assert!(list[0].get("group").is_none());
}

#[test]
fn scripts_of_other_workspaces_and_deleted_connections_are_not_listed() {
    let server = two_companies(Scope::default());
    let list = server.scripts(None).unwrap();
    assert!(!list.contains("Claims by payer"), "{list}");
    assert!(!list.contains("From a deleted connection"), "{list}");
    let harbor = two_companies(Scope {
        workspace: "harbor".into(),
        group: None,
    });
    assert!(harbor.scripts(None).unwrap().contains("Claims by payer"));
}

#[tokio::test]
async fn a_database_outside_the_list_gets_the_same_reply_whether_or_not_it_exists() {
    let server = two_companies(acme_scope(None));
    let reply = |name: &str| {
        let server = &server;
        let name = name.to_string();
        async move {
            server
                .target_in_reach("server", Some(name.clone()))
                .await
                .unwrap_err()
                .replace(&name, "<name>")
        }
    };
    assert_eq!(reply("harbor_claims").await, reply("no_such_db").await);
    let opened = server
        .target_in_reach("server", Some("ACME_ORDERS".into()))
        .await
        .unwrap();
    assert_eq!(opened.database.as_deref(), Some("ACME_ORDERS"));
    // Unnamed, it opens the list's database, never the login's own default.
    let opened = server.target_in_reach("server", None).await.unwrap();
    assert_eq!(opened.database.as_deref(), Some("acme_orders"));
}

#[test]
fn a_database_outside_the_list_can_not_be_opened_or_noted() {
    let server = two_companies(acme_scope(None));
    assert!(
        server
            .query_connection("server", Some("harbor_claims"))
            .is_err()
    );
    assert!(
        server
            .query_connection("server", Some("acme_orders"))
            .is_ok()
    );
    let note = |database: &str| {
        server.suggest(
            "Test",
            SuggestNoteParams {
                connection_id: "server".into(),
                database: Some(database.into()),
                text: "orders is unused".into(),
                evidence: "no reads in api/".into(),
                replaces: None,
                remove: None,
                check: None,
            },
        )
    };
    assert!(note("harbor_claims").is_err());
    assert!(note("acme_orders").is_ok());
}

#[test]
fn an_unreadable_layout_refuses_rather_than_showing_everything() {
    let server = two_companies(acme_scope(None));
    let dir = server.state.store.dir().to_path_buf();
    std::fs::write(dir.join("layout.json"), "{oops").unwrap();
    let mut state = AppState::new(dir.clone(), dir).unwrap();
    state.store = state.store.without_repair();
    let server = SayqlServer::new(state).with_scope(acme_scope(None));
    assert_eq!(server.connections().unwrap_err(), SETTINGS_UNREADABLE);
    assert_eq!(server.scripts(None).unwrap_err(), SETTINGS_UNREADABLE);
    assert_eq!(server.limit_for("acme").unwrap_err(), SETTINGS_UNREADABLE);
}

#[test]
fn the_log_says_which_workspace_and_group_a_call_came_through() {
    let server = two_companies(acme_scope(Some("portside")));
    server.log_call("Test", "list_connections", "", "", &Ok(String::new()));
    let log = server.state.store.app_activity(1).unwrap();
    assert_eq!(log[0].workspace.as_deref(), Some("acme"));
    assert_eq!(log[0].group.as_deref(), Some("portside"));
}
