//! The agent loop with a scripted model against the tenant fixture: no API key, no network.

use super::model::{Llm, Mock, MockTurn};
use super::run::{self, Event, Io, Outcome, Reply, RunInput, StepStatus};
use crate::db::{self, DbPool, Kind, tenant_fixture};
use crate::store::agent::{AgentAccess, AgentSettings, Results};
use serde_json::json;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;

struct Fixture {
    pool: DbPool,
    dir: std::path::PathBuf,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

async fn fixture() -> Fixture {
    let dir = std::env::temp_dir().join(format!("sayql-agent-run-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    Fixture {
        pool: tenant_fixture::sqlite(&dir.join("tenant.sqlite")).await,
        dir,
    }
}

fn access(results: Results, ask: bool) -> AgentAccess {
    AgentAccess {
        enabled: true,
        results,
        ask_before_send: ask,
        never_send: vec![],
        ..Default::default()
    }
}

struct Ran {
    events: Vec<Event>,
    outcome: Outcome,
    /// Everything the model was sent, turn by turn.
    seen: Vec<String>,
}

impl Ran {
    fn done(&self) -> (&Option<run::Answer>, &Option<String>, bool) {
        match self.events.last() {
            Some(Event::Done {
                answer,
                error,
                stopped,
                ..
            }) => (answer, error, *stopped),
            other => panic!("no Done event: {other:?}"),
        }
    }

    fn steps(&self) -> Vec<run::Step> {
        steps_of(&self.events)
    }
}

/// The last event for each step id.
fn steps_of(events: &[Event]) -> Vec<run::Step> {
    let mut out: Vec<run::Step> = Vec::new();
    for e in events {
        if let Event::Step { step, .. } = e {
            match out.iter_mut().find(|s| s.id == step.id) {
                Some(s) => *s = step.clone(),
                None => out.push(step.clone()),
            }
        }
    }
    out
}

async fn go(
    pool: &DbPool,
    turns: Vec<MockTurn>,
    access: AgentAccess,
    limits: AgentSettings,
    replies: Vec<Reply>,
) -> Ran {
    go_with(
        pool,
        turns,
        access,
        limits,
        replies,
        CancellationToken::new(),
    )
    .await
}

async fn go_with(
    pool: &DbPool,
    turns: Vec<MockTurn>,
    access: AgentAccess,
    limits: AgentSettings,
    replies: Vec<Reply>,
    stop: CancellationToken,
) -> Ran {
    let schema = db::introspect(pool, pool.kind()).await.unwrap();
    let schema_text = crate::schema::render_full(&schema);
    let input = RunInput {
        run_id: "r1".into(),
        question: "Who owns Northfield Family Practice?".into(),
        kind: pool.kind(),
        schema,
        schema_text,
        notes: vec!["owner = memberships.role = 'owner'".into()],
        notes_home: None,
        focus: vec![],
        skill: None,
        access,
        databases: Default::default(),
        limits,
        history: None,
    };
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    for r in replies {
        tx.send(r).unwrap();
    }
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = events.clone();
    let mut io = Io {
        emit: Arc::new(move |e| sink.lock().unwrap().push(e)),
        replies: rx,
        stop,
    };
    let mut llm = Llm::Mock(Mock::new(turns));
    let outcome = run::run(&mut llm, pool, input, &mut io).await;
    let Llm::Mock(mock) = llm else { unreachable!() };
    let seen = mock.seen.lock().unwrap().clone();
    let events = events.lock().unwrap().clone();
    Ran {
        events,
        outcome,
        seen,
    }
}

fn plan() -> MockTurn {
    MockTurn::Tools(vec![(
        "propose_plan",
        json!({ "steps": ["Find the owner role", "Look up the owner"] }),
    )])
}

fn go_on() -> Reply {
    Reply::Go { plan: None }
}

const OWNER_EMAIL: &str = "SELECT u.email FROM memberships m JOIN users u ON u.id = m.user_id JOIN tenants t ON t.id = m.tenant_id \
                           WHERE t.name = 'Northfield Family Practice' AND m.role = 'owner'";

#[tokio::test]
async fn a_full_run_ends_with_a_cited_answer_and_sends_no_personal_data() {
    let f = fixture().await;
    let turns = vec![
        plan(),
        MockTurn::Tools(vec![(
            "profile_column",
            json!({ "table": "memberships", "column": "role" }),
        )]),
        MockTurn::Tools(vec![(
            "run_query",
            json!({ "sql": OWNER_EMAIL, "purpose": "Finding the owner" }),
        )]),
        // Follows the lead with the token it got back, without seeing the email.
        MockTurn::Dynamic(Box::new(|last| {
            let token = &last[last.find('‹').unwrap()..last.find('›').unwrap() + '›'.len_utf8()];
            assert!(token.starts_with("‹email-"), "{last}");
            vec![(
                "run_query",
                json!({ "sql": format!("SELECT full_name FROM users WHERE email = '{token}'"), "purpose": "Getting the owner's name" }),
            )]
        })),
        MockTurn::Dynamic(Box::new(|last| {
            let token = &last[last.find('‹').unwrap()..last.find('›').unwrap() + '›'.len_utf8()];
            vec![(
                "finish",
                json!({
                    "answer": format!("The owner is {token}."),
                    "citations": [{ "sentence": format!("The owner is {token}."), "steps": [2, 3] }],
                    "confidence": "high",
                    "notes": ["owner = memberships.role = 'owner'", format!("the owner is {token}")]
                }),
            )]
        })),
    ];
    let ran = go(
        &f.pool,
        turns,
        access(Results::Rows, false),
        AgentSettings::default(),
        vec![go_on()],
    )
    .await;
    let (answer, error, stopped) = ran.done();
    assert!(error.is_none() && !stopped, "{error:?}");
    let answer = answer.as_ref().unwrap();
    assert_eq!(answer.text, "The owner is Priya Natarajan.");
    assert_eq!(answer.sentences[0].steps, [2, 3]);
    assert!(answer.unverified.is_empty(), "{:?}", answer.unverified);
    assert_eq!(
        answer.notes,
        ["owner = memberships.role = 'owner'"],
        "a note with a value is dropped"
    );
    let steps = ran.steps();
    assert_eq!(steps.len(), 3);
    assert!(
        steps.iter().all(|s| s.status == StepStatus::Done),
        "{steps:?}"
    );
    assert_eq!(
        steps[2].rows,
        [[json!("Priya Natarajan")]],
        "the person sees the real rows"
    );
    assert!(
        steps[2].sql.as_deref().unwrap().contains("@example.com"),
        "the SQL that ran has the real value"
    );
    assert!(
        steps[1]
            .sent_summary
            .as_deref()
            .unwrap()
            .contains("1 column masked"),
        "{:?}",
        steps[1].sent_summary
    );
    assert!(
        ran.events
            .iter()
            .any(|e| matches!(e, Event::Plan { waiting: true, .. }))
    );
    // Nothing planted reached the model, on any turn.
    for sent in &ran.seen {
        for p in tenant_fixture::planted_pii() {
            assert!(!sent.contains(p.as_str()), "{p:?} was sent: {sent}");
        }
    }
    assert_eq!(ran.outcome.history.next_step, 4);
}

#[tokio::test]
async fn writes_never_run_and_come_back_as_proposals() {
    let f = fixture().await;
    let before = db::run_query(&f.pool, "SELECT COUNT(*) FROM appointments", true)
        .await
        .unwrap()
        .rows;
    let turns = vec![
        plan(),
        MockTurn::Tools(vec![(
            "run_query",
            json!({ "sql": "DELETE FROM appointments WHERE status_id = 3", "purpose": "Deleting" }),
        )]),
        MockTurn::Tools(vec![(
            "run_query",
            json!({ "sql": "SELECT 1; DELETE FROM appointments", "purpose": "Sneaky" }),
        )]),
        MockTurn::Tools(vec![(
            "finish",
            json!({ "answer": "Run the statement below to delete them.", "proposed_sql": "DELETE FROM appointments WHERE status_id = 3" }),
        )]),
    ];
    let ran = go(
        &f.pool,
        turns,
        access(Results::Counts, false),
        AgentSettings::default(),
        vec![go_on()],
    )
    .await;
    let steps = ran.steps();
    assert!(
        steps[0].error.as_deref().unwrap().contains("read-only"),
        "{steps:?}"
    );
    assert!(steps[1].error.is_some(), "{steps:?}");
    let after = db::run_query(&f.pool, "SELECT COUNT(*) FROM appointments", true)
        .await
        .unwrap()
        .rows;
    assert_eq!(before, after);
    let answer = ran.done().0.clone().unwrap();
    assert_eq!(
        answer.proposed_sql.as_deref(),
        Some("DELETE FROM appointments WHERE status_id = 3")
    );
    assert!(answer.proposed_changes_data);
}

#[tokio::test]
async fn limits_hold() {
    let f = fixture().await;
    let q = |sql: &str| MockTurn::Tools(vec![("run_query", json!({ "sql": sql, "purpose": "x" }))]);
    let limits = AgentSettings {
        max_queries: 1,
        max_steps: 4,
        skip_plan: true,
        ..Default::default()
    };
    let turns = vec![
        plan(),
        q("SELECT 1"),
        q("SELECT 2"),
        MockTurn::Text("I'm not sure."),
    ];
    let ran = go(
        &f.pool,
        turns,
        access(Results::Counts, false),
        limits,
        vec![],
    )
    .await;
    let steps = ran.steps();
    assert!(
        steps[1]
            .error
            .as_deref()
            .unwrap()
            .contains("used its 1 queries"),
        "{steps:?}"
    );
    // The last turn was offered only finish, and told so.
    assert!(
        ran.seen[3].contains("reached the step limit"),
        "{}",
        ran.seen[3]
    );
    assert_eq!(ran.done().0.as_ref().unwrap().confidence, "low");
    assert!(
        ran.events
            .iter()
            .any(|e| matches!(e, Event::Plan { waiting: false, .. }))
    );
}

#[tokio::test]
async fn stop_ends_a_waiting_model_quickly() {
    let f = fixture().await;
    let stop = CancellationToken::new();
    let later = stop.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(200)).await;
        later.cancel();
    });
    let start = Instant::now();
    let ran = go_with(
        &f.pool,
        vec![MockTurn::Hang],
        access(Results::Counts, false),
        AgentSettings::default(),
        vec![],
        stop,
    )
    .await;
    assert!(ran.done().2, "stopped");
    assert!(
        start.elapsed() < Duration::from_secs(2),
        "{:?}",
        start.elapsed()
    );
}

