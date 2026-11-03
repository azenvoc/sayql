//! Masks likely personal data in query results before agent mode sends them to an AI
//! provider, and fills it back in locally.
//!
//! A value is masked when its column name looks personal (email, phone, name, address,
//! birth, ssn, mrn, patient, token, password, notes, ...), when the person marked the column
//! "never send", or when the value itself is shaped like personal data (emails, phone
//! numbers, card-like numbers, US social security numbers, IP addresses, long ID-like strings,
//! and free text containing any of those). JSON objects and arrays are always masked.
//!
//! Masked values become tokens such as `‹email-3›`, the same token for the same value for
//! the whole run, so the model can follow a lead ("orders for ‹email-3›") without seeing the
//! value. `unmask_sql` puts the real values back into the model's SQL as literals of the
//! connection's dialect before it runs.
//!
//! This is best effort. A personal column under a name the model chose (`full_name AS
//! creator`) is masked by reading the SQL (`learn_aliases`), but a name inside free text in a
//! column with an ordinary name, or a value the SQL builds in a way the reading misses, is
//! caught only when the value has a recognisable shape. That is why sending rows is off by default and the "counts and
//! summaries only" setting (`summarize`) sends no values at all.

use crate::db::{Kind, QueryResult};
use crate::safety::never_send::Entry;
use crate::safety::pii::{
    is_card, is_email, is_long_id, is_phone, is_ssn, starts_with_date as is_date,
};
use crate::safety::sql_lex::{Dialect, Tok, lex};
use serde::Serialize;
use serde_json::Value;
use std::collections::{HashMap, HashSet};

mod kinds;
mod shape;

pub use kinds::{column_kind, value_kind};
use shape::*;

const OPEN: char = '‹';
const CLOSE: char = '›';

/// What one result looks like after masking: what the model may see.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MaskedResult {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Value>>,
    /// Rows the query returned; `rows` holds at most the send limit of them.
    pub row_count: usize,
    /// The query had more rows than SayQL read.
    pub truncated: bool,
    /// Columns with at least one masked value.
    pub masked_columns: Vec<String>,
}

/// The "counts and summaries only" view of a result: no row values and no group labels.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResultSummary {
    pub row_count: usize,
    pub truncated: bool,
    pub columns: Vec<ColumnSummary>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ColumnSummary {
    pub name: String,
    /// number, date, boolean, text, json, mixed, or empty (only nulls).
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub nulls: usize,
    pub distinct: usize,
    /// Smallest and largest value, for number and date columns that don't look personal.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max: Option<Value>,
}

/// Masking state for one agent run: the tokens handed out so far and the columns never to send.
#[derive(Debug, Clone, Default)]
pub struct Masker {
    /// The never-send entries (`column`, `table.column`).
    entries: Vec<Entry>,
    /// Lowercase never-send column names that apply: until a query is learned every entry's
    /// column (fail closed); then bare entries and those whose table the query names.
    never_send: HashSet<String>,
    /// What the current query's SQL says about its result columns (see `learn_aliases`).
    shape: Option<Shape>,
    /// The JSON text of a masked value → its token.
    tokens: HashMap<String, String>,
    /// Token → the real value.
    values: HashMap<String, Value>,
    /// Tokens handed out per kind, for numbering.
    counts: HashMap<&'static str, usize>,
    /// Result names the current query gives to personal columns (`c.full_name creator`).
    aliases: HashMap<String, &'static str>,
}

impl Masker {
    pub fn new<S: AsRef<str>>(never_send: impl IntoIterator<Item = S>) -> Self {
        let list: Vec<String> = never_send
            .into_iter()
            .map(|c| c.as_ref().to_string())
            .collect();
        let entries = crate::safety::never_send::entries(&list);
        Masker {
            never_send: entries.iter().map(|e| e.column.clone()).collect(),
            entries,
            ..Default::default()
        }
    }

    /// The query's result columns `positions` hold never-send values passed straight through
    /// (from the Postgres plan, see never_send.rs): masked or withheld whatever their name.
    /// Call after `learn_aliases`.
    pub fn withhold_positions(&mut self, positions: &[usize]) {
        if let Some(s) = &mut self.shape {
            s.withheld = positions.to_vec();
        }
    }

    /// Whether result column `i` of the current query is a never-send value, by its name or
    /// its position.
    pub fn is_withheld(&self, i: usize, column: &str) -> bool {
        self.is_never_send(column) || self.shape.as_ref().is_some_and(|s| s.withheld.contains(&i))
    }

    /// The first `max_rows` rows of `r`, with likely personal values replaced by tokens.
    pub fn mask(&mut self, r: &QueryResult, max_rows: usize) -> MaskedResult {
        let withheld = |i: usize| self.shape.as_ref().is_some_and(|s| s.withheld.contains(&i));
        let by_name: Vec<Option<&'static str>> = r
            .columns
            .iter()
            .enumerate()
            .map(|(i, c)| {
                self.column_rule(c)
                    .or_else(|| withheld(i).then_some("value"))
            })
            .collect();
        // SQL whose result columns SayQL can't trace to their source: every text value is masked.
        let opaque = self.shape.as_ref().is_some_and(|s| s.opaque);
        let mut masked = vec![false; r.columns.len()];
        let rows = r
            .rows
            .iter()
            .take(max_rows)
            .map(|row| {
                row.iter()
                    .enumerate()
                    .map(|(i, v)| {
                        if v.is_null() {
                            return Value::Null;
                        }
                        let by_column = by_name
                            .get(i)
                            .copied()
                            .flatten()
                            .filter(|k| !numbers_are_safe(k, v));
                        let untraced = opaque
                            && matches!(v, Value::String(_) | Value::Array(_) | Value::Object(_));
                        match by_column
                            .or_else(|| value_kind(v))
                            .or_else(|| untraced.then_some("text"))
                        {
                            Some(kind) => {
                                if let Some(m) = masked.get_mut(i) {
                                    *m = true;
                                }
                                Value::String(self.token(kind, v))
                            }
                            None => v.clone(),
                        }
                    })
                    .collect()
            })
            .collect();
        let masked_columns = r
            .columns
            .iter()
            .zip(masked)
            .filter(|(_, m)| *m)
            .map(|(c, _)| c.clone())
            .collect();
        MaskedResult {
            columns: r.columns.clone(),
            rows,
            row_count: r.rows.len(),
            truncated: r.truncated,
            masked_columns,
        }
    }

