//! Database errors as an AI may see them. Errors about failed conversions, duplicate keys or
//! unknown names often quote a value from the data (`invalid input syntax for type integer:
//! "ann@example.com"`, `unrecognized configuration parameter "Alice Smith"`), so they can
//! carry row values out of a query whose results the AI may not see.
//!
//! Two forms:
//! - [`safe_error`]: what AI apps (MCP) and agent mode get. A fixed message per kind of error,
//!   chosen from the database's error code (Postgres SQLSTATE, MySQL error number, SQL Server
//!   error number, SQLite's message prefix). The database's own text is never passed on; a
//!   name from it is added only when that exact text is in the SQL the AI wrote, so it tells
//!   the AI nothing it didn't already know.
//! - [`database_error`]: the database's message with every quoted value masked as `…`. Quoted
//!   text is kept only when it is in the SQL (names the query used) or names a part of the
//!   query (`in 'field list'`); lines that can quote data unmarked (Postgres DETAIL and
//!   CONTEXT) are dropped, except the key lists, whose values are masked.

use crate::error::AppError;

const MASK: &str = "…";

/// Text before a parenthesised value list, masked up to the last `)` on the line:
/// Postgres `Key (email)=(a@b.com) already exists.` and `Failing row contains (1, x).`,
/// SQL Server `The duplicate key value is (42).`
const VALUE_LISTS: &[&str] = &[")=(", "failing row contains (", "duplicate key value is ("];

/// MySQL names parts of the query in quotes: `Unknown column 'x' in 'field list'`; MariaDB 11
/// names the clause instead: `in 'SELECT'`.
const QUERY_PARTS: &[&str] = &[
    "field list",
    "where clause",
    "on clause",
    "order clause",
    "group statement",
    "having clause",
    "select",
    "where",
    "on",
    "order by",
    "group by",
    "having",
    "set",
    "returning",
];

/// Text that follows a quoted value in MySQL and SQL Server messages, used to find where
/// a value that itself contains a quote ends (`Duplicate entry 'O'Brien' for key ...`).
const AFTER_VALUE: &[&str] = &[
    "' for key",
    "' for column",
    "' to data type",
    "' at row",
    "' overflowed",
];

/// The database error with values masked: quoted text that isn't in `sql` becomes `…`, and
/// lines that can hold unquoted data are dropped.
pub(crate) fn database_error(error: &str, sql: &str) -> String {
    let mut out = Vec::new();
    for (i, line) in error.lines().enumerate() {
        let t = line.trim_start();
        let keep = i == 0
            || t.starts_with("HINT:")
            || t.starts_with("(SQL Server error")
            || (t.starts_with("DETAIL:") && has_value_list(t));
        if keep {
            out.push(mask_quotes(&mask_value_lists(line), sql));
        }
    }
    out.join("\n")
}

fn has_value_list(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    VALUE_LISTS.iter().any(|m| lower.contains(m))
}

fn mask_value_lists(line: &str) -> String {
    let mut line = line.to_string();
    let mut from = 0;
    loop {
        let lower = line.to_ascii_lowercase();
        let Some(start) = VALUE_LISTS
            .iter()
            .filter_map(|m| lower[from..].find(m).map(|i| from + i + m.len()))
            .min()
        else {
            return line;
        };
        let end = line[start..].rfind(')').map_or(line.len(), |i| start + i);
        line.replace_range(start..end, MASK);
        from = start + MASK.len();
    }
}

/// Whether `text` is in the SQL the AI wrote (so passing it back reveals nothing new).
fn in_sql(text: &str, sql: &str) -> bool {
    let t = text.trim();
    !t.is_empty() && sql.to_lowercase().contains(&t.to_lowercase())
}

fn mask_quotes(line: &str, sql: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut i = 0;
    while let Some(offset) = line[i..].find(['\'', '"']) {
        let open = i + offset;
        let quote = &line[open..open + 1];
        out.push_str(&line[i..open]);
        let body = open + 1;
        let is_apostrophe = quote == "'"
            && line[..open]
                .chars()
                .next_back()
                .is_some_and(char::is_alphanumeric)
            && line[body..]
                .chars()
                .next()
                .is_some_and(char::is_alphanumeric);
        if is_apostrophe {
            out.push_str(quote);
            i = body;
            continue;
        }
        let next_quote = line[body..].find(quote).map(|j| body + j);
        let keep = next_quote.is_some_and(|q| {
            let text = &line[body..q];
            QUERY_PARTS
                .iter()
                .any(|p| p.eq_ignore_ascii_case(text.trim()))
                || in_sql(text, sql)
        });
        if keep && let Some(close) = next_quote {
            out.push_str(&line[open..=close]);
            i = close + 1;
            continue;
        }
        out.push_str(quote);
        out.push_str(MASK);
        match value_end(line, body, quote) {
            Some(close) => {
                out.push_str(quote);
                i = close + 1;
            }
            None => return out,
        }
    }
    out.push_str(&line[i..]);
    out
}