#[tokio::test]
async fn stop_ends_a_running_query_quickly() {
    let f = fixture().await;
    let slow = "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 500000000) SELECT COUNT(*) FROM n";
    let turns = vec![MockTurn::Tools(vec![(
        "run_query",
        json!({ "sql": slow, "purpose": "slow" }),
    )])];
    let stop = CancellationToken::new();
    let later = stop.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(300)).await;
        later.cancel();
    });
    let start = Instant::now();
    let ran = go_with(
        &f.pool,
        turns,
        access(Results::Counts, false),
        AgentSettings::default(),
        vec![],
        stop,
    )
    .await;
    assert!(ran.done().2, "stopped: {:?}", ran.done());
    assert!(
        start.elapsed() < Duration::from_secs(2),
        "{:?}",
        start.elapsed()
    );
}

#[tokio::test]
async fn the_run_time_limit_stops_a_slow_model() {
    let f = fixture().await;
    let limits = AgentSettings {
        run_seconds: 1,
        ..Default::default()
    };
    let ran = go(
        &f.pool,
        vec![MockTurn::Hang],
        access(Results::Counts, false),
        limits,
        vec![],
    )
    .await;
    assert!(
        ran.done().1.as_deref().unwrap().contains("time limit"),
        "{:?}",
        ran.done()
    );
}

