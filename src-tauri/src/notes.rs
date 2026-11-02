//! Keeping database notes true.
//!
//! Every time a database's schema is read, its notes are reviewed against it: a note whose
//! table or column was dropped, renamed or changed is marked doubtful, and a fix is suggested
//! for the person to accept (a rename rewrites the note; a drop removes it). Nothing changes a
//! note's text without the person. A note's check, a read-only query answering true or false,
//! tells whether what it says about the data still holds.

use crate::db::{self, DbPool};
use crate::error::AppResult;
use crate::readonly;
use crate::schema::Schema;
use crate::schema::anchors::{self, Change, Snapshot};
use crate::store::Store;
use crate::store::agent::{CheckRun, Note, now_ms};
use crate::store::suggestions::Change as Suggest;
use std::time::Duration;

/// How SayQL signs the fixes it suggests.
pub const APP: &str = "SayQL";

/// What reviewing the notes against a fresh schema found.
#[derive(Debug, Default)]
pub struct Review {
    /// How the schema changed since it was last read (empty the first time).
    pub changes: Vec<Change>,
    /// Notes that became doubtful now, with why.
    pub suspects: Vec<(String, String)>,
}

/// Reviews the notes of one database against its freshly read `schema`, and keeps the schema
/// for next time. Notes saved before anchors existed get theirs now.
pub fn review(
    store: &Store,
    connection_id: &str,
    database: Option<&str>,
    schema: &Schema,
) -> AppResult<Review> {
    let snapshot = Snapshot::of(schema);
    let old = store.schema_snapshot(connection_id, database)?;
    let changes = old
        .as_ref()
        .map(|o| anchors::diff(o, &snapshot))
        .unwrap_or_default();
    if old.as_ref() != Some(&snapshot) {
        store.save_schema_snapshot(connection_id, database, snapshot)?;
    }
    let mut out = Review {
        changes,
        suspects: Vec::new(),
    };
    for note in store.notes(connection_id, database)? {
        if note.anchors.is_empty() {
            let found = anchors::resolve(&note.text, schema);
            if !found.anchors.is_empty() {
                store.update_note(&note.id, |n| n.anchors = found.anchors.clone())?;
            }
            if let Some(missing) = found.dangling.first() {
                flag(
                    store,
                    &note,
                    &format!("it names {missing}, which doesn't exist"),
                    &mut out,
                )?;
            }
            continue;
        }
        let Some(reason) = note.anchors.iter().find_map(|a| anchors::broken(a, schema)) else {
            continue;
        };
        if note.suspect.is_some() {
            continue;
        }
        flag(store, &note, &reason, &mut out)?;
        suggest_fix(store, connection_id, database, &note, &reason, &out.changes);
    }
    Ok(out)
}

fn flag(store: &Store, note: &Note, reason: &str, out: &mut Review) -> AppResult<()> {
    if note.suspect.is_none() {
        store.mark_suspect(&note.id, reason)?;
        out.suspects.push((note.id.clone(), reason.to_string()));
    }
    Ok(())
}

/// Suggests the fix for a note whose anchor broke: the note rewritten for a rename, or its
/// removal for a drop. A changed column gets no suggestion: the note may still hold, and its
/// check (or the person) decides. A suggestion that can't be made (one is already waiting)
/// is skipped.
fn suggest_fix(
    store: &Store,
    connection_id: &str,
    database: Option<&str>,
    note: &Note,
    reason: &str,
    changes: &[Change],
) {
    let renamed = note.anchors.iter().find_map(|a| {
        let table = a.table.to_lowercase();
        changes.iter().find_map(|c| match c {
            Change::TableRenamed { from, to } if *from == table => {
                let bare = |q: &str| q.rsplit('.').next().unwrap_or(q).to_string();
                Some((
                    bare(&a.table),
                    bare(to),
                    format!("table {} was renamed to {to}", a.table),
                ))
            }
            Change::ColumnRenamed { table: t, from, to }
                if *t == table
                    && a.column
                        .as_deref()
                        .is_some_and(|c| c.eq_ignore_ascii_case(from)) =>
            {
                Some((
                    from.clone(),
                    to.clone(),
                    format!("column {}.{from} was renamed to {to}", a.table),
                ))
            }
            _ => None,
        })
    });
    let _ = match renamed {
        Some((from, to, what)) => store.suggest_note_change(
            connection_id,
            database,
            &anchors::rename_in(&note.text, &from, &to),
            &format!("The schema changed: the {what}."),
            APP,
            Suggest::Replace(&note.id),
        ),
        None if reason.contains("no longer exists") => store.suggest_note_change(
            connection_id,
            database,
            &format!("{} The note no longer applies.", capitalize(reason)),
            &format!("The schema changed: {reason}."),
            APP,
            Suggest::Remove(&note.id),
        ),
        None => return,
    };
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str() + ".",
        None => String::new(),
    }
}