/// Where a masked value that starts at `body` ends: before known trailing text if the
/// line has it, else at the first quote followed by a space, punctuation or the line end.
fn value_end(line: &str, body: usize, quote: &str) -> Option<usize> {
    let rest = &line[body..];
    if quote == "'" {
        let lower = rest.to_ascii_lowercase();
        if let Some(i) = AFTER_VALUE.iter().filter_map(|t| lower.find(t)).min() {
            return Some(body + i);
        }
    }
    rest.match_indices(quote)
        .find(|(i, _)| {
            rest[i + 1..]
                .chars()
                .next()
                .is_none_or(|c| c.is_whitespace() || ".,;:)]".contains(c))
        })
        .map(|(i, _)| body + i)
}

/// Every quoted run in `msg` (`'x'`, `"x"`, `[x]`), and Postgres' unquoted
/// `column t.x does not exist`.
fn candidates(msg: &str) -> Vec<String> {
    let mut out = Vec::new();
    for (open, close) in [('"', '"'), ('\'', '\''), ('[', ']')] {
        let mut rest = msg;
        while let Some(a) = rest.find(open) {
            let after = &rest[a + 1..];
            let Some(b) = after.find(close) else { break };
            out.push(after[..b].to_string());
            rest = &after[b + 1..];
        }
    }
    for (before, after) in [
        ("column ", " does not exist"),
        ("no such column: ", ""),
        ("no such table: ", ""),
        ("no such function: ", ""),
        ("ambiguous column name: ", ""),
    ] {
        if let Some(a) = msg.find(before) {
            let rest = &msg[a + before.len()..];
            let end = if after.is_empty() {
                rest.find(char::is_whitespace).unwrap_or(rest.len())
            } else {
                rest.find(after).unwrap_or(0)
            };
            out.push(rest[..end].trim_matches('"').to_string());
        }
    }
    out
}

/// ` "x"`: the first name in the message that is also in the SQL, else nothing.
fn name(msg: &str, sql: &str) -> String {
    candidates(msg)
        .into_iter()
        .find(|c| c.chars().count() <= 128 && in_sql(c, sql))
        .map(|c| format!(": \"{c}\""))
        .unwrap_or_default()
}

const HIDDEN: &str =
    "The database's own message isn't shown, because it can quote values from the data.";
const VALUE_HIDDEN: &str = "The value isn't shown.";

/// The error for an AI that may not see row values: a fixed message for its kind, from the
/// database's error code. See the module docs.
pub(crate) fn safe_error(e: &AppError, sql: &str) -> String {
    match e {
        AppError::Database(sqlx::Error::Database(db)) => {
            if let Some(pg) = db.try_downcast_ref::<sqlx::postgres::PgDatabaseError>() {
                let at = match pg.position() {
                    Some(sqlx::postgres::PgErrorPosition::Original(p)) => {
                        format!(" (at character {p})")
                    }
                    _ => String::new(),
                };
                return postgres(pg.code(), pg.message(), &at, sql);
            }
            if let Some(my) = db.try_downcast_ref::<sqlx::mysql::MySqlDatabaseError>() {
                return mysql(my.number(), my.message(), sql);
            }
            sqlite(db.message(), sql)
        }
        AppError::Database(sqlx::Error::Io(_)) => "Lost the connection to the database.".into(),
        AppError::Database(sqlx::Error::PoolTimedOut) => {
            "Timed out waiting for a connection to the database.".into()
        }
        AppError::Database(_) => {
            format!("The database driver reported an error, so the query didn't finish. {HIDDEN}")
        }
        AppError::SqlServer(tiberius::error::Error::Server(t)) => mssql(t.code(), t.message(), sql),
        AppError::SqlServer(_) => {
            format!("Lost the connection to SQL Server, or its driver reported an error. {HIDDEN}")
        }
        // SayQL's own messages (stopped, read-only, nothing to run) and a team server's.
        AppError::User(m) | AppError::TeamRefused(m) => database_error(m, sql),
        other => database_error(&other.to_string(), sql),
    }
}