#[tokio::test]
async fn ask_before_send_pauses_and_skip_sends_only_the_count() {
    let f = fixture().await;
    let turns = vec![
        plan(),
        MockTurn::Tools(vec![(
            "run_query",
            json!({ "sql": "SELECT full_name FROM users", "purpose": "names" }),
        )]),
        MockTurn::Tools(vec![(
            "run_query",
            json!({ "sql": "SELECT COUNT(*) AS n FROM users", "purpose": "count" }),
        )]),
        MockTurn::Tools(vec![("finish", json!({ "answer": "There are 24 users." }))]),
    ];
    let ran = go(
        &f.pool,
        turns,
        access(Results::Rows, true),
        AgentSettings::default(),
        vec![go_on(), Reply::Skip, Reply::Send],
    )
    .await;
    let sends: Vec<&String> = ran
        .events
        .iter()
        .filter_map(|e| {
            if let Event::Send { payload, .. } = e {
                Some(payload)
            } else {
                None
            }
        })
        .collect();
    assert_eq!(sends.len(), 2);
    assert!(
        sends[0].contains("‹name-1›") && !sends[0].contains("Dana Reyes"),
        "the pause shows the masked payload: {}",
        sends[0]
    );
    let steps = ran.steps();
    assert!(
        steps[0]
            .sent
            .as_deref()
            .unwrap()
            .contains("chose not to send"),
        "{:?}",
        steps[0].sent
    );
    // The prompt's own example uses ‹name-1›; the skipped rows add none.
    assert_eq!(
        ran.seen[2].matches("‹name-1›").count(),
        ran.seen[0].matches("‹name-1›").count(),
        "skipped rows never reached the model"
    );
    assert!(ran.seen[2].contains("chose not to send"));
    assert!(steps[1].sent.as_deref().unwrap().contains("24"));
    assert!(ran.done().0.as_ref().unwrap().unverified.is_empty());
}

#[tokio::test]
async fn counts_and_off_send_no_values() {
    let f = fixture().await;
    for results in [Results::Off, Results::Counts] {
        let turns = vec![
            MockTurn::Tools(vec![(
                "run_query",
                json!({ "sql": "SELECT t.name, COUNT(*) AS n FROM tenants t JOIN patients p ON p.tenant_id = t.id GROUP BY t.name", "purpose": "x" }),
            )]),
            MockTurn::Tools(vec![(
                "profile_column",
                json!({ "table": "patients", "column": "last_name" }),
            )]),
            MockTurn::Tools(vec![("finish", json!({ "answer": "Done." }))]),
        ];
        let ran = go(
            &f.pool,
            turns,
            access(results, false),
            AgentSettings::default(),
            vec![],
        )
        .await;
        let sent = ran.seen.join("\n");
        for hidden in ["Harbor Lane Clinic", "Pinecrest", "Abernathy", "Kowalczyk"] {
            assert!(!sent.contains(hidden), "{results:?}: {hidden} was sent");
        }
        assert!(
            ran.steps()[0].rows.len() == 4,
            "the person still sees the rows"
        );
    }
}

#[tokio::test]
async fn never_send_columns_are_refused_in_sql() {
    let f = fixture().await;
    let turns = vec![
        MockTurn::Tools(vec![(
            "run_query",
            json!({ "sql": "SELECT COUNT(*) FROM patients WHERE notes LIKE '%penicillin%'", "purpose": "x" }),
        )]),
        MockTurn::Tools(vec![(
            "finish",
            json!({ "answer": "I can't check notes." , "cannot_answer": true }),
        )]),
    ];
    let mut a = access(Results::Rows, false);
    a.never_send = vec!["patients.notes".into()];
    let ran = go(&f.pool, turns, a, AgentSettings::default(), vec![]).await;
    assert!(
        ran.steps()[0]
            .error
            .as_deref()
            .unwrap()
            .contains("never send"),
        "{:?}",
        ran.steps()
    );
    assert!(ran.done().0.as_ref().unwrap().cannot_answer);
}

#[tokio::test]
async fn ask_user_waits_for_the_answer() {
    let f = fixture().await;
    let turns = vec![
        plan(),
        MockTurn::Tools(vec![(
            "ask_user",
            json!({ "question": "Owner as creator, or the owner role?", "choices": ["Creator", "Owner role"] }),
        )]),
        MockTurn::Tools(vec![("finish", json!({ "answer": "OK." }))]),
    ];
    let ran = go(
        &f.pool,
        turns,
        access(Results::Counts, false),
        AgentSettings::default(),
        vec![
            go_on(),
            Reply::Answer {
                text: "Owner role".into(),
            },
        ],
    )
    .await;
    assert!(
        ran.events
            .iter()
            .any(|e| matches!(e, Event::Question { choices, .. } if choices.len() == 2))
    );
    assert!(
        ran.seen[2].contains("The person answered: Owner role"),
        "{}",
        ran.seen[2]
    );
}