    /// Row count and per-column type, null count and distinct count; min and max only for
    /// number and date columns whose name and values don't look personal and, once a query is
    /// learned (`learn_aliases`), only for result columns that are a table's own column or a
    /// count, sum, average, min or max of one (not computed expressions, which could encode
    /// text as numbers).
    pub fn summarize(&self, r: &QueryResult) -> ResultSummary {
        let eligible = self.shape.as_ref().map(|s| s.min_max_columns(&r.columns));
        let columns = r
            .columns
            .iter()
            .enumerate()
            .map(|(i, name)| {
                let values: Vec<&Value> = r
                    .rows
                    .iter()
                    .filter_map(|row| row.get(i))
                    .filter(|v| !v.is_null())
                    .collect();
                let distinct = values
                    .iter()
                    .map(|v| v.to_string())
                    .collect::<HashSet<_>>()
                    .len();
                let kind = column_type(&values);
                let by_column = self
                    .column_rule(name)
                    .filter(|k| !values.iter().all(|v| numbers_are_safe(k, v)));
                let traced = eligible
                    .as_ref()
                    .is_none_or(|e| e.get(i).copied().unwrap_or(false));
                let personal = by_column.is_some()
                    || self.is_withheld(i, name)
                    || !traced
                    || values.iter().any(|v| value_kind(v).is_some());
                let (min, max) = match kind {
                    "number" | "date" if !personal => min_max(&values, kind),
                    _ => (None, None),
                };
                ColumnSummary {
                    name: name.clone(),
                    kind,
                    nulls: r.rows.len() - values.len(),
                    distinct,
                    min,
                    max,
                }
            })
            .collect();
        ResultSummary {
            row_count: r.rows.len(),
            truncated: r.truncated,
            columns,
        }
    }

    /// `sql` with every token replaced by its real value, written as a literal of `kind`'s
    /// dialect (or, inside a string the model wrote, escaped for that string). Refuses tokens
    /// this run didn't hand out, tokens in identifiers, and SQL where the value's place
    /// depends on server settings (backslashes in MySQL and Postgres strings).
    pub fn unmask_sql(&self, sql: &str, kind: Kind) -> Result<String, String> {
        if !sql.contains(OPEN) {
            return Ok(sql.to_string());
        }
        // Whether `\` escapes a quote depends on server settings (MySQL's NO_BACKSLASH_ESCAPES,
        // Postgres' standard_conforming_strings), so the values must land the same way both ways.
        match kind {
            Kind::Mysql | Kind::Postgres => {
                let a = self.fill(sql, kind, true)?;
                if a != self.fill(sql, kind, false)? {
                    return Err(AMBIGUOUS.into());
                }
                Ok(a)
            }
            Kind::Sqlite | Kind::Mssql => self.fill(sql, kind, false),
        }
    }

    /// `text` with tokens replaced by the real values, for the person's screen. Unknown
    /// tokens stay as they are.
    pub fn unmask_text(&self, text: &str) -> String {
        let mut out = String::with_capacity(text.len());
        let mut rest = text;
        while let Some(start) = rest.find(OPEN) {
            out.push_str(&rest[..start]);
            let tail = &rest[start..];
            match token_at(tail).and_then(|t| self.values.get(t).map(|v| (t, v))) {
                Some((t, v)) => {
                    out.push_str(&plain(v));
                    rest = &tail[t.len()..];
                }
                None => {
                    out.push(OPEN);
                    rest = &tail[OPEN.len_utf8()..];
                }
            }
        }
        out.push_str(rest);
        out
    }

    /// Reads the SQL about to run for result names it gives to expressions that use a personal
    /// or never-send column (`c.full_name creator`, `upper(email) AS e`, through CTEs and
    /// subqueries too), so those result columns are masked like the column itself. Counts of
    /// a personal column (`COUNT(email) AS n`) aren't personal.
    ///
    /// It also reads which never-send entries apply (bare ones, and `table.column` ones whose
    /// table the query names), whether the SQL moves values in ways this reading can't follow
    /// (then `mask` masks every text value), and which result columns are a table's own column
    /// or an aggregate of one (only those get min and max in `summarize`).
    pub fn learn_aliases(&mut self, sql: &str) {
        self.aliases.clear();
        self.never_send = crate::safety::never_send::active_columns(sql, &self.entries);
        self.shape = None;
        self.learn_names(sql);
        let shape = Shape::of(sql, &|w: &str| self.column_rule(w).is_some());
        self.shape = Some(shape);
    }