fn postgres(code: &str, msg: &str, at: &str, sql: &str) -> String {
    let n = || name(msg, sql);
    match code {
        "42601" => format!("Syntax error in the SQL{}{at}.", n()),
        "42703" => format!("A column in the query does not exist{}{at}.", n()),
        "42P01" => format!("A table or view in the query does not exist{}{at}.", n()),
        "3F000" => format!("A schema in the query does not exist{}{at}.", n()),
        "3D000" => format!("A database in the query does not exist{}.", n()),
        "42883" => format!(
            "A function or operator in the query does not exist for these argument types{}{at}. Check the name, or add casts.",
            n()
        ),
        "42704" => format!(
            "Something the query names (a type, setting, collation or other object) does not exist{}{at}.",
            n()
        ),
        "42702" => format!(
            "A column name in the query is ambiguous{}{at}; qualify it with its table.",
            n()
        ),
        "42725" => format!(
            "A function call in the query is ambiguous{}{at}; add casts.",
            n()
        ),
        "42803" => format!(
            "A selected column must appear in GROUP BY or be used in an aggregate function{}{at}.",
            n()
        ),
        "42804" | "42846" | "42P18" => format!(
            "Types in the query don't match (a comparison, CASE branches, UNION columns or a cast){at}; add casts."
        ),
        "42501" => "The saved login doesn't have permission for something this query reads.".into(),
        "25006" => "This statement could change data; SayQL runs these queries read-only.".into(),
        "57014" => "The query was stopped (a statement timeout or a cancel).".into(),
        "22012" => "Division by zero.".into(),
        "22003" => format!("A number is out of range for its type. {VALUE_HIDDEN}"),
        "22001" => format!("A value is too long for its type. {VALUE_HIDDEN}"),
        "22007" | "22008" => {
            format!("A date or time value couldn't be read or is out of range. {VALUE_HIDDEN}")
        }
        "22P02" => format!(
            "A value couldn't be converted to the type the query asks for (a cast, or a comparison with a value in the data). {VALUE_HIDDEN}"
        ),
        "22023" => format!("A function got an argument value it doesn't accept. {VALUE_HIDDEN}"),
        "2200M" | "2200N" | "2200L" => format!("A value isn't valid XML. {VALUE_HIDDEN}"),
        "22030" | "22032" | "22033" | "22034" | "22035" | "22036" | "22037" | "22038" | "22039"
        | "2203A" | "2203B" | "2203C" | "2203D" | "2203E" | "2203F" => {
            format!("A value isn't valid JSON, or a JSON path failed. {VALUE_HIDDEN}")
        }
        "21000" => "A subquery used as a value returned more than one row.".into(),
        "0A000" => {
            "This database doesn't support something in the query (feature not supported).".into()
        }
        "P0001" => format!("A database function raised an error (SQLSTATE P0001). {HIDDEN}"),
        c if c.starts_with("42") => format!(
            "The query isn't valid SQL for this database (SQLSTATE {c}){}{at}.",
            n()
        ),
        c if c.starts_with("22") => format!(
            "A value in the data or the query couldn't be processed (SQLSTATE {c}). {VALUE_HIDDEN}"
        ),
        c if c.starts_with("08") => "Lost the connection to the database.".into(),
        c if c.starts_with("53") => {
            "The database ran out of resources (memory, disk or connections).".into()
        }
        c if c.starts_with("54") => {
            "The query is too complex or too large for the database.".into()
        }
        c if c.starts_with("40") => {
            "The database rolled the query back (a deadlock or serialization failure); try again."
                .into()
        }
        c => format!("The database rejected the query (SQLSTATE {c}). {HIDDEN}"),
    }
}