#[tokio::test]
async fn an_edited_plan_is_followed() {
    let f = fixture().await;
    let turns = vec![
        plan(),
        MockTurn::Tools(vec![("finish", json!({ "answer": "OK." }))]),
    ];
    let edited = Reply::Go {
        plan: Some(vec!["Only check memberships".into()]),
    };
    let ran = go(
        &f.pool,
        turns,
        access(Results::Counts, false),
        AgentSettings::default(),
        vec![edited],
    )
    .await;
    assert!(
        ran.seen[1].contains("1. Only check memberships"),
        "{}",
        ran.seen[1]
    );
}

#[tokio::test]
async fn made_up_numbers_and_double_counting_joins_are_flagged() {
    let f = fixture().await;
    let sum = "SELECT SUM(t.id) FROM tenants t JOIN patients p ON p.tenant_id = t.id";
    let turns = vec![
        MockTurn::Tools(vec![(
            "run_query",
            json!({ "sql": "SELECT COUNT(*) AS n FROM appointments", "purpose": "count" }),
        )]),
        MockTurn::Tools(vec![("run_query", json!({ "sql": sum, "purpose": "sum" }))]),
        MockTurn::Tools(vec![(
            "finish",
            json!({ "answer": "There were 255 appointments, up 12% on last year." }),
        )]),
    ];
    let ran = go(
        &f.pool,
        turns,
        access(Results::Counts, false),
        AgentSettings::default(),
        vec![],
    )
    .await;
    let answer = ran.done().0.clone().unwrap();
    assert_eq!(answer.unverified, ["12"], "255 is in a result; 12% is not");
    let steps = ran.steps();
    assert!(
        steps[1].checks[0].contains("tenants rows repeat"),
        "{:?}",
        steps[1].checks
    );
    assert!(
        ran.seen[2].contains("Check: tenants rows repeat"),
        "the model is told"
    );
}

#[tokio::test]
async fn a_follow_up_continues_the_conversation() {
    let f = fixture().await;
    let first = go(
        &f.pool,
        vec![MockTurn::Tools(vec![(
            "finish",
            json!({ "answer": "Hi." }),
        )])],
        access(Results::Counts, false),
        AgentSettings::default(),
        vec![],
    )
    .await;
    let schema = db::introspect(&f.pool, Kind::Sqlite).await.unwrap();
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<Reply>();
    drop(tx);
    let mut io = Io {
        emit: Arc::new(|_| {}),
        replies: rx,
        stop: CancellationToken::new(),
    };
    let input = RunInput {
        run_id: "r2".into(),
        question: "Follow-up from the person: and Harbor?".into(),
        kind: Kind::Sqlite,
        schema_text: crate::schema::render_full(&schema),
        schema,
        notes: vec![],
        notes_home: None,
        focus: vec![],
        skill: None,
        access: access(Results::Counts, false),
        databases: Default::default(),
        limits: AgentSettings::default(),
        history: Some(first.outcome.history),
    };
    let mut llm = Llm::Mock(Mock::new(vec![MockTurn::Tools(vec![(
        "finish",
        json!({ "answer": "Also." }),
    )])]));
    let out = run::run(&mut llm, &f.pool, input, &mut io).await;
    let Llm::Mock(mock) = llm else { unreachable!() };
    let seen = mock.seen.lock().unwrap().clone();
    assert!(
        seen[0].contains("Who owns Northfield") && seen[0].contains("and Harbor?"),
        "{}",
        seen[0]
    );
    assert_eq!(out.answer.unwrap().text, "Also.");
}

#[tokio::test]
async fn big_tables_need_a_filter_or_a_checked_plan() {
    let f = fixture().await;
    let mut schema = db::introspect(&f.pool, Kind::Sqlite).await.unwrap();
    schema
        .tables
        .iter_mut()
        .find(|t| t.name == "audit_log")
        .unwrap()
        .row_estimate = Some(9_000_000);
    let all = "SELECT COUNT(*) AS n FROM audit_log";
    let turns = vec![
        MockTurn::Tools(vec![("run_query", json!({ "sql": all, "purpose": "x" }))]),
        MockTurn::Tools(vec![("explain_query", json!({ "sql": all }))]),
        MockTurn::Tools(vec![("run_query", json!({ "sql": all, "purpose": "x" }))]),
        MockTurn::Tools(vec![("finish", json!({ "answer": "Done." }))]),
    ];
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<Reply>();
    drop(tx);
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = events.clone();
    let mut io = Io {
        emit: Arc::new(move |e| sink.lock().unwrap().push(e)),
        replies: rx,
        stop: CancellationToken::new(),
    };
    let input = RunInput {
        run_id: "r".into(),
        question: "How many audit rows?".into(),
        kind: Kind::Sqlite,
        schema_text: crate::schema::render_full(&schema),
        schema,
        notes: vec![],
        notes_home: None,
        focus: vec![],
        skill: None,
        access: access(Results::Counts, false),
        databases: Default::default(),
        limits: AgentSettings::default(),
        history: None,
    };
    let mut llm = Llm::Mock(Mock::new(turns));
    run::run(&mut llm, &f.pool, input, &mut io).await;
    let steps = steps_of(&events.lock().unwrap());
    assert!(
        steps[0]
            .error
            .as_deref()
            .unwrap()
            .contains("reads all of audit_log"),
        "{steps:?}"
    );
    assert_eq!(steps[1].status, StepStatus::Done, "{steps:?}");
    assert_eq!(
        steps[2].status,
        StepStatus::Done,
        "after the plan check it runs: {steps:?}"
    );
}

