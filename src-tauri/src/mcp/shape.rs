//! Query results and errors as the connection's "AI apps may see" setting allows: row counts,
//! column summaries, a page of masked rows, or a page of rows with never-send columns withheld;
//! and the form of an error an AI app gets.

use super::*;

/// One page of a query's rows.
pub(super) struct Page {
    pub(super) rows: Vec<Vec<Value>>,
    pub(super) offset: usize,
    /// Where the next page starts, when there are more rows SayQL read.
    pub(super) next_offset: Option<usize>,
    /// More rows exist, read or not.
    pub(super) more: bool,
    /// The page reached the end of what SayQL reads, and the query has more.
    pub(super) past_cap: bool,
}

/// A query's result as the connection's "AI apps may see" setting allows: row counts, column
/// summaries, a page of masked rows, or a page of rows with never-send columns withheld.
/// `withheld`: result positions the never-send check found to hold never-send values.
pub(super) fn shape(
    level: AppResults,
    never: &mut Masker,
    sql: &str,
    withheld: &[usize],
    mut r: QueryResult,
    offset: usize,
    limit: usize,
) -> Value {
    // Which result columns are renamed personal or never-send ones, and what the SQL lets
    // SayQL trace: for counts (min/max) and masked rows alike.
    never.learn_aliases(sql);
    never.withhold_positions(withheld);
    let rows = match level {
        AppResults::Off => {
            return json!({ "columns": r.columns, "row_count": r.rows.len(), "truncated": r.truncated,
                           "elapsed_ms": r.elapsed_ms, "note": OFF_NOTE });
        }
        AppResults::Counts => {
            return json!({ "summary": never.summarize(&r), "elapsed_ms": r.elapsed_ms, "note": COUNTS_NOTE });
        }
        AppResults::Masked | AppResults::All => std::mem::take(&mut r.rows),
    };
    let elapsed_ms = r.elapsed_ms;
    let mut page = page(rows, r.truncated, offset, limit);
    let mut columns = r.columns.clone();
    let mut notes = Vec::new();
    if level == AppResults::Masked {
        let masked = never.mask(
            &QueryResult {
                rows: std::mem::take(&mut page.rows),
                ..r
            },
            usize::MAX,
        );
        (columns, page.rows) = (masked.columns, masked.rows);
        if !masked.masked_columns.is_empty() {
            notes.push(MASKED_NOTE);
        }
    } else {
        withhold(never, &columns, &mut page.rows);
    }
    if page.past_cap {
        notes.push(
            "SayQL reads at most 5,000 rows per query. For later rows, add a WHERE or LIMIT/OFFSET to the SQL; \
             to save them all, use export_csv.",
        );
    }
    let mut out = json!({
        "columns": columns,
        "row_count": page.rows.len(),
        "rows": page.rows,
        "offset": page.offset,
        "next_offset": page.next_offset,
        "truncated": page.more,
        "elapsed_ms": elapsed_ms,
    });
    if !notes.is_empty() {
        out["note"] = json!(notes.join(" "));
    }
    out
}

/// A failed query's error as the AI app may see it: the database's message only at `all` on a
/// query that reads no table with never-send columns; otherwise a fixed description of the
/// error (see safety/redact.rs), since messages can quote values.
pub(super) fn shown_error(level: AppResults, guard: &Guard, e: QueryFailure) -> String {
    if level == AppResults::All && !guard.protected {
        e.full
    } else {
        e.safe
    }
}

/// A query that failed in the database: logged in its safe form (never values), and shown as
/// `shown_error` says.
pub(super) fn failed(
    log: &mut Activity,
    level: AppResults,
    guard: &Guard,
    e: QueryFailure,
) -> String {
    log.status = "error".into();
    log.detail = Some(e.safe.chars().take(160).collect());
    shown_error(level, guard, e)
}

/// `sql` with the text of every string literal and comment replaced by `…`, read as `kind`'s
/// database reads it (every engine's way when the connection is gone).
pub(super) fn mask_literals(sql: &str, kind: Option<crate::db::Kind>) -> String {
    use crate::safety::sql_lex::Dialect;
    match kind {
        Some(k) => crate::safety::sql_lex::mask_values(sql, &[never_send::dialect(k)]),
        None => crate::safety::sql_lex::mask_values(
            sql,
            &[
                Dialect::Postgres,
                Dialect::MySql,
                Dialect::Sqlite,
                Dialect::Mssql,
            ],
        ),
    }
}

/// Marks never-send columns in the schema text the AI app reads: `ssn text [never sent to AI]`.
pub(super) fn mark_never_send(schema: &mut Schema, never: &[never_send::Entry]) {
    for t in &mut schema.tables {
        let table = t.name.to_lowercase();
        for c in &mut t.columns {
            let column = c.name.to_lowercase();
            if never
                .iter()
                .any(|e| e.column == column && e.table.as_ref().is_none_or(|x| *x == table))
            {
                c.data_type.push_str(" [never sent to AI]");
            }
        }
    }
}

/// Marks the log entry refused, with a short reason, and returns the message for the AI app.
pub(super) fn refused(log: &mut Activity, why: &str, message: String) -> String {
    log.status = "refused".into();
    log.detail = Some(why.to_string());
    message
}

/// Replaces the values of never-send columns (by name, or by position from the never-send
/// check). Call after `learn_aliases`, so a `table.column` entry applies only where its table is read.
pub(super) fn withhold(never: &Masker, columns: &[String], rows: &mut [Vec<Value>]) {
    let hidden: Vec<usize> = columns
        .iter()
        .enumerate()
        .filter(|(i, c)| never.is_withheld(*i, c))
        .map(|(i, _)| i)
        .collect();
    for row in rows.iter_mut() {
        for &i in &hidden {
            if let Some(v) = row.get_mut(i).filter(|v| !v.is_null()) {
                *v = json!("(withheld: never sent to AI)");
            }
        }
    }
}

pub(super) fn page(rows: Vec<Vec<Value>>, cut_off: bool, offset: usize, limit: usize) -> Page {
    let total = rows.len();
    let rows: Vec<_> = rows.into_iter().skip(offset).take(limit).collect();
    let end = offset.min(total) + rows.len();
    Page {
        offset,
        next_offset: (end < total).then_some(end),
        more: end < total || cut_off,
        past_cap: end >= total && cut_off,
        rows,
    }
}