fn mysql(number: u16, msg: &str, sql: &str) -> String {
    let n = || name(msg, sql);
    let line = msg
        .rsplit_once(" at line ")
        .and_then(|(_, l)| l.trim().parse::<u32>().ok())
        .map(|l| format!(" (line {l})"))
        .unwrap_or_default();
    match number {
        1064 => format!("Syntax error in the SQL{}{line}.", n()),
        1054 => format!("A column in the query does not exist{}.", n()),
        1146 => format!("A table or view in the query does not exist{}.", n()),
        1049 => format!("A database in the query does not exist{}.", n()),
        1305 | 1630 => format!("A function in the query does not exist{}.", n()),
        1052 => format!("A column name in the query is ambiguous{}; qualify it with its table.", n()),
        1055 | 1140 | 1111 => "A selected column must appear in GROUP BY, or an aggregate is used where it isn't allowed.".into(),
        1242 => "A subquery used as a value returned more than one row.".into(),
        1241 => "A subquery or row returns a different number of columns than expected.".into(),
        1222 => "The SELECTs in a UNION return different numbers of columns.".into(),
        1248 => "Every derived table (subquery in FROM) needs an alias.".into(),
        1060 | 1066 => format!("A column or table alias is used twice{}.", n()),
        1582 | 1584 => format!("A function got the wrong number or kind of arguments{}.", n()),
        1267 | 1271 => "The query mixes text in collations that can't be compared; add COLLATE or CONVERT.".into(),
        1292 | 1366 | 1367 | 1525 | 1411 => format!(
            "A value couldn't be converted to the type the query asks for. {VALUE_HIDDEN}"
        ),
        1690 | 1264 => format!("A number is out of range for its type. {VALUE_HIDDEN}"),
        1365 => "Division by zero.".into(),
        1406 => format!("A value is too long for its type. {VALUE_HIDDEN}"),
        3140 | 3141 | 3143 | 3146 | 3156 => {
            format!("A value isn't valid JSON, or a JSON path failed. {VALUE_HIDDEN}")
        }
        1044 | 1045 | 1142 | 1143 | 1227 | 1370 => {
            "The saved login doesn't have permission for something this query reads.".into()
        }
        1792 | 1290 => "This statement could change data; SayQL runs these queries read-only.".into(),
        3024 | 1317 | 1969 | 1028 => "The query was stopped (a statement time limit or a cancel).".into(),
        1235 => "This MySQL version doesn't support something in the query.".into(),
        n => format!("MySQL rejected the query (error {n}). {HIDDEN}"),
    }
}

fn sqlite(msg: &str, sql: &str) -> String {
    let n = || name(msg, sql);
    let m = msg.to_ascii_lowercase();
    let has = |p: &str| m.contains(p);
    if has("no such column") {
        format!("A column in the query does not exist{}.", n())
    } else if has("no such table") {
        format!("A table or view in the query does not exist{}.", n())
    } else if has("no such function") || has("wrong number of arguments") {
        format!(
            "A function in the query does not exist, or got the wrong number of arguments{}.",
            n()
        )
    } else if has("syntax error") || has("incomplete input") || has("unrecognized token") {
        format!("Syntax error in the SQL{}.", n())
    } else if has("ambiguous column") {
        format!(
            "A column name in the query is ambiguous{}; qualify it with its table.",
            n()
        )
    } else if has("misuse of aggregate") || has("group by") || has("aggregate functions") {
        "An aggregate function is used where it isn't allowed, or GROUP BY is missing.".into()
    } else if has("do not have the same number of result columns") {
        "The SELECTs in a UNION return different numbers of columns.".into()
    } else if has("sub-select returns") || has("row value misused") {
        "A subquery or row returns a different number of columns than expected.".into()
    } else if has("json") {
        format!("A value isn't valid JSON, or a JSON path failed. {VALUE_HIDDEN}")
    } else if has("readonly") || has("read-only") {
        "This statement could change data; SayQL runs these queries read-only.".into()
    } else if has("interrupted") {
        "The query was stopped.".into()
    } else if has("integer overflow") {
        format!("A number is out of range. {VALUE_HIDDEN}")
    } else if has("datatype mismatch") {
        "Types in the query don't match.".into()
    } else if has("out of range") {
        "A term in ORDER BY or GROUP BY is out of range.".into()
    } else if has("too many") || has("stack overflow") || has("too big") {
        "The query is too complex or too large for SQLite.".into()
    } else {
        format!("SQLite rejected the query. {HIDDEN}")
    }
}