#[tokio::test]
async fn follow_ups_mask_values_the_person_quotes() {
    let f = fixture().await;
    let first = go(
        &f.pool,
        vec![
            MockTurn::Tools(vec![(
                "run_query",
                json!({ "sql": "SELECT email FROM users WHERE id = 9", "purpose": "x" }),
            )]),
            MockTurn::Tools(vec![("finish", json!({ "answer": "Found it." }))]),
        ],
        access(Results::Rows, false),
        AgentSettings::default(),
        vec![],
    )
    .await;
    let schema = db::introspect(&f.pool, Kind::Sqlite).await.unwrap();
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<Reply>();
    drop(tx);
    let mut io = Io {
        emit: Arc::new(|_| {}),
        replies: rx,
        stop: CancellationToken::new(),
    };
    let input = RunInput {
        run_id: "r2".into(),
        question: "I changed step 1's query. Run this instead: SELECT full_name FROM users WHERE email = 'priya.natarajan@example.com'".into(),
        kind: Kind::Sqlite,
        schema_text: crate::schema::render_full(&schema),
        schema,
        notes: vec![],
        notes_home: None,
        focus: vec![],
        skill: None,
        access: access(Results::Rows, false),
        databases: Default::default(),
        limits: AgentSettings::default(),
        history: Some(first.outcome.history),
    };
    let mut llm = Llm::Mock(Mock::new(vec![MockTurn::Tools(vec![(
        "finish",
        json!({ "answer": "OK." }),
    )])]));
    run::run(&mut llm, &f.pool, input, &mut io).await;
    let Llm::Mock(mock) = llm else { unreachable!() };
    let seen = mock.seen.lock().unwrap().clone();
    assert!(
        !seen[0].contains("priya.natarajan@example.com") && seen[0].contains("'‹email-1›'"),
        "{}",
        seen[0]
    );
}

/// profile_column, explain_query and a token follow-up on every database: SQLite always, the
/// servers when SAYQL_TEST_*_URL is set (see db/integration_tests.rs).
#[tokio::test]
async fn tools_work_on_every_database() {
    let f = fixture().await;
    let mut pools = vec![f.pool.clone()];
    pools.extend(tenant_fixture::servers().await);
    for pool in &pools {
        let kind = pool.kind();
        let turns = vec![
            MockTurn::Tools(vec![(
                "profile_column",
                json!({ "table": "appointments", "column": "status_id" }),
            )]),
            MockTurn::Tools(vec![(
                "explain_query",
                json!({ "sql": "SELECT COUNT(*) FROM appointments WHERE tenant_id = 3" }),
            )]),
            MockTurn::Tools(vec![(
                "show_value",
                json!({ "table": "tenants", "column": "name", "key_column": "id", "key": 3 }),
            )]),
            MockTurn::Tools(vec![(
                "run_query",
                json!({ "sql": "SELECT email FROM users WHERE id = 9", "purpose": "x" }),
            )]),
            MockTurn::Dynamic(Box::new(|last| {
                let token =
                    &last[last.find('‹').unwrap()..last.find('›').unwrap() + '›'.len_utf8()];
                vec![(
                    "run_query",
                    json!({ "sql": format!("SELECT full_name FROM users WHERE email = '{token}'"), "purpose": "y" }),
                )]
            })),
            MockTurn::Tools(vec![("finish", json!({ "answer": "Done." }))]),
        ];
        let schema = db::introspect(pool, kind).await.unwrap();
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<Reply>();
        drop(tx);
        let events = Arc::new(Mutex::new(Vec::new()));
        let sink = events.clone();
        let mut io = Io {
            emit: Arc::new(move |e| sink.lock().unwrap().push(e)),
            replies: rx,
            stop: CancellationToken::new(),
        };
        let input = RunInput {
            run_id: "r".into(),
            question: "q".into(),
            kind,
            schema_text: crate::schema::render_full(&schema),
            schema,
            notes: vec![],
            notes_home: None,
            focus: vec![],
            skill: None,
            access: access(Results::Rows, false),
            databases: Default::default(),
            limits: AgentSettings::default(),
            history: None,
        };
        let mut llm = Llm::Mock(Mock::new(turns));
        run::run(&mut llm, pool, input, &mut io).await;
        let steps = steps_of(&events.lock().unwrap());
        let profile = &steps[0];
        assert_eq!(profile.status, StepStatus::Done, "{kind:?}: {profile:?}");
        assert!(
            profile.sent.as_deref().unwrap().contains("\"rows\":255"),
            "{kind:?}: {:?}",
            profile.sent
        );
        if kind == Kind::Mssql {
            assert!(
                steps[1]
                    .error
                    .as_deref()
                    .unwrap()
                    .contains("SQL Server plans"),
                "{kind:?}"
            );
        } else {
            assert_eq!(
                steps[1].status,
                StepStatus::Done,
                "{kind:?}: {:?}",
                steps[1]
            );
        }
        assert_eq!(
            steps[2].rows,
            [[json!("Pinecrest Physio")]],
            "{kind:?}: {:?}",
            steps[2]
        );
        assert_eq!(
            steps[4].rows,
            [[json!("Priya Natarajan")]],
            "{kind:?}: {:?}",
            steps[4]
        );
    }
}

/// The leak the first Sonnet evaluation found: a name column under another name.
#[tokio::test]
async fn renamed_name_columns_never_reach_the_model() {
    let f = fixture().await;
    let sql = "select t.id, c.full_name creator, u.full_name from tenants t join users c on c.id = t.created_by_user_id \
               join memberships m on m.tenant_id = t.id and m.role = 'owner' join users u on u.id = m.user_id";
    let turns = vec![
        MockTurn::Tools(vec![(
            "run_query",
            json!({ "sql": sql, "purpose": "owners" }),
        )]),
        MockTurn::Tools(vec![("finish", json!({ "answer": "Done." }))]),
    ];
    let ran = go(
        &f.pool,
        turns,
        access(Results::Rows, false),
        AgentSettings::default(),
        vec![],
    )
    .await;
    assert_eq!(
        ran.steps()[0].rows.len(),
        4,
        "the person sees the real rows"
    );
    for name in [
        "Dana Reyes",
        "Grace Holloway",
        "Priya Natarajan",
        "Noor Haddad",
        "Victor Lindqvist",
    ] {
        assert!(!ran.seen[1].contains(name), "{name} reached the model");
    }
}