    fn learn_names(&mut self, sql: &str) {
        let toks: Vec<String> = crate::ai::agent::checks::tokens(sql)
            .iter()
            .map(|t| t.to_lowercase())
            .collect();
        let mut items: Vec<Vec<String>> = Vec::new();
        let mut i = 0;
        while i < toks.len() {
            if toks[i] != "select" {
                i += 1;
                continue;
            }
            i += 1;
            let (mut depth, mut cur) = (0, Vec::new());
            while i < toks.len() {
                let t = &toks[i];
                if t == "(" {
                    depth += 1;
                } else if t == ")" {
                    if depth == 0 {
                        break;
                    }
                    depth -= 1;
                } else if depth == 0
                    && [
                        "from", "where", "union", "into", "group", "order", "having", "limit",
                    ]
                    .contains(&t.as_str())
                {
                    break;
                }
                if depth == 0 && t == "," {
                    items.push(std::mem::take(&mut cur));
                } else {
                    cur.push(t.clone());
                }
                i += 1;
            }
            items.push(cur);
        }
        let ident =
            |t: &str| t.starts_with(|c: char| c.is_alphabetic() || c == '_' || "\"`[".contains(c));
        let bare = |t: &str| -> String {
            let t: String = t.chars().filter(|c| !"\"`[]".contains(*c)).collect();
            t.rsplit('.').next().unwrap_or("").to_string()
        };
        loop {
            let mut changed = false;
            for item in &items {
                let n = item.len();
                let (alias, expr) = if n >= 3 && item[n - 2] == "as" {
                    (bare(&item[n - 1]), &item[..n - 2])
                } else if n >= 2
                    && ident(&item[n - 1])
                    && !["end", "null", "true", "false"].contains(&item[n - 1].as_str())
                    && (ident(&item[n - 2]) || item[n - 2] == ")")
                    && !["as", "distinct", "case", "else", "then"].contains(&item[n - 2].as_str())
                {
                    (bare(&item[n - 1]), &item[..n - 1])
                } else {
                    continue;
                };
                // A count, sum or average of a personal column is a number about it, not the column's values.
                let wrappers = [
                    "(", "select", "coalesce", "round", "cast", "abs", "floor", "ceil", "ceiling",
                    "trunc", "nullif",
                ];
                let lead = expr.iter().find(|t| !wrappers.contains(&t.as_str()));
                let aggregate = lead.is_some_and(|f| ["count", "sum", "avg"].contains(&f.as_str()));
                if alias.is_empty() || self.aliases.contains_key(&alias) || aggregate {
                    continue;
                }
                let kind = expr
                    .iter()
                    .filter(|t| ident(t))
                    .map(|t| bare(t))
                    .find_map(|w| self.column_rule(&w));
                if let Some(kind) = kind {
                    self.aliases.insert(alias, kind);
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
    }

    /// Whether values of `column` are always masked (its name, or the never-send list).
    pub fn is_personal_column(&self, column: &str) -> bool {
        self.column_rule(column).is_some()
    }

    /// Whether `column` (a result column's name) is on the never-send list.
    pub fn is_never_send(&self, column: &str) -> bool {
        self.never_send.contains(&column.trim().to_lowercase())
    }

    /// `text` with every real value this run masked replaced by its token again: for
    /// database messages and plans that quote values from the SQL.
    pub fn remask_text(&self, text: &str) -> String {
        let mut pairs: Vec<(String, &String)> = self
            .values
            .iter()
            .map(|(t, v)| (plain(v), t))
            .filter(|(v, _)| v.chars().count() >= 3)
            .collect();
        pairs.sort_by_key(|(v, _)| std::cmp::Reverse(v.len()));
        let mut out = text.to_string();
        for (v, t) in pairs {
            out = out.replace(&v, t);
        }
        out
    }

    /// A token for a value the person may see but the AI may not, whatever its column or shape.
    pub fn token_for(&mut self, column: &str, v: &Value) -> String {
        let kind = self
            .column_rule(column)
            .or_else(|| value_kind(v))
            .unwrap_or("value");
        self.token(kind, v)
    }

    /// Masks one value as it would be masked in a column called `column`.
    pub fn mask_value(&mut self, column: &str, v: &Value) -> Value {
        if v.is_null() {
            return Value::Null;
        }
        match self.column_rule(column).or_else(|| value_kind(v)) {
            Some(kind) => Value::String(self.token(kind, v)),
            None => v.clone(),
        }
    }

    /// The kind to mask a whole column as, from its name or the never-send list.
    fn column_rule(&self, column: &str) -> Option<&'static str> {
        let name = column.trim().to_lowercase();
        let kind = column_kind(column).or_else(|| self.aliases.get(&name).copied());
        if self.never_send.contains(&name) {
            Some(kind.unwrap_or("value"))
        } else {
            kind
        }
    }

    fn token(&mut self, kind: &'static str, v: &Value) -> String {
        let key = v.to_string();
        if let Some(t) = self.tokens.get(&key) {
            return t.clone();
        }
        let n = self.counts.entry(kind).or_default();
        *n += 1;
        let token = format!("{OPEN}{kind}-{n}{CLOSE}");
        self.tokens.insert(key, token.clone());
        self.values.insert(token.clone(), v.clone());
        token
    }

    /// One pass of `unmask_sql`, reading plain `'...'` strings with or without `\` escapes.
    fn fill(&self, sql: &str, kind: Kind, backslash: bool) -> Result<String, String> {
        let (mysql, postgres) = (kind == Kind::Mysql, kind == Kind::Postgres);
        let mut out = String::with_capacity(sql.len() + 32);
        let mut state = State::Code;
        let mut i = 0;
        while i < sql.len() {
            let rest = &sql[i..];
            if let Some(token) = token_at(rest) {
                let value = self.values.get(token).ok_or_else(|| {
                    format!("{token} isn't a value SayQL masked in this run. Use only tokens from earlier results.")
                })?;
                match &mut state {
                    State::Code => out.push_str(&literal(value, kind)?),
                    State::Str {
                        quote,
                        escapes,
                        start,
                        national,
                    } => {
                        let text = in_string(value, *quote, *escapes);
                        // SQL Server keeps non-ASCII text only in N'...' strings.
                        if kind == Kind::Mssql && !*national && !text.is_ascii() {
                            out.insert(*start, 'N');
                            *national = true;
                        }
                        out.push_str(&text);
                    }
                    State::Dollar(_) => {
                        return Err(format!(
                            "Put {token} in a single-quoted string, not a $$ string."
                        ));
                    }
                    State::Comment(_) => out.push_str(token),
                }
                i += token.len();
                continue;
            }
            let mut chars = rest.chars();
            let c = chars.next().unwrap_or_default();
            let next = chars.next();
            let mut len = c.len_utf8();
            let mut next_state = None;
            match &state {
                State::Code => match c {
                    '\'' => {
                        let before: Vec<char> = sql[..i].chars().rev().take(2).collect();
                        let prefixed = |p: char| {
                            before.first().is_some_and(|b| b.eq_ignore_ascii_case(&p))
                                && before.get(1).is_none_or(|b| {
                                    !(b.is_alphanumeric() || *b == '_' || *b == '$')
                                })
                        };
                        // Postgres E'...' strings always take backslash escapes.
                        let escapes =
                            (postgres && prefixed('e')) || ((mysql || postgres) && backslash);
                        let national = kind == Kind::Mssql && prefixed('n');
                        next_state = Some(State::Str {
                            quote: '\'',
                            escapes,
                            start: out.len(),
                            national,
                        });
                    }
                    '"' if mysql => {
                        next_state = Some(State::Str {
                            quote: '"',
                            escapes: backslash,
                            start: out.len(),
                            national: false,
                        })
                    }
                    '"' => len += quoted_name(&rest[1..], '"')?,
                    '`' if mysql || kind == Kind::Sqlite => len += quoted_name(&rest[1..], '`')?,
                    '[' if !mysql && !postgres => len += quoted_name(&rest[1..], ']')?,
                    '$' if postgres => {
                        if let Some(tag) = dollar_tag(rest) {
                            len = tag.len();
                            next_state = Some(State::Dollar(tag.to_string()));
                        }
                    }
                    '-' if next == Some('-')
                        && (!mysql || rest[2..].chars().next().is_none_or(char::is_whitespace)) =>
                    {
                        next_state = Some(State::Comment("\n"));
                    }
                    '#' if mysql => next_state = Some(State::Comment("\n")),
                    // MySQL runs the inside of `/*! ... */` as SQL, so tokens there are code.
                    '/' if next == Some('*')
                        && !(mysql
                            && (rest[2..].starts_with('!') || rest[2..].starts_with("M!"))) =>
                    {
                        len = 2;
                        next_state = Some(State::Comment("*/"));
                    }
                    _ => {}
                },
                State::Str { quote, escapes, .. } => {
                    if c == '\\' && *escapes {
                        len += next.map_or(0, char::len_utf8);
                    } else if c == *quote && next == Some(*quote) {
                        len += 1;
                    } else if c == *quote {
                        next_state = Some(State::Code);
                    }
                }
                State::Dollar(tag) if rest.starts_with(tag.as_str()) => {
                    len = tag.len();
                    next_state = Some(State::Code);
                }
                State::Comment(end) if rest.starts_with(end) => {
                    len = end.len();
                    next_state = Some(State::Code);
                }
                State::Dollar(_) | State::Comment(_) => {}
            }
            if let Some(s) = next_state {
                state = s;
            }
            out.push_str(&sql[i..i + len]);
            i += len;
        }
        Ok(out)
    }
}

const AMBIGUOUS: &str = "SayQL couldn't place a masked value safely in this SQL, because a string in it uses a \
backslash. Write the query without backslashes in strings.";

enum State {
    Code,
    /// Inside a string literal closed by `quote`. `escapes`: `\` escapes the next character;
    /// `start`: where its opening quote is in the output; `national`: SQL Server N'...'.
    Str {
        quote: char,
        escapes: bool,
        start: usize,
        national: bool,
    },
    /// Postgres $tag$ ... $tag$.
    Dollar(String),
    /// A comment, until this text.
    Comment(&'static str),
}

/// The length of a quoted name after its opening quote (`body`), up to and including
/// `close`; a doubled `close` is a literal one. A token inside is refused.
fn quoted_name(body: &str, close: char) -> Result<usize, String> {
    let mut j = 0;
    let end = loop {
        match body[j..].find(close) {
            Some(p) if body[j + p + 1..].starts_with(close) => j += p + 2,
            Some(p) => break j + p + 1,
            None => break body.len(),
        }
    };
    match body[..end].find(OPEN).and_then(|p| token_at(&body[p..])) {
        Some(t) => Err(format!(
            "{t} is a value, not a name; put it in single quotes."
        )),
        None => Ok(end),
    }
}

/// `$$` or `$tag$` at the start of `s` (not `$1`).
fn dollar_tag(s: &str) -> Option<&str> {
    let body = &s[1..];
    if body.starts_with(|c: char| c.is_ascii_digit()) {
        return None;
    }
    let end = body.find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))?;
    body[end..].starts_with('$').then(|| &s[..end + 2])
}

/// The token at the start of `s`: `‹kind-n›`.
fn token_at(s: &str) -> Option<&str> {
    let body = s.strip_prefix(OPEN)?;
    let end = body.find(CLOSE)?;
    let (kind, n) = body[..end].rsplit_once('-')?;
    let ok = !kind.is_empty()
        && kind.chars().all(|c| c.is_ascii_lowercase())
        && !n.is_empty()
        && n.chars().all(|c| c.is_ascii_digit());
    ok.then(|| &s[..OPEN.len_utf8() + end + CLOSE.len_utf8()])
}

/// A number in a column that is personal only by its name ("name", "notes") is a count or an
/// id the model computed (`names_per_worker`), never a name or free text. Phone numbers,
/// card numbers and other identifiers stay masked: their kinds hold numbers.
fn numbers_are_safe(kind: &str, v: &Value) -> bool {
    matches!(kind, "name" | "text") && v.is_number()
}

/// The value as text, as the person would read it.
fn plain(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// The value as a SQL literal of `kind`'s dialect.
pub(crate) fn literal(v: &Value, kind: Kind) -> Result<String, String> {
    match v {
        Value::Number(n) => return Ok(n.to_string()),
        Value::Bool(b) if kind == Kind::Mssql => return Ok(if *b { "1" } else { "0" }.into()),
        Value::Bool(b) => return Ok(if *b { "TRUE" } else { "FALSE" }.into()),
        _ => {}
    }
    let s = plain(v);
    let quoted = s.replace('\'', "''");
    Ok(match kind {
        // An E'' string reads the same whatever standard_conforming_strings is.
        Kind::Postgres if s.contains('\\') => format!("E'{}'", quoted.replace('\\', "\\\\")),
        Kind::Postgres | Kind::Sqlite => format!("'{quoted}'"),
        // Whether MySQL reads `\` as an escape depends on the server's sql_mode.
        Kind::Mysql if s.contains('\\') => return Err(AMBIGUOUS.into()),
        Kind::Mysql => format!("'{quoted}'"),
        Kind::Mssql => format!("N'{quoted}'"),
    })
}

/// The value escaped to sit inside a string literal the model wrote.
fn in_string(v: &Value, quote: char, escapes: bool) -> String {
    let mut s = plain(v);
    if escapes {
        s = s.replace('\\', "\\\\");
    }
    s.replace(quote, &format!("{quote}{quote}"))
}

fn as_number(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        // Exact decimals arrive as text.
        Value::String(s) => {
            let t = s.strip_prefix('-').unwrap_or(s);
            let ok = !t.is_empty()
                && t.chars().all(|c| c.is_ascii_digit() || c == '.')
                && t.matches('.').count() <= 1
                && !t.starts_with('.')
                && !t.ends_with('.');
            if ok { s.parse().ok() } else { None }
        }
        _ => None,
    }
}

fn column_type(values: &[&Value]) -> &'static str {
    let all = |f: &dyn Fn(&Value) -> bool| values.iter().all(|v| f(v));
    if values.is_empty() {
        "empty"
    } else if all(&|v| as_number(v).is_some()) {
        "number"
    } else if all(&|v| v.as_str().is_some_and(is_date)) {
        "date"
    } else if all(&Value::is_boolean) {
        "boolean"
    } else if all(&Value::is_string) {
        "text"
    } else if all(&|v| v.is_object() || v.is_array()) {
        "json"
    } else {
        "mixed"
    }
}