fn mssql(code: u32, msg: &str, sql: &str) -> String {
    let n = || name(msg, sql);
    match code {
        102 | 156 | 170 | 105 | 319 => format!("Syntax error in the SQL{}.", n()),
        207 => format!("A column in the query does not exist{}.", n()),
        208 => format!("A table or view in the query does not exist{}.", n()),
        4104 => format!("A column reference in the query couldn't be bound to a table{}.", n()),
        209 => format!("A column name in the query is ambiguous{}; qualify it with its table.", n()),
        195 | 4121 | 2812 => format!("A function in the query does not exist{}.", n()),
        174 | 189 => format!("A function got the wrong number of arguments{}.", n()),
        8120 | 130 | 147 => "A selected column must appear in GROUP BY, or an aggregate is used where it isn't allowed.".into(),
        512 => "A subquery used as a value returned more than one row.".into(),
        116 => "A subquery returns more than one column where only one is allowed.".into(),
        205 => "The SELECTs in a UNION return different numbers of columns.".into(),
        145 | 1033 => "ORDER BY isn't allowed here, or its items must appear in the select list.".into(),
        4145 => "A WHERE or ON condition isn't a true/false expression.".into(),
        245 | 241 | 242 | 8114 | 8116 | 295 | 9807 | 8169 | 6522 => format!(
            "A value couldn't be converted to the type the query asks for. {VALUE_HIDDEN}"
        ),
        220 | 232 | 8115 | 8152 | 2628 => format!("A value is out of range or too long for its type. {VALUE_HIDDEN}"),
        8134 => "Division by zero.".into(),
        13607..=13609 => format!("A value isn't valid JSON, or a JSON path failed. {VALUE_HIDDEN}"),
        229 | 230 | 297 | 916 | 262 | 300 => {
            "The saved login doesn't have permission for something this query reads.".into()
        }
        3906 => "This statement could change data; SayQL runs these queries read-only.".into(),
        1222 => "The query waited too long for a lock.".into(),
        c => format!("SQL Server rejected the query (error {c}). {HIDDEN}"),
    }
}

#[cfg(test)]
mod tests {
    use super::database_error;

    fn r(e: &str) -> String {
        database_error(e, "")
    }

    #[test]
    fn postgres_values_are_masked_and_names_from_the_sql_kept() {
        assert_eq!(
            r(
                "duplicate key value violates unique constraint \"users_email_key\"\nDETAIL:  Key (email)=(ann@example.com) already exists."
            ),
            "duplicate key value violates unique constraint \"…\"\nDETAIL:  Key (email)=(…) already exists."
        );
        assert_eq!(
            database_error(
                "DETAIL:  Key (user_id)=(99) is not present in table \"users\".",
                "insert into orders select 99 from users"
            ),
            "DETAIL:  Key (user_id)=(…) is not present in table \"users\"."
        );
        assert_eq!(
            r(
                "null value in column \"name\" of relation \"clients\" violates not-null constraint\nDETAIL:  Failing row contains (12, null, Ann (VIP), 2024-01-01)."
            ),
            "null value in column \"…\" of relation \"…\" violates not-null constraint\nDETAIL:  Failing row contains (…)."
        );
        assert_eq!(
            r("invalid input syntax for type integer: \"abc\""),
            "invalid input syntax for type integer: \"…\""
        );
        assert_eq!(
            database_error("column \"nme\" does not exist", "SELECT nme FROM t"),
            "column \"nme\" does not exist"
        );
        assert_eq!(
            database_error(
                "relation \"public.clients\" does not exist",
                "select * from public.clients"
            ),
            "relation \"public.clients\" does not exist"
        );
    }

    /// The forms the governance test found leaking values (2026-10-28).
    #[test]
    fn quoted_data_never_survives_without_being_in_the_sql() {
        for (e, sql, secret) in [
            (
                "unrecognized configuration parameter \"chandra@example.in\"",
                "SELECT current_setting(email) FROM customers WHERE id=3",
                "chandra",
            ),
            (
                "unrecognized configuration parameter \"987-65-4321\"",
                "SELECT current_setting(to_jsonb(c)->>('s'||'sn')) FROM customers c",
                "987",
            ),
            (
                "invalid source encoding name \"Alice Smith\"",
                "SELECT convert_from(name::bytea, name) FROM customers",
                "Alice",
            ),
            (
                "invalid XML document\nDETAIL:  line 1: Start tag expected, '<' not found\nAlice Smith\n^",
                "SELECT xmlparse(document name) FROM customers",
                "Alice",
            ),
            (
                "schema \"alice@example\" does not exist",
                "SELECT email::regclass FROM customers",
                "alice",
            ),
            (
                "syntax error at or near \"Smith\"",
                "SELECT name::regtype FROM customers",
                "Smith",
            ),
            (
                "invalid input syntax for type json\nDETAIL:  Token \"alice\" is invalid.\nCONTEXT:  JSON data, line 1: alice...",
                "SELECT email::json FROM customers",
                "alice",
            ),
            (
                "relation \"bob\" does not exist",
                "SELECT name::regclass FROM customers",
                "bob",
            ),
        ] {
            let m = database_error(e, sql);
            assert!(!m.contains(secret), "{e} → {m}");
        }
    }