/// On "counts only" the agent can still name who it found: show_value gives it a token, and
/// the person sees the name.
#[tokio::test]
async fn show_value_names_things_without_sending_them() {
    let f = fixture().await;
    let turns = vec![
        MockTurn::Tools(vec![(
            "show_value",
            json!({ "table": "users", "column": "full_name", "key_column": "id", "key": 9 }),
        )]),
        MockTurn::Dynamic(Box::new(|last| {
            let token = &last[last.find('‹').unwrap()..last.find('›').unwrap() + '›'.len_utf8()];
            vec![(
                "finish",
                json!({ "answer": format!("{token} owns Northfield.") }),
            )]
        })),
    ];
    let ran = go(
        &f.pool,
        turns,
        access(Results::Counts, false),
        AgentSettings::default(),
        vec![],
    )
    .await;
    assert_eq!(
        ran.done().0.as_ref().unwrap().text,
        "Priya Natarajan owns Northfield."
    );
    assert!(
        !ran.seen.join("\n").contains("Priya"),
        "the name never reached the model"
    );
    assert_eq!(
        ran.steps()[0].sent_summary.as_deref(),
        Some("sent a token only")
    );
}

/// show_value with a query: the name of whoever the query picks, without the model seeing it.
#[tokio::test]
async fn show_value_takes_a_query() {
    let f = fixture().await;
    let sql = "SELECT u.full_name FROM users u WHERE u.id = (SELECT user_id FROM memberships GROUP BY user_id ORDER BY COUNT(*) DESC, user_id LIMIT 1)";
    let turns = vec![
        MockTurn::Tools(vec![("show_value", json!({ "sql": sql }))]),
        MockTurn::Dynamic(Box::new(|last| {
            let token = &last[last.find('‹').unwrap()..last.find('›').unwrap() + '›'.len_utf8()];
            vec![(
                "finish",
                json!({ "answer": format!("{token} is in the most tenants.") }),
            )]
        })),
    ];
    let ran = go(
        &f.pool,
        turns,
        access(Results::Off, false),
        AgentSettings::default(),
        vec![],
    )
    .await;
    assert_eq!(
        ran.done().0.as_ref().unwrap().text,
        "Dana Reyes is in the most tenants."
    );
    assert!(
        !ran.seen.join("\n").contains("Dana"),
        "the name never reached the model"
    );
}

/// An answer that claims something about every row is sent back once to be checked.
#[tokio::test]
async fn claims_about_all_rows_are_checked_once() {
    let f = fixture().await;
    let turns = vec![
        MockTurn::Tools(vec![(
            "run_query",
            json!({ "sql": "SELECT COUNT(*) AS n FROM appointments WHERE status_id = 3", "purpose": "x" }),
        )]),
        MockTurn::Tools(vec![(
            "finish",
            json!({ "answer": "There are 25 cancelled appointments. Every appointment has a patient." }),
        )]),
        MockTurn::Tools(vec![(
            "run_query",
            json!({ "sql": "SELECT COUNT(*) AS n FROM appointments a LEFT JOIN patients p ON p.id = a.patient_id WHERE p.id IS NULL", "purpose": "check" }),
        )]),
        MockTurn::Tools(vec![(
            "finish",
            json!({ "answer": "There are 25 cancelled appointments. 3 appointments have no patient." }),
        )]),
    ];
    let ran = go(
        &f.pool,
        turns,
        access(Results::Counts, false),
        AgentSettings::default(),
        vec![],
    )
    .await;
    assert!(
        ran.seen[2].contains("- Every appointment has a patient"),
        "{}",
        ran.seen[2]
    );
    assert!(
        !ran.seen[2].contains("- There are 25"),
        "only the sweeping claim is sent back"
    );
    assert_eq!(
        ran.done().0.as_ref().unwrap().text,
        "There are 25 cancelled appointments. 3 appointments have no patient."
    );
    assert!(ran.steps().iter().any(|s| s.tool == "review"));
    // A second sweeping answer stands: the check runs once.
    let again = go(
        &f.pool,
        vec![
            MockTurn::Tools(vec![(
                "run_query",
                json!({ "sql": "SELECT 1", "purpose": "x" }),
            )]),
            MockTurn::Tools(vec![("finish", json!({ "answer": "All good." }))]),
            MockTurn::Tools(vec![("finish", json!({ "answer": "All good, checked." }))]),
        ],
        access(Results::Counts, false),
        AgentSettings::default(),
        vec![],
    )
    .await;
    assert_eq!(again.done().0.as_ref().unwrap().text, "All good, checked.");
}