fn min_max(values: &[&Value], kind: &str) -> (Option<Value>, Option<Value>) {
    let mut sorted: Vec<&Value> = values.to_vec();
    if kind == "number" {
        sorted.sort_by(|a, b| {
            as_number(a)
                .partial_cmp(&as_number(b))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
    } else {
        sorted.sort_by_key(|v| v.as_str().unwrap_or(""));
    }
    (
        sorted.first().map(|v| (*v).clone()),
        sorted.last().map(|v| (*v).clone()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn result(columns: &[&str], rows: Vec<Vec<Value>>) -> QueryResult {
        QueryResult {
            columns: columns.iter().map(|c| c.to_string()).collect(),
            rows,
            rows_affected: None,
            truncated: false,
            elapsed_ms: 1,
            withheld: vec![],
        }
    }

    /// Planted personal data, each under a column name that gives it away and again under
    /// one that doesn't, so only its shape can catch it.
    const PLANTED: &[(&str, &str)] = &[
        ("email", "dana.reyes@example.org"),
        ("phone_number", "+1 (555) 014-2233"),
        ("full_name", "Dana Reyes"),
        ("home_address", "12 Harbour Lane, Portsmouth"),
        ("date_of_birth", "1984-03-09"),
        ("ssn", "123-45-6789"),
        ("mrn", "MRN-00042"),
        ("patient_id", "P-77812"),
        ("api_token", "sk_live_9f8e7d6c5b4a3210"),
        ("password_hash", "$2b$12$abcdefghijklmnopqrstuv"),
        ("notes", "Prefers mornings; daughter is Maya"),
        ("PatientFirstName", "Marisol"),
        ("cardNumber", "4111 1111 1111 1111"),
        ("ip_address", "203.0.113.7"),
        ("name", "Kwame Mensah"),
        ("username", "kmensah"),
        ("dob", "1990-12-01"),
        ("zip", "02139"),
    ];
    fn shaped() -> Vec<(&'static str, Value)> {
        vec![
            ("contact", json!("ali.khan@example.com")),
            ("c1", json!("+44 20 7946 0958")),
            ("c2", json!("555-867-5309")),
            ("c3", json!("5558675309")),
            ("ref", json!("4111-1111-1111-1111")),
            ("ref2", json!(4012888888881881_u64)),
            ("x", json!("078-05-1120")),
            (
                "details",
                json!("Call me on +1 555 014 9876 or mail sam@example.net"),
            ),
            ("details2", json!("card 5500 0000 0000 0004 expired")),
            ("ext_id", json!("550e8400-e29b-41d4-a716-446655440000")),
            ("meta", json!({ "name": "Ines Duarte" })),
            ("addr_v6", json!("2001:db8::1")),
        ]
    }

    fn all_planted() -> (QueryResult, Vec<String>) {
        let mut columns: Vec<&str> = PLANTED.iter().map(|(c, _)| *c).collect();
        let mut row: Vec<Value> = PLANTED.iter().map(|(_, v)| json!(v)).collect();
        let shaped = shaped();
        columns.extend(shaped.iter().map(|(c, _)| *c));
        row.extend(shaped.iter().map(|(_, v)| v.clone()));
        let mut secrets: Vec<String> = row.iter().map(plain).collect();
        // Parts that would identify someone on their own.
        secrets.extend(
            [
                "Dana",
                "Reyes",
                "Maya",
                "Ines Duarte",
                "sam@example.net",
                "555 014 9876",
                "5500 0000",
            ]
            .map(String::from),
        );
        (result(&columns, vec![row.clone(), row]), secrets)
    }

    #[test]
    fn no_planted_value_survives_masking() {
        let (r, secrets) = all_planted();
        let mut m = Masker::new(Vec::<String>::new());
        let sent = serde_json::to_string(&m.mask(&r, 20)).unwrap();
        for s in &secrets {
            assert!(
                !sent.contains(s.as_str()),
                "{s:?} reached the payload: {sent}"
            );
        }
        let summary = serde_json::to_string(&m.summarize(&r)).unwrap();
        for s in &secrets {
            assert!(
                !summary.contains(s.as_str()),
                "{s:?} reached the summary: {summary}"
            );
        }
    }

    #[test]
    fn ordinary_values_are_kept() {
        let r = result(
            &[
                "id",
                "status",
                "country",
                "city",
                "total",
                "amount",
                "ordered_at",
                "product_name",
                "n",
                "last_login_at",
                "is_active",
                "category",
            ],
            vec![vec![
                json!(42),
                json!("paid"),
                json!("UK"),
                json!("London"),
                json!("1234567.89"),
                json!(19.99),
                json!("2026-10-23 11:32:00"),
                json!("Smart Lamp 3"),
                json!(3000),
                json!("2026-10-23T11:32:00.000Z"),
                json!(true),
                json!("Home & Kitchen"),
            ]],
        );
        let masked = Masker::new(Vec::<String>::new()).mask(&r, 20);
        assert_eq!(masked.rows, r.rows);
        assert!(masked.masked_columns.is_empty());
    }

    #[test]
    fn tokens_are_stable_for_the_run() {
        let mut m = Masker::new(Vec::<String>::new());
        let r = result(
            &["email"],
            vec![
                vec![json!("a@example.com")],
                vec![json!("b@example.com")],
                vec![json!("a@example.com")],
                vec![Value::Null],
            ],
        );
        let out = m.mask(&r, 20);
        assert_eq!(
            out.rows,
            [
                [json!("‹email-1›")],
                [json!("‹email-2›")],
                [json!("‹email-1›")],
                [Value::Null]
            ]
        );
        // The same value in a later step, under another column, keeps its token.
        let again = m.mask(
            &result(&["created_by"], vec![vec![json!("b@example.com")]]),
            20,
        );
        assert_eq!(again.rows, [[json!("‹email-2›")]]);
        assert_eq!(out.masked_columns, ["email"]);
    }

    #[test]
    fn send_limit_keeps_the_row_count() {
        let rows = (0..50).map(|i| vec![json!(i)]).collect();
        let out = Masker::new(Vec::<String>::new()).mask(&result(&["n"], rows), 20);
        assert_eq!((out.rows.len(), out.row_count), (20, 50));
    }

    #[test]
    fn never_send_columns_are_masked() {
        let mut m = Masker::new(["patients.diagnosis", "Severity"]);
        let r = result(
            &["diagnosis", "severity", "visits"],
            vec![vec![json!("asthma"), json!(3), json!(2)]],
        );
        let out = m.mask(&r, 20);
        assert_eq!(
            out.rows,
            [[json!("‹value-1›"), json!("‹value-2›"), json!(2)]]
        );
        let summary = m.summarize(&r);
        assert!(summary.columns[1].min.is_none() && summary.columns[2].min == Some(json!(2)));
    }

    #[test]
    fn column_names_that_look_personal() {
        for c in [
            "email",
            "EmailAddress",
            "contact_email",
            "phone",
            "mobile_no",
            "Telephone",
            "first_name",
            "LastName",
            "full_name",
            "name",
            "patient_name",
            "username",
            "home_address",
            "street",
            "zip_code",
            "postcode",
            "date_of_birth",
            "DOB",
            "birthDate",
            "ssn",
            "SocialSecurityNumber",
            "national_id",
            "passport_no",
            "mrn",
            "medical_record_number",
            "patient_id",
            "PatientId",
            "api_token",
            "password",
            "password_hash",
            "reset_token",
            "secret",
            "api_key",
            "ip_address",
            "notes",
            "comment",
            "card_number",
            "iban",
            "insurance_member_id",
            "latitude",
            "login",
            "login_name",
        ] {
            assert!(column_kind(c).is_some(), "{c} should be masked");
        }
        for c in [
            "id",
            "status",
            "country",
            "city",
            "state",
            "created_at",
            "last_login_at",
            "first_seen",
            "product_name",
            "category_name",
            "tenant_name",
            "role",
            "amount",
            "total",
            "quantity",
            "ordered_at",
            "plan",
            "tenant_id",
            "user_id",
            "rating",
            "body",
            "kind",
            "action",
            "domain_name",
            "is_active",
            "membership_count",
            "description",
            "code",
            "label",
            "statuses",
            "login_count",
        ] {
            assert!(
                column_kind(c).is_none(),
                "{c} should not be masked ({:?})",
                column_kind(c)
            );
        }
        assert_eq!(column_kind("EmailAddress"), Some("email"));
        assert_eq!(column_kind("ip_address"), Some("ip"));
        assert_eq!(column_kind("PatientId"), Some("patient"));
    }

    #[test]
    fn value_shapes() {
        let yes = [
            ("dana@example.org", "email"),
            ("o'neil+tag@mail.example.co.uk", "email"),
            ("123-45-6789", "ssn"),
            ("4111111111111111", "card"),
            ("+1 (555) 014-2233", "phone"),
            ("(555) 014-2233", "phone"),
            ("+353 1 234 5678", "phone"),
            ("10.0.0.12", "ip"),
            ("550e8400-e29b-41d4-a716-446655440000", "id"),
            (
                "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.abc123",
                "id",
            ),
            ("met at 9am, email jo@example.com", "text"),
        ];
        for (v, kind) in yes {
            assert_eq!(value_kind(&json!(v)), Some(kind), "{v}");
        }
        for v in [
            "paid",
            "2026-10-23",
            "2026-10-23 11:32:00",
            "2026-10-23T11:32:00.000+02:00",
            "1234567.89",
            "19.99",
            "2019-2024",
            "4111111111111112",
            "Smart Lamp 3",
            "11:32:00",
            "orders in 2026: 3000",
            "-12.5",
            "0.000001",
            "ORD-1042",
            "Great value, would buy again.",
        ] {
            assert_eq!(value_kind(&json!(v)), None, "{v}");
        }
        assert_eq!(value_kind(&json!(4111111111111111_u64)), Some("card"));
        assert_eq!(value_kind(&json!(15551234567_u64)), Some("phone"));
        for v in [
            json!(42),
            json!(2026),
            json!(19.99),
            json!(true),
            json!(3000000),
        ] {
            assert_eq!(value_kind(&v), None, "{v}");
        }
    }

    #[test]
    fn summary_has_no_values_or_labels() {
        let r = result(
            &["country", "day", "orders", "revenue", "email", "ok"],
            vec![
                vec![
                    json!("UK"),
                    json!("2026-10-20"),
                    json!(12),
                    json!("120.50"),
                    json!("a@example.com"),
                    json!(true),
                ],
                vec![
                    json!("India"),
                    json!("2026-10-21"),
                    json!(30),
                    json!("99.00"),
                    json!("b@example.com"),
                    json!(false),
                ],
                vec![
                    json!("UK"),
                    json!("2026-10-22"),
                    Value::Null,
                    json!("1000.25"),
                    json!("a@example.com"),
                    json!(true),
                ],
            ],
        );
        let s = Masker::new(Vec::<String>::new()).summarize(&r);
        let text = serde_json::to_string(&s).unwrap();
        for hidden in ["UK", "India", "@example.com"] {
            assert!(!text.contains(hidden), "{hidden} in {text}");
        }
        let col = |n: &str| s.columns.iter().find(|c| c.name == n).unwrap();
        assert_eq!(
            (s.row_count, col("country").kind, col("country").distinct),
            (3, "text", 2)
        );
        assert_eq!(
            (
                col("day").kind,
                col("day").min.clone(),
                col("day").max.clone()
            ),
            ("date", Some(json!("2026-10-20")), Some(json!("2026-10-22")))
        );
        assert_eq!(
            (
                col("orders").nulls,
                col("orders").min.clone(),
                col("orders").max.clone()
            ),
            (1, Some(json!(12)), Some(json!(30)))
        );
        assert_eq!(
            (col("revenue").min.clone(), col("revenue").max.clone()),
            (Some(json!("99.00")), Some(json!("1000.25")))
        );
        assert_eq!(
            (
                col("email").kind,
                col("email").distinct,
                col("email").min.clone()
            ),
            ("text", 2, None)
        );
        assert_eq!(col("ok").kind, "boolean");
        assert!(
            text.contains("\"type\":\"number\"") && text.contains("rowCount"),
            "{text}"
        );
    }

    #[test]
    fn birth_dates_get_no_min_or_max() {
        let r = result(
            &["date_of_birth", "n"],
            vec![vec![json!("1984-03-09"), json!(1)]],
        );
        let s = Masker::new(Vec::<String>::new()).summarize(&r);
        assert_eq!(
            (s.columns[0].kind, s.columns[0].min.clone()),
            ("date", None)
        );
    }

    fn masker_with(values: &[Value]) -> (Masker, Vec<String>) {
        let mut m = Masker::new(["v"]);
        let rows = values.iter().map(|v| vec![v.clone()]).collect();
        let out = m.mask(&result(&["v"], rows), 100);
        let tokens = out
            .rows
            .into_iter()
            .map(|r| r[0].as_str().unwrap().to_string())
            .collect();
        (m, tokens)
    }

    #[test]
    fn tokens_become_dialect_literals() {
        let (m, t) = masker_with(&[json!("O'Brien"), json!(42), json!("a\\b"), json!("Zoë")]);
        let sql = format!(
            "SELECT * FROM t WHERE a = {} AND b = '{}' AND c = {} AND d LIKE '%{}%'",
            t[0], t[0], t[1], t[0]
        );
        assert_eq!(
            m.unmask_sql(&sql, Kind::Sqlite).unwrap(),
            "SELECT * FROM t WHERE a = 'O''Brien' AND b = 'O''Brien' AND c = 42 AND d LIKE '%O''Brien%'"
        );
        assert_eq!(
            m.unmask_sql(&format!("SELECT {}", t[3]), Kind::Mssql)
                .unwrap(),
            "SELECT N'Zoë'"
        );
        assert_eq!(
            m.unmask_sql(&format!("SELECT {}", t[0]), Kind::Mysql)
                .unwrap(),
            "SELECT 'O''Brien'"
        );
        // Postgres: an E'' string means the same whatever standard_conforming_strings is.
        assert_eq!(
            m.unmask_sql(&format!("SELECT {}", t[2]), Kind::Postgres)
                .unwrap(),
            "SELECT E'a\\\\b'"
        );
        assert_eq!(
            m.unmask_sql(&format!("SELECT E'x{}'", t[2]), Kind::Postgres)
                .unwrap(),
            "SELECT E'xa\\\\b'"
        );
        // MySQL's reading of `\` depends on sql_mode, so it refuses rather than guess.
        assert_eq!(
            m.unmask_sql(&format!("SELECT {}", t[2]), Kind::Mysql)
                .unwrap_err(),
            AMBIGUOUS
        );
        assert_eq!(
            m.unmask_sql(&format!("SELECT '{}'", t[2]), Kind::Postgres)
                .unwrap_err(),
            AMBIGUOUS
        );
        // A backslash elsewhere changes where strings end on some servers.
        assert_eq!(
            m.unmask_sql(&format!("SELECT 'a\\' = {}", t[0]), Kind::Mysql)
                .unwrap_err(),
            AMBIGUOUS
        );
        assert_eq!(
            m.unmask_sql(&format!("SELECT 'a\\' = {}", t[0]), Kind::Sqlite)
                .unwrap(),
            "SELECT 'a\\' = 'O''Brien'"
        );
        // MySQL double-quoted strings.
        let (m2, t2) = masker_with(&[json!("say \"hi\"")]);
        assert_eq!(
            m2.unmask_sql(&format!("SELECT \"{}\"", t2[0]), Kind::Mysql)
                .unwrap(),
            "SELECT \"say \"\"hi\"\"\""
        );
    }

    #[test]
    fn unmasking_refuses_what_it_cannot_place() {
        let (m, t) = masker_with(&[json!("x@example.com")]);
        assert!(
            m.unmask_sql("SELECT '‹email-9›'", Kind::Sqlite)
                .unwrap_err()
                .contains("isn't a value SayQL masked")
        );
        for (sql, kind) in [
            (format!("SELECT \"{}\" FROM t", t[0]), Kind::Postgres),
            (format!("SELECT [{}] FROM t", t[0]), Kind::Mssql),
            (format!("SELECT `{}` FROM t", t[0]), Kind::Mysql),
        ] {
            assert!(
                m.unmask_sql(&sql, kind).unwrap_err().contains("not a name"),
                "{sql}"
            );
        }
        assert!(
            m.unmask_sql(&format!("SELECT $${}$$", t[0]), Kind::Postgres)
                .unwrap_err()
                .contains("$$")
        );
        // Comments are left alone, and SQL without tokens is untouched.
        let c = format!("SELECT 1 -- {}\n", t[0]);
        assert_eq!(m.unmask_sql(&c, Kind::Postgres).unwrap(), c);
        assert_eq!(
            m.unmask_sql("SELECT 'it''s'", Kind::Mssql).unwrap(),
            "SELECT 'it''s'"
        );
        // A quoted name with a doubled quote doesn't hide a token after it.
        let q = format!("SELECT \"a\"\"b\" FROM t WHERE x = {}", t[0]);
        assert_eq!(
            m.unmask_sql(&q, Kind::Postgres).unwrap(),
            "SELECT \"a\"\"b\" FROM t WHERE x = 'x@example.com'"
        );
    }

    #[test]
    fn renamed_personal_columns_stay_masked() {
        let mut m = Masker::new(["patients.notes"]);
        // What Sonnet 5.5 wrote in the first evaluation: the creator's name under another name.
        m.learn_aliases(
            "select t.id, c.full_name creator, m.role, u.full_name, u.email from tenants t left join users c on c.id = t.created_by_user_id \
             left join memberships m on m.tenant_id = t.id left join users u on u.id = m.user_id where t.name like '%North%'",
        );
        let r = result(
            &["id", "creator", "role", "full_name", "email"],
            vec![vec![
                json!(2),
                json!("Grace Holloway"),
                json!("admin"),
                json!("Ravi Menon"),
                json!("r@example.com"),
            ]],
        );
        let out = serde_json::to_string(&m.mask(&r, 20)).unwrap();
        assert!(
            !out.contains("Grace Holloway") && !out.contains("Ravi Menon") && out.contains("admin"),
            "{out}"
        );
        for (sql, col) in [
            (
                "SELECT upper(first_name) || ' ' || last_name AS who FROM patients",
                "who",
            ),
            (
                "SELECT \"u\".\"email\" AS \"Contact\" FROM users u",
                "contact",
            ),
            (
                "WITH x AS (SELECT phone AS p FROM users) SELECT p AS reach FROM x",
                "reach",
            ),
            ("SELECT n FROM (SELECT notes n FROM patients) s", "n"),
            (
                "SELECT CASE WHEN id > 1 THEN full_name ELSE 'x' END AS label FROM users",
                "label",
            ),
        ] {
            m.learn_aliases(sql);
            assert!(m.is_personal_column(col), "{sql}: {col} should be masked");
        }
        m.learn_aliases(
            "SELECT COUNT(email) AS n, COUNT(DISTINCT phone) people, status s FROM users",
        );
        assert!(
            !m.is_personal_column("n")
                && !m.is_personal_column("people")
                && !m.is_personal_column("s")
        );
        m.learn_aliases("SELECT COALESCE(SUM(CASE WHEN first_name = 'x' THEN 1 END), 0) AS a1, (SELECT COUNT(*) FROM users WHERE full_name LIKE 'A%') a2, MAX(full_name) AS top FROM users");
        assert!(
            !m.is_personal_column("a1")
                && !m.is_personal_column("a2")
                && m.is_personal_column("top")
        );
        // Numbers under a name-like column name are counts, not names; identifiers stay masked.
        let r = result(
            &["max_per_name", "distinct_names", "first_name", "phone"],
            vec![vec![
                json!(3),
                json!(810),
                json!("Dana"),
                json!(5550142233_u64),
            ]],
        );
        let out = m.mask(&r, 20);
        assert_eq!(out.rows[0][..2], [json!(3), json!(810)]);
        assert!(
            out.rows[0][2].as_str().unwrap().starts_with('‹')
                && out.rows[0][3].as_str().unwrap().starts_with('‹'),
            "{:?}",
            out.rows
        );
        m.learn_aliases("SELECT count(*) AS max_per_name, count(DISTINCT full_name) AS distinct_names, first_name, phone FROM users GROUP BY first_name, phone");
        let s = m.summarize(&r);
        assert_eq!(
            (s.columns[0].min.clone(), s.columns[1].max.clone()),
            (Some(json!(3)), Some(json!(810)))
        );
        // Each query starts fresh.
        m.learn_aliases("SELECT 1 AS who");
        assert!(!m.is_personal_column("who"));
    }

    #[test]
    fn values_are_masked_again_in_text() {
        let (mut m, t) = masker_with(&[json!("dana@example.org")]);
        let msg = "duplicate key: email=(dana@example.org) for ab";
        assert_eq!(
            m.remask_text(msg),
            format!("duplicate key: email=({}) for ab", t[0])
        );
        assert_eq!(m.mask_value("phone", &json!("x")), json!("‹phone-1›"));
        assert_eq!(m.mask_value("total", &json!(3)), json!(3));
    }

    /// `table.column` entries apply to their table; bare ones to every table. Until a query is
    /// learned, every entry's column is masked (fail closed).
    #[test]
    fn never_send_entries_follow_their_table() {
        let mut m = Masker::new(["patients.code", "ssn", "public.patients.notes"]);
        assert!(m.is_never_send("code") && m.is_never_send("notes"));
        m.learn_aliases("SELECT code, notes, ssn FROM staff");
        assert!(!m.is_never_send("code") && !m.is_never_send("notes") && m.is_never_send("ssn"));
        m.learn_aliases("SELECT p.code FROM \"Patients\" p");
        assert!(m.is_never_send("code") && m.is_never_send("notes"));
        let r = result(&["code"], vec![vec![json!("x")]]);
        m.learn_aliases("SELECT code FROM staff");
        assert_eq!(m.mask(&r, 5).rows[0][0], json!("x"));
        m.learn_aliases("SELECT code FROM patients");
        assert!(m.mask(&r, 5).rows[0][0].as_str().unwrap().starts_with('‹'));
    }

    #[test]
    fn rows_wider_or_narrower_than_their_columns_are_masked_without_panicking() {
        let mut m = Masker::new(Vec::<String>::new());
        let r = result(
            &["id"],
            vec![vec![json!(1), json!("dana.reyes@example.org")], vec![]],
        );
        let out = m.mask(&r, 5);
        assert!(out.rows[0][1].as_str().unwrap().starts_with('‹'), "{out:?}");
        let s = m.summarize(&r);
        assert_eq!((s.row_count, s.columns.len()), (2, 1));
    }

    #[test]
    fn text_gets_real_values_back() {
        let (m, t) = masker_with(&[json!("Dana Reyes")]);
        assert_eq!(
            m.unmask_text(&format!("The owner is {}. ‹name-9› is unknown.", t[0])),
            "The owner is Dana Reyes. ‹name-9› is unknown."
        );
    }

    /// The phase 1 gate: no planted value in the tenant fixture survives masking, in any
    /// table, on SQLite and on each server that is set.
    #[tokio::test]
    async fn tenant_fixture_pii_never_survives() {
        use crate::db::{self, tenant_fixture};
        let dir = std::env::temp_dir().join(format!("sayql-mask-tenant-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut pools = vec![tenant_fixture::sqlite(&dir.join("tenant.sqlite")).await];
        pools.extend(tenant_fixture::servers().await);
        let planted = tenant_fixture::planted_pii();
        for pool in &pools {
            let mut m = Masker::new(Vec::<String>::new());
            let schema = db::introspect(pool, pool.kind()).await.unwrap();
            assert_eq!(schema.tables.len(), 9, "{:?}", pool.kind());
            let mut raw = String::new();
            for t in &schema.tables {
                let r = db::run_query(pool, &format!("SELECT * FROM {}", t.name), true)
                    .await
                    .unwrap();
                raw.push_str(&serde_json::to_string(&r.rows).unwrap());
                let sent = serde_json::to_string(&m.mask(&r, r.rows.len())).unwrap();
                let summary = serde_json::to_string(&m.summarize(&r)).unwrap();
                for p in &planted {
                    assert!(
                        !sent.contains(p.as_str()),
                        "{:?} {}: {p:?} survived masking",
                        pool.kind(),
                        t.name
                    );
                    assert!(
                        !summary.contains(p.as_str()),
                        "{:?} {}: {p:?} is in the summary",
                        pool.kind(),
                        t.name
                    );
                }
            }
            // Every planted value is really there, so the check above isn't empty.
            for p in &planted {
                assert!(
                    raw.contains(p.as_str()),
                    "{:?}: {p:?} is not in the fixture",
                    pool.kind()
                );
            }
            // Tokens lead back to the real rows: the planted owner, found again by email.
            let r = db::run_query(
                pool,
                "SELECT email FROM users WHERE full_name = 'Priya Natarajan'",
                true,
            )
            .await
            .unwrap();
            let token = m.mask(&r, 1).rows[0][0].as_str().unwrap().to_string();
            let sql = m
                .unmask_sql(
                    &format!("SELECT full_name FROM users WHERE email = '{token}'"),
                    pool.kind(),
                )
                .unwrap();
            assert_eq!(
                db::run_query(pool, &sql, true).await.unwrap().rows,
                [[json!("Priya Natarajan")]]
            );
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Tricky values go in as literals and come back unchanged, on SQLite always and on the
    /// servers when the SAYQL_TEST_*_URL variables are set (see db/integration_tests.rs).
    #[tokio::test]
    async fn literals_round_trip_on_real_databases() {
        use crate::db::{self, ConnConfig};
        let values = [
            json!("O'Brien"),
            json!("Zoë Ångström 李"),
            json!("a\"b`c]d"),
            json!("50% off_"),
            json!("line\nbreak"),
        ];
        let (m, t) = masker_with(&values);
        let dir = std::env::temp_dir().join(format!("sayql-mask-{}", uuid::Uuid::new_v4()));
        let file = dir.join("demo.sqlite");
        db::ensure_demo(&file).await.unwrap();
        let sqlite = ConnConfig {
            id: "t".into(),
            name: "t".into(),
            kind: Kind::Sqlite,
            host: String::new(),
            port: None,
            database: String::new(),
            user: String::new(),
            file: file.to_string_lossy().into_owned(),
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
            let pool = db::connect(&cfg, password).await.unwrap();
            for (v, token) in values.iter().zip(&t) {
                for sql in [
                    format!("SELECT {token} AS v"),
                    format!("SELECT 'x{token}' AS v"),
                ] {
                    let real = m.unmask_sql(&sql, cfg.kind).unwrap();
                    assert!(db::is_read_only(&real, cfg.kind), "{real}");
                    let r = db::run_query(&pool, &real, true)
                        .await
                        .unwrap_or_else(|e| panic!("{:?} {real}: {e}", cfg.kind));
                    let want = if sql.contains("'x") {
                        format!("x{}", plain(v))
                    } else {
                        plain(v)
                    };
                    assert_eq!(r.rows[0][0], json!(want), "{:?}: {real}", cfg.kind);
                }
            }
        }
        let _ = std::fs::remove_dir_all(dir);
    }
}