/// Whether a database error says a table or column doesn't exist, in any dialect SayQL
/// supports (and as a team server passes it on).
pub fn says_missing(error: &str) -> bool {
    let e = error.to_lowercase();
    [
        "does not exist",
        "doesn't exist",
        "no such table",
        "no such column",
        "unknown column",
        "invalid column name",
        "invalid object name",
    ]
    .iter()
    .any(|p| e.contains(p))
}

/// How long a check may run.
const CHECK_TIME: Duration = Duration::from_secs(5);

/// Runs a note's check and returns its answer, keeping nothing else. The query must read
/// only and stay off never-send columns (as every query the AI runs), and return one row with
/// one column that is true or false (1 or 0); anything else is an error, so a check can't be
/// used to read data.
pub async fn run_check(pool: &DbPool, sql: &str, never_send: &[String]) -> CheckRun {
    let at = now_ms();
    let fail = |e: String| CheckRun {
        ok: None,
        at,
        error: Some(e),
    };
    if !matches!(pool, DbPool::Team(_)) && !db::is_read_only(sql, pool.kind()) {
        return fail("A check can only read.".into());
    }
    if let Err(f) = crate::never_send::check(pool, sql, never_send).await {
        return fail(first_line(&f.safe));
    }
    match readonly::run_limited(pool, sql, 2, CHECK_TIME).await {
        Err(e) => fail(first_line(&e)),
        Ok(r) if r.rows.len() != 1 || r.columns.len() != 1 => {
            fail("A check must return one row with one column: true or false.".into())
        }
        Ok(r) => match r.rows.first().and_then(|row| row.first()).and_then(truth) {
            Some(ok) => CheckRun {
                ok: Some(ok),
                at,
                error: None,
            },
            None => fail("A check must return true or false (1 or 0).".into()),
        },
    }
}

fn first_line(e: &str) -> String {
    e.lines().next().unwrap_or(e).chars().take(200).collect()
}

/// A query's answer as true or false, when it is one.
fn truth(v: &serde_json::Value) -> Option<bool> {
    match v {
        serde_json::Value::Bool(b) => Some(*b),
        serde_json::Value::Number(n) => match n.as_i64() {
            Some(1) => Some(true),
            Some(0) => Some(false),
            _ => None,
        },
        serde_json::Value::String(s) => match s.trim().to_lowercase().as_str() {
            "true" | "t" | "1" => Some(true),
            "false" | "f" | "0" => Some(false),
            _ => None,
        },
        _ => None,
    }
}

/// Runs the checks of one database's notes and records the results. With `older_than`, only
/// those not run for that long (in milliseconds). Returns how many ran.
pub async fn check_all(
    store: &Store,
    pool: &DbPool,
    connection_id: &str,
    database: Option<&str>,
    older_than: Option<u64>,
) -> AppResult<usize> {
    let now = now_ms();
    let due: Vec<String> = store
        .notes(connection_id, database)?
        .into_iter()
        .filter(|n| match (&n.check, older_than) {
            (None, _) => false,
            (Some(_), None) => true,
            (Some(c), Some(age)) => c
                .last
                .as_ref()
                .is_none_or(|l| now.saturating_sub(l.at) > age),
        })
        .map(|n| n.id)
        .collect();
    for id in &due {
        check_note(store, pool, id).await?;
    }
    Ok(due.len())
}