/// On PostgreSQL, tables whose row-level security the login can't bypass are flagged to the
/// agent and the person (SAYQL_TEST_PG_URL; see db/integration_tests.rs).
#[tokio::test]
async fn row_level_security_is_flagged_on_postgres() {
    let Some((mut cfg, password)) = db::env_config("SAYQL_TEST_PG_URL", Kind::Postgres) else {
        return;
    };
    let mut pools = tenant_fixture::servers().await;
    let Some(admin) = pools
        .iter()
        .position(|p| p.kind() == Kind::Postgres)
        .map(|i| pools.remove(i))
    else {
        return;
    };
    for sql in [
        "DROP SCHEMA IF EXISTS sayql_rls CASCADE",
        "DROP ROLE IF EXISTS sayql_rls_reader",
        "CREATE SCHEMA sayql_rls",
        "CREATE TABLE sayql_rls.claims (id int PRIMARY KEY)",
        "INSERT INTO sayql_rls.claims VALUES (1), (2), (3)",
        "ALTER TABLE sayql_rls.claims ENABLE ROW LEVEL SECURITY",
        "CREATE ROLE sayql_rls_reader LOGIN PASSWORD 'rls-test-only'",
        "GRANT USAGE ON SCHEMA sayql_rls TO sayql_rls_reader",
        "GRANT SELECT ON sayql_rls.claims TO sayql_rls_reader",
    ] {
        db::run_query(&admin, sql, false)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"));
    }
    cfg.database = tenant_fixture::DATABASE.into();
    cfg.user = "sayql_rls_reader".into();
    let _ = password;
    let reader = db::connect(&cfg, Some("rls-test-only".into()))
        .await
        .unwrap();
    let turns = vec![
        MockTurn::Tools(vec![(
            "run_query",
            json!({ "sql": "SELECT COUNT(*) AS n FROM sayql_rls.claims", "purpose": "x" }),
        )]),
        MockTurn::Tools(vec![(
            "finish",
            json!({ "answer": "Row-level security may hide rows." }),
        )]),
    ];
    let ran = go(
        &reader,
        turns,
        access(Results::Counts, false),
        AgentSettings::default(),
        vec![],
    )
    .await;
    let steps = ran.steps();
    assert_eq!(steps[0].tool, "notice", "{steps:?}");
    assert!(
        steps[0]
            .sent
            .as_deref()
            .unwrap()
            .contains("sayql_rls.claims"),
        "{:?}",
        steps[0].sent
    );
    assert!(
        ran.seen[0].contains("Row-level security is on for 1 tables (sayql_rls.claims)"),
        "the agent is told"
    );
    assert_eq!(steps[1].rows, [[json!(0)]], "the login really sees no rows");
    // The superuser bypasses it, so nothing is flagged there.
    let ran = go(
        &admin,
        vec![MockTurn::Tools(vec![("finish", json!({ "answer": "x" }))])],
        access(Results::Counts, false),
        AgentSettings::default(),
        vec![],
    )
    .await;
    assert!(ran.steps().is_empty());
    for sql in [
        "DROP SCHEMA sayql_rls CASCADE",
        "DROP OWNED BY sayql_rls_reader",
        "DROP ROLE sayql_rls_reader",
    ] {
        let _ = db::run_query(&admin, sql, false).await;
    }
}