    #[test]
    fn mysql_and_sql_server_values_are_masked() {
        assert_eq!(
            r("Duplicate entry 'ann@example.com' for key 'users.email'"),
            "Duplicate entry '…' for key '…'"
        );
        assert_eq!(
            r("Duplicate entry 'O'Brien-42' for key 'PRIMARY'"),
            "Duplicate entry '…' for key '…'"
        );
        assert_eq!(
            database_error(
                "Incorrect integer value: 'abc' for column 'age' at row 1",
                "insert into p (age) values (x)"
            ),
            "Incorrect integer value: '…' for column 'age' at row 1"
        );
        assert_eq!(
            database_error("Unknown column 'nme' in 'field list'", "select nme from t"),
            "Unknown column 'nme' in 'field list'"
        );
        assert_eq!(
            database_error("Unknown column 'nme' in 'SELECT'", "select nme from t"),
            "Unknown column 'nme' in 'SELECT'"
        );
        assert_eq!(
            r("Conversion failed when converting the varchar value 'abc' to data type int."),
            "Conversion failed when converting the varchar value '…' to data type int."
        );
        assert_eq!(
            r("The conversion of the varchar value '99999999999' overflowed an int column."),
            "The conversion of the varchar value '…' overflowed an int column."
        );
        assert_eq!(
            database_error(
                "Invalid column name 'nme'.\n(SQL Server error 207, line 1)",
                "SELECT nme FROM t"
            ),
            "Invalid column name 'nme'.\n(SQL Server error 207, line 1)"
        );
        assert_eq!(
            database_error("Incorrect syntax near 'FROM'.", "SELECT FROM t"),
            "Incorrect syntax near 'FROM'."
        );
    }

    #[test]
    fn sqlite_messages_keep_their_names() {
        for e in [
            "UNIQUE constraint failed: users.email",
            "no such column: nme",
            "datatype mismatch",
        ] {
            assert_eq!(r(e), e);
        }
        assert_eq!(
            database_error("near \"SELEC\": syntax error", "SELEC 1"),
            "near \"SELEC\": syntax error"
        );
    }

    #[test]
    fn unknown_quoted_text_is_masked() {
        assert_eq!(
            r("something odd with 'secret' here"),
            "something odd with '…' here"
        );
        assert_eq!(r("unterminated value: 'secret"), "unterminated value: '…");
    }

    #[test]
    fn safe_messages_name_only_what_the_sql_has() {
        assert_eq!(
            super::postgres(
                "42703",
                "column c.nme does not exist",
                " (at character 8)",
                "SELECT c.nme FROM t c"
            ),
            "A column in the query does not exist: \"c.nme\" (at character 8)."
        );
        let m = super::postgres(
            "42704",
            "unrecognized configuration parameter \"chandra@example.in\"",
            "",
            "SELECT current_setting(email) FROM customers",
        );
        assert!(
            !m.contains("chandra") && m.contains("does not exist"),
            "{m}"
        );
        let m = super::postgres(
            "22P02",
            "invalid input syntax for type integer: \"alice@x.com\"",
            "",
            "SELECT email::int",
        );
        assert!(!m.contains("alice") && m.contains("isn't shown"), "{m}");
        let m = super::postgres(
            "3F000",
            "schema \"alice@example\" does not exist",
            "",
            "SELECT email::regclass",
        );
        assert!(!m.contains("alice"), "{m}");
        let m = super::postgres(
            "42601",
            "syntax error at or near \"Smith\"",
            "",
            "SELECT name::regtype",
        );
        assert!(!m.contains("Smith"), "{m}");
        let m = super::postgres("2200N", "invalid XML document", "", "x");
        assert!(m.contains("XML"), "{m}");
        assert!(
            super::mysql(1054, "Unknown column 'nme' in 'field list'", "select nme")
                .contains("\"nme\"")
        );
        assert!(
            !super::mysql(
                1292,
                "Truncated incorrect DOUBLE value: 'ann@x.com'",
                "select email+0"
            )
            .contains("ann")
        );
        assert!(
            !super::mssql(
                245,
                "Conversion failed when converting the varchar value 'Ann' to data type int.",
                "select cast(name as int)"
            )
            .contains("Ann")
        );
        assert!(super::mssql(207, "Invalid column name 'nme'.", "select nme").contains("\"nme\""));
        assert!(super::sqlite("no such column: nme", "select nme").contains("\"nme\""));
        assert!(
            !super::sqlite("bad JSON path: 'Alice'", "select json_extract(x, name)")
                .contains("Alice")
        );
    }
}