/// Runs the check of note `id` and records the result on it. `None` when the note is gone or
/// has no check.
pub async fn check_note(store: &Store, pool: &DbPool, id: &str) -> AppResult<Option<Note>> {
    let Some((note, sql)) = store
        .all_notes()?
        .into_iter()
        .find(|n| n.id == id)
        .and_then(|n| n.check.clone().map(|c| (n, c.sql)))
    else {
        return Ok(None);
    };
    let never_send = store.agent_access(&note.connection_id)?.never_send;
    let run = run_check(pool, &sql, &never_send).await;
    store.record_check(id, run)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{Kind, introspect, run_query, tenant_fixture};
    use crate::store::agent::{NoteCheck, render_notes};

    async fn schema(pool: &DbPool) -> Schema {
        introspect(pool, Kind::Sqlite).await.unwrap()
    }

    fn note(store: &Store, schema: &Schema, text: &str, check: Option<&str>) -> Note {
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
                Some(schema),
            )
            .unwrap()
    }

    fn get(store: &Store, id: &str) -> Note {
        store
            .all_notes()
            .unwrap()
            .into_iter()
            .find(|n| n.id == id)
            .unwrap()
    }

    /// The staleness harness: notes on the tenant fixture, then the schema and the data change
    /// under them. The right notes turn doubtful with the right fixes suggested, the rest stay.
    #[tokio::test]
    async fn notes_follow_the_database() {
        let dir = std::env::temp_dir().join(format!("sayql-notes-{}", uuid::Uuid::new_v4()));
        let store = Store::new(dir.clone());
        let pool = tenant_fixture::sqlite(&dir.join("tenant.sqlite")).await;
        let s = schema(&pool).await;

        let owner = note(
            &store,
            &s,
            "owner = memberships.role = 'owner'",
            Some("SELECT count(*) > 0 FROM memberships WHERE role = 'owner'"),
        );
        let tier = note(
            &store,
            &s,
            "tenants.tier is the plan the tenant pays for",
            None,
        );
        let login = note(
            &store,
            &s,
            "users.last_login_at is set on every sign-in",
            None,
        );
        let label = note(
            &store,
            &s,
            "appointment_statuses.label is what patients see",
            None,
        );
        let invites = note(&store, &s, "invites rows are deleted once accepted", None);
        assert_eq!(
            login
                .anchors
                .iter()
                .filter_map(|a| a.column.clone())
                .collect::<Vec<_>>(),
            ["last_login_at"]
        );
        assert_eq!(
            get(&store, &owner.id).check.unwrap().last,
            None,
            "checks run when asked"
        );

        // The first read keeps the schema; nothing is doubtful.
        let first = review(&store, "c1", None, &s).unwrap();
        assert!(first.changes.is_empty() && first.suspects.is_empty());

        // The schema changes.
        for sql in [
            "ALTER TABLE users RENAME COLUMN last_login_at TO last_seen_at",
            "ALTER TABLE appointment_statuses DROP COLUMN label",
            "DROP TABLE invites",
        ] {
            run_query(&pool, sql, false).await.unwrap();
        }
        let s2 = schema(&pool).await;
        let second = review(&store, "c1", None, &s2).unwrap();
        assert!(second.changes.contains(&Change::ColumnRenamed {
            table: "users".into(),
            from: "last_login_at".into(),
            to: "last_seen_at".into()
        }));
        let doubtful: Vec<&str> = second.suspects.iter().map(|(id, _)| id.as_str()).collect();
        for n in [&login, &label, &invites] {
            assert!(
                doubtful.contains(&n.id.as_str()),
                "{} should be doubtful",
                n.text
            );
        }
        for n in [&owner, &tier] {
            assert!(
                get(&store, &n.id).suspect.is_none(),
                "{} still holds",
                n.text
            );
        }
        assert_eq!(
            get(&store, &login.id).suspect.unwrap().reason,
            "the column users.last_login_at no longer exists"
        );

        // Fixes wait for the person: a rewrite for the rename, removals for the drops.
        let waiting = store.note_suggestions().unwrap();
        let fix = waiting
            .iter()
            .find(|w| w.replaces.as_deref() == Some(&login.id))
            .unwrap();
        assert_eq!(fix.text, "users.last_seen_at is set on every sign-in");
        assert!(!fix.remove && fix.app == APP);
        for n in [&label, &invites] {
            assert!(
                waiting
                    .iter()
                    .any(|w| w.replaces.as_deref() == Some(&n.id) && w.remove),
                "removal of {}",
                n.text
            );
        }
        // A second read doesn't suggest them again.
        review(&store, "c1", None, &s2).unwrap();
        assert_eq!(store.note_suggestions().unwrap().len(), waiting.len());

        // Accepting the rewrite updates the note, keeps the old text and clears the doubt.
        let accepted = store
            .accept_note_suggestion_with(&fix.id, Some(&s2))
            .unwrap()
            .unwrap();
        assert_eq!(accepted.id, login.id);
        assert!(accepted.suspect.is_none());
        assert_eq!(
            accepted.history[0].text,
            "users.last_login_at is set on every sign-in"
        );
        assert!(
            accepted
                .anchors
                .iter()
                .any(|a| a.column.as_deref() == Some("last_seen_at"))
        );

        // The prompt says how sure each note is.
        let text = render_notes(&store.notes("c1", None).unwrap(), false);
        assert!(
            text.contains("(may be out of date: the table invites no longer exists) invites rows"),
            "{text}"
        );
        assert!(text.contains("follow the schema"));

        // Checks: passing verifies the note, failing makes it doubtful, passing again clears it.
        assert_eq!(check_all(&store, &pool, "c1", None, None).await.unwrap(), 1);
        let n = get(&store, &owner.id);
        assert!(n.verified_at.is_some() && n.suspect.is_none());
        assert!(render_notes(&[n], false).contains("(checked today) owner"));
        run_query(&pool, "UPDATE memberships SET role = 'member'", false)
            .await
            .unwrap();
        check_all(&store, &pool, "c1", None, None).await.unwrap();
        assert!(
            get(&store, &owner.id)
                .suspect
                .unwrap()
                .reason
                .starts_with("its check failed")
        );
        run_query(
            &pool,
            "UPDATE memberships SET role = 'owner' WHERE id = 1",
            false,
        )
        .await
        .unwrap();
        check_all(&store, &pool, "c1", None, None).await.unwrap();
        assert!(
            get(&store, &owner.id).suspect.is_none(),
            "a passing check clears its own doubt"
        );
        assert_eq!(
            check_all(&store, &pool, "c1", None, Some(60_000))
                .await
                .unwrap(),
            0,
            "nothing due within the minute"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn a_check_answers_only_true_or_false() {
        let dir = std::env::temp_dir().join(format!("sayql-checks-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let pool = tenant_fixture::sqlite(&dir.join("tenant.sqlite")).await;
        let none: Vec<String> = vec![];
        let ok = run_check(&pool, "SELECT count(*) > 0 FROM users", &none).await;
        assert_eq!((ok.ok, ok.error), (Some(true), None));
        for (sql, why) in [
            ("SELECT id, email FROM users", "one row with one column"),
            ("SELECT email FROM users LIMIT 1", "true or false"),
            ("DELETE FROM users", "only read"),
        ] {
            let r = run_check(&pool, sql, &none).await;
            assert!(
                r.ok.is_none() && r.error.as_deref().unwrap().contains(why),
                "{sql}: {r:?}"
            );
        }
        let never = vec!["users.email".to_string()];
        let r = run_check(
            &pool,
            "SELECT count(*) > 0 FROM users WHERE email LIKE '%a%'",
            &never,
        )
        .await;
        assert!(
            r.ok.is_none(),
            "never-send columns stay out of checks: {r:?}"
        );
        assert!(
            says_missing("ERROR: column \"x\" does not exist") && !says_missing("syntax error")
        );
        let _ = std::fs::remove_dir_all(dir);
    }
}