/// Notes in a run: the model checks a saved note, a query on a column that no longer exists
/// makes SayQL read the schema again (and flags the note about it), and the notes `finish`
/// suggests (a fix to a saved note, with a check, and a new one) wait for the person.
#[tokio::test]
async fn a_run_checks_notes_and_suggests_fixes_for_the_person() {
    use crate::store::Store;
    use crate::store::agent::{Note, NoteCheck, note_ref};
    let f = fixture().await;
    let store = Store::new(f.dir.join("config"));
    let schema = db::introspect(&f.pool, Kind::Sqlite).await.unwrap();
    let save = |text: &str, check: Option<&str>| {
        store
            .save_note_with(
                Note {
                    id: uuid::Uuid::new_v4().to_string(),
                    connection_id: "c1".into(),
                    text: text.into(),
                    check: check.map(|c| NoteCheck {
                        sql: c.into(),
                        last: None,
                    }),
                    ..Default::default()
                },
                Some(&schema),
            )
            .unwrap()
    };
    let owner = save(
        "owner = memberships.role = 'owner'",
        Some("SELECT count(*) > 0 FROM memberships WHERE role = 'owner'"),
    );
    let login = save("users.last_login_at is set on every sign-in", None);
    crate::notes::review(&store, "c1", None, &schema).unwrap();
    // The column is renamed after the run read the schema.
    db::run_query(
        &f.pool,
        "ALTER TABLE users RENAME COLUMN last_login_at TO last_seen_at",
        false,
    )
    .await
    .unwrap();
    let (owner_ref, login_ref) = (note_ref(&owner.id), note_ref(&login.id));
    let turns = vec![
        MockTurn::Tools(vec![("check_note", json!({ "note": owner_ref }))]),
        MockTurn::Tools(vec![(
            "run_query",
            json!({ "sql": "SELECT max(last_login_at) FROM users", "purpose": "Last sign-in" }),
        )]),
        MockTurn::Tools(vec![(
            "finish",
            json!({
                "answer": "Sign-ins are in users.last_seen_at.",
                "confidence": "high",
                "notes": [
                    {
                        "text": "users.last_seen_at is set on every sign-in",
                        "replaces": login_ref,
                        "check": "SELECT count(*) >= 0 FROM users WHERE last_seen_at IS NOT NULL",
                        "evidence": "Step 2: last_login_at no longer exists."
                    },
                    "tenants.tier is the plan the tenant pays for"
                ]
            }),
        )]),
    ];
    let input = RunInput {
        run_id: "r1".into(),
        question: "When did people last sign in?".into(),
        kind: Kind::Sqlite,
        schema_text: crate::schema::render_full(&schema),
        schema,
        notes: store.notes("c1", None).unwrap(),
        notes_home: Some(run::NotesHome {
            store: store.clone(),
            connection_id: "c1".into(),
            database: None,
            team: None,
        }),
        focus: vec![],
        skill: None,
        access: access(Results::Counts, false),
        databases: Default::default(),
        limits: AgentSettings {
            skip_plan: true,
            ..Default::default()
        },
        history: None,
    };
    let (_tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = events.clone();
    let mut io = Io {
        emit: Arc::new(move |e| sink.lock().unwrap().push(e)),
        replies: rx,
        stop: CancellationToken::new(),
    };
    let mut llm = Llm::Mock(Mock::new(turns));
    run::run(&mut llm, &f.pool, input, &mut io).await;
    let Llm::Mock(mock) = llm else { unreachable!() };
    let seen = mock.seen.lock().unwrap().clone();

    assert!(
        seen[0].contains(&format!("[{owner_ref}] owner")),
        "notes go with their ids"
    );
    assert!(seen[1].contains("The check passed"), "{}", seen[1]);
    let notes = store.notes("c1", None).unwrap();
    assert!(
        notes
            .iter()
            .any(|n| n.id == owner.id && n.verified_at.is_some())
    );
    assert!(
        seen[2].contains("SayQL read the schema again")
            && seen[2].contains("users.last_login_at was renamed to last_seen_at"),
        "{}",
        seen[2]
    );
    assert!(
        notes
            .iter()
            .any(|n| n.id == login.id && n.suspect.is_some()),
        "after the reread, the note about the renamed column is doubtful"
    );

    let events = events.lock().unwrap().clone();
    let Some(Event::Done {
        answer: Some(answer),
        ..
    }) = events.last()
    else {
        panic!("no answer: {:?}", events.last());
    };
    assert!(answer.notes_queued);
    let waiting = store.note_suggestions().unwrap();
    assert!(
        waiting
            .iter()
            .any(|s| s.app == "Investigate" && s.text.starts_with("tenants.tier")),
        "{waiting:?}"
    );
    assert!(
        steps_of(&events)
            .iter()
            .any(|s| s.tool == "check_note" && s.status == StepStatus::Done)
    );
}

/// On a team connection whose server keeps shared notes, the run reads the team's notes and
/// the notes `finish` suggests go to the server for its knowledge owners, not to the local
/// queue. A fix names the team note by the short id the model saw.
#[tokio::test]
async fn on_a_team_connection_suggested_notes_go_to_the_team_server() {
    use crate::store::Store;
    use crate::store::agent::note_ref;
    use crate::team::knowledge::{Cache, tests::knowledge_server};
    use crate::team::tests::{client, fake};
    let f = fixture().await;
    let server = fake(knowledge_server).await;
    let store = Store::new(f.dir.join("config"));
    let cache = Arc::new(Cache::default());
    let shared = cache.notes(&client(&server.url), "team:c_1").await.unwrap();
    let schema = db::introspect(&f.pool, Kind::Sqlite).await.unwrap();
    let login_ref = note_ref("kn_bbbb2222");
    let turns = vec![MockTurn::Tools(vec![(
        "finish",
        json!({
            "answer": "Sign-ins are in users.last_seen_at.",
            "confidence": "high",
            "notes": [
                { "text": "users.last_seen_at is set on every sign-in", "replaces": login_ref,
                  "evidence": "last_login_at no longer exists." },
                "tenants.tier is the plan the tenant pays for"
            ]
        }),
    )])];
    let input = RunInput {
        run_id: "r1".into(),
        question: "When did people last sign in?".into(),
        kind: Kind::Sqlite,
        schema_text: crate::schema::render_full(&schema),
        schema,
        notes: shared.notes,
        notes_home: Some(run::NotesHome {
            store: store.clone(),
            connection_id: "team:c_1".into(),
            database: None,
            team: Some(run::TeamNotes {
                client: client(&server.url),
                cache: cache.clone(),
            }),
        }),
        focus: vec![],
        skill: None,
        access: access(Results::Counts, false),
        databases: Default::default(),
        limits: AgentSettings {
            skip_plan: true,
            ..Default::default()
        },
        history: None,
    };
    let (_tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = events.clone();
    let mut io = Io {
        emit: Arc::new(move |e| sink.lock().unwrap().push(e)),
        replies: rx,
        stop: CancellationToken::new(),
    };
    let mut llm = Llm::Mock(Mock::new(turns));
    run::run(&mut llm, &f.pool, input, &mut io).await;
    let Llm::Mock(mock) = llm else { unreachable!() };
    let seen = mock.seen.lock().unwrap().clone();
    assert!(
        seen[0].contains("users.last_login_at is set on every sign-in")
            && seen[0]
                .contains("may be out of date: the column users.last_login_at no longer exists"),
        "the prompt has the team's notes: {}",
        seen[0]
    );

    let events = events.lock().unwrap().clone();
    let Some(Event::Done {
        answer: Some(answer),
        ..
    }) = events.last()
    else {
        panic!("no answer: {:?}", events.last());
    };
    assert!(answer.notes_shared && answer.notes_queued);
    assert_eq!(answer.notes.len(), 2);
    assert!(
        store.note_suggestions().unwrap().is_empty(),
        "nothing waits on this computer"
    );
    let posted: Vec<serde_json::Value> = server
        .seen
        .lock()
        .unwrap()
        .iter()
        .filter(|s| s.path == "/api/v1/connections/c_1/knowledge/suggestions")
        .map(|s| s.body.clone())
        .collect();
    assert_eq!(posted.len(), 2);
    assert_eq!(posted[0]["replaces"], "kn_bbbb2222", "the server's own id");
    assert_eq!(posted[0]["source"], "Investigate");
    assert_eq!(posted[0]["evidence"], "last_login_at no longer exists.");
    assert_eq!(
        posted[1]["text"],
        "tenants.tier is the plan the tenant pays for"
    );
}
