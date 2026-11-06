//! The databases an AI may reach on a server shared with other companies or projects.
//!
//! A login on a shared server can often open every database on it, and a query in one database
//! can read another by name. [`DbLimit`] says which ones this connection may use, and [`check`]
//! refuses SQL that names any other database or reads the server's list of them, on the same
//! token lists the read-only and never-send checks use.
//!
//! This stops mistakes and well-behaved AI apps, not a determined query: on SQL Server a view or
//! synonym in an allowed database can point at another one, and SQL text can't show that. Only a
//! separate database login per company guarantees separation, and the docs say so.

use crate::db::Kind;
use crate::safety::never_send::dialect;
use crate::safety::sql_lex::{Tok, lex};

/// What [`check`] refuses with. It names nothing, so it can't confirm another database exists.
pub const REFUSED: &str = "This query refers to a database that isn't available here, or reads the \
server's list of databases. Use the databases list_databases shows.";

/// Which databases on a server may be used. Open by default, as a connection with no list is.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DbLimit {
    /// Only these, when set.
    only: Option<Vec<String>>,
    /// Never these (the databases another group holds).
    except: Vec<String>,
}

impl DbLimit {
    pub fn new(only: Option<Vec<String>>, except: Vec<String>) -> Self {
        DbLimit { only, except }
    }

    /// True when every database on the server may be used.
    pub fn is_open(&self) -> bool {
        self.only.is_none() && self.except.is_empty()
    }

    /// Whether `name` may be used. Names compare without regard to case, which can only refuse
    /// more on a server where case matters.
    pub fn allows(&self, name: &str) -> bool {
        let same = |other: &String| other.eq_ignore_ascii_case(name);
        !self.except.iter().any(same) && self.only.as_ref().is_none_or(|o| o.iter().any(same))
    }

    /// The first database in the list, for a connection whose default isn't allowed.
    pub fn first(&self) -> Option<&str> {
        self.only
            .as_ref()?
            .iter()
            .find(|d| self.allows(d))
            .map(String::as_str)
    }
}

/// SQL Server's ways to read the list of databases or another database's files.
const MSSQL_TABLES: [&str; 3] = ["sysdatabases", "sysaltfiles", "master_files"];
const MSSQL_SYS_VIEWS: [&str; 3] = ["databases", "master_files", "dm_db_file_space_usage"];
const MSSQL_FUNCTIONS: [&str; 6] = [
    "db_name",
    "db_id",
    "has_dbaccess",
    "databasepropertyex",
    "file_name",
    "object_name",
];
/// Postgres' views of every database on the server.
const POSTGRES_NAMES: [&str; 8] = [
    "pg_database",
    "pg_stat_database",
    "pg_stat_database_conflicts",
    "pg_shdescription",
    "pg_db_role_setting",
    "pg_stat_activity",
    "pg_stat_replication",
    "pg_replication_slots",
];
const POSTGRES_FUNCTIONS: [&str; 1] = ["pg_database_size"];

/// Which part of a statement the tokens are in, per bracket depth. Only MySQL needs it: `a.b` in
/// a FROM list is `database.table`, and anywhere else `table.column`.
#[derive(Clone, Copy, PartialEq)]
enum Clause {
    Other,
    From,
    /// A join's ON: `a.b` is a column again, until a comma starts another table.
    On,
}

/// Refuses SQL that could read outside `limit`. An open limit refuses nothing.
pub fn check(kind: Kind, sql: &str, limit: &DbLimit) -> Result<(), &'static str> {
    if limit.is_open() || kind == Kind::Sqlite {
        return Ok(());
    }
    for lexed in lex(sql, dialect(kind)) {
        // MySQL runs what's in `/*! … */`, which the tokens don't show.
        if lexed.exec_comment {
            return Err(REFUSED);
        }
        check_tokens(kind, &lexed.toks, limit)?;
    }
    Ok(())
}

fn check_tokens(kind: Kind, toks: &[Tok], limit: &DbLimit) -> Result<(), &'static str> {
    let first_word = toks.iter().find_map(|t| match t {
        Tok::Word(w) => Some(w.to_lowercase()),
        _ => None,
    });
    let word = |i: usize| match toks.get(i) {
        Some(Tok::Word(w)) => Some(w.to_lowercase()),
        _ => None,
    };
    if kind == Kind::Mysql && first_word.as_deref() == Some("show") {
        let lists_elsewhere = (0..toks.len())
            .filter_map(word)
            .any(|w| matches!(w.as_str(), "databases" | "schemas" | "from" | "in"));
        if lists_elsewhere {
            return Err(REFUSED);
        }
    }
    let starts_with_table = matches!(first_word.as_deref(), Some("desc" | "describe" | "explain"));
    let mut clauses = vec![if starts_with_table {
        Clause::From
    } else {
        Clause::Other
    }];
    let mut i = 0;
    while let Some(tok) = toks.get(i) {
        match tok {
            Tok::Punct('(') => {
                let top = clauses.last().copied().unwrap_or(Clause::Other);
                clauses.push(if top == Clause::From {
                    Clause::From
                } else {
                    Clause::Other
                });
            }
            Tok::Punct(')') if clauses.len() > 1 => {
                clauses.pop();
            }
            Tok::Punct(',') => {
                if let Some(top) = clauses.last_mut()
                    && *top == Clause::On
                {
                    *top = Clause::From;
                }
            }
            _ => {}
        }
        let Some(first) = tok.ident() else {
            i += 1;
            continue;
        };
        // What follows a dot is part of the name that started before it.
        if i > 0 && toks.get(i - 1).is_some_and(|t| t.is_punct('.')) {
            i += 1;
            continue;
        }
        if matches!(tok, Tok::Word(_))
            && let Some(top) = clauses.last_mut()
        {
            match first.as_str() {
                "from" | "join" | "straight_join" => *top = Clause::From,
                "on" if *top == Clause::From => *top = Clause::On,
                "where" | "group" | "order" | "having" | "limit" | "select" | "union"
                | "except" | "intersect" | "window" | "for" | "set" | "offset" | "fetch"
                | "into" => *top = Clause::Other,
                _ => {}
            }
        }
        let (parts, skipped_part, end) = name_at(toks, i);
        let called = toks.get(end).is_some_and(|t| t.is_punct('('));
        let clause = clauses.last().copied().unwrap_or(Clause::Other);
        check_name(kind, &parts, skipped_part, called, clause, limit)?;
        i = end.max(i + 1);
    }
    Ok(())
}

/// The dotted name starting at `start`, lowercased: its parts, whether a part was left out
/// (T-SQL `db..table`), and the index after it.
fn name_at(toks: &[Tok], start: usize) -> (Vec<String>, bool, usize) {
    let mut parts: Vec<String> = toks.get(start).and_then(Tok::ident).into_iter().collect();
    let mut skipped = false;
    let mut i = start + 1;
    while toks.get(i).is_some_and(|t| t.is_punct('.')) {
        i += 1;
        if toks.get(i).is_some_and(|t| t.is_punct('.')) {
            skipped = true;
            continue;
        }
        match toks.get(i).and_then(Tok::ident) {
            Some(part) => {
                parts.push(part);
                i += 1;
            }
            // `t.*`, or a dot before something that isn't a name.
            None => break,
        }
    }
    (parts, skipped, i)
}

fn check_name(
    kind: Kind,
    parts: &[String],
    skipped_part: bool,
    called: bool,
    clause: Clause,
    limit: &DbLimit,
) -> Result<(), &'static str> {
    let (Some(first), Some(last)) = (parts.first(), parts.last()) else {
        return Ok(());
    };
    let names_database = match kind {
        Kind::Mssql => {
            // A four-part name goes through a linked server, to any database at all.
            if parts.len() >= 4 {
                return Err(REFUSED);
            }
            let in_sys = parts.len() >= 2
                && parts.get(parts.len() - 2).is_some_and(|p| p == "sys")
                && MSSQL_SYS_VIEWS.contains(&last.as_str());
            if in_sys
                || MSSQL_TABLES.contains(&last.as_str())
                || (called && MSSQL_FUNCTIONS.contains(&last.as_str()))
            {
                return Err(REFUSED);
            }
            parts.len() == 3 || skipped_part
        }
        Kind::Mysql => parts.len() >= 3 || (parts.len() == 2 && (clause == Clause::From || called)),
        Kind::Postgres => {
            if POSTGRES_NAMES.contains(&last.as_str())
                || (called && POSTGRES_FUNCTIONS.contains(&last.as_str()))
            {
                return Err(REFUSED);
            }
            // Postgres can't read another database by name; the server refuses it.
            false
        }
        Kind::Sqlite => false,
    };
    if names_database && !limit.allows(first) {
        return Err(REFUSED);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn acme() -> DbLimit {
        DbLimit::new(Some(vec!["acme".into(), "acme_crm".into()]), vec![])
    }

    fn refuses(kind: Kind, sql: &str) {
        assert_eq!(check(kind, sql, &acme()), Err(REFUSED), "{sql}");
    }

    fn allows(kind: Kind, sql: &str) {
        assert_eq!(check(kind, sql, &acme()), Ok(()), "{sql}");
    }

    #[test]
    fn an_open_limit_refuses_nothing() {
        let open = DbLimit::default();
        assert_eq!(
            check(Kind::Mssql, "SELECT * FROM other.dbo.t", &open),
            Ok(())
        );
        assert_eq!(check(Kind::Mysql, "SHOW DATABASES", &open), Ok(()));
    }

    #[test]
    fn sql_server_names_outside_the_list_are_refused_however_they_are_written() {
        for sql in [
            "SELECT * FROM other.dbo.customers",
            "SELECT * FROM [other].[dbo].[customers]",
            "SELECT * FROM \"other\".dbo.customers",
            "SELECT * FROM OTHER . dbo . customers",
            "SELECT * FROM other..customers",
            "SELECT * FROM acme.dbo.t JOIN other.dbo.t ON 1 = 1",
            "SELECT (SELECT count(*) FROM other.dbo.t) AS n",
            "SELECT other.dbo.fn_secret(1)",
            "SELECT * FROM /* acme */ other/* x */.dbo.t",
            "SELECT * FROM linked.acme.dbo.t",
        ] {
            refuses(Kind::Mssql, sql);
        }
    }

    #[test]
    fn sql_server_queries_inside_the_list_still_run() {
        for sql in [
            "SELECT * FROM customers",
            "SELECT * FROM dbo.customers",
            "SELECT c.name FROM dbo.customers c",
            "SELECT * FROM acme.dbo.customers",
            "SELECT * FROM ACME_CRM.dbo.customers",
            "SELECT * FROM acme..customers",
            "SELECT 'other.dbo.t' AS text",
        ] {
            allows(Kind::Mssql, sql);
        }
    }

    #[test]
    fn sql_server_can_not_read_the_list_of_databases() {
        for sql in [
            "SELECT name FROM sys.databases",
            "SELECT name FROM [sys].[databases]",
            "SELECT name FROM acme.sys.databases",
            "SELECT name FROM master.sys.databases",
            "SELECT name FROM sysdatabases",
            "SELECT * FROM sys.master_files",
            "SELECT DB_NAME(5)",
            "SELECT HAS_DBACCESS('other')",
        ] {
            refuses(Kind::Mssql, sql);
        }
        // A column or table of the same name is fine.
        allows(Kind::Mssql, "SELECT databases FROM dbo.inventory");
    }

    #[test]
    fn mysql_names_outside_the_list_are_refused_however_they_are_written() {
        for sql in [
            "SELECT * FROM other.customers",
            "SELECT * FROM `other`.`customers`",
            "SELECT * FROM other . customers",
            "SELECT * FROM a, other.customers",
            "SELECT * FROM (a, other.customers)",
            "SELECT * FROM a JOIN other.customers ON 1 = 1",
            "SELECT * FROM a JOIN b ON a.x = b.x, other.customers",
            "SELECT * FROM a WHERE id IN (SELECT id FROM other.t)",
            "SELECT other.customers.email FROM a",
            "SELECT other.secret_fn(1)",
            "DESCRIBE other.customers",
            "SELECT * FROM acme.t UNION SELECT * FROM other.t",
            "SELECT schema_name FROM information_schema.schemata",
            "SELECT * FROM mysql.user",
            "SHOW DATABASES",
            "SHOW SCHEMAS",
            "SHOW TABLES FROM other",
            "SHOW TABLES IN other",
            "SHOW COLUMNS FROM t FROM other",
            "SELECT 1 /*! FROM other.t */",
        ] {
            refuses(Kind::Mysql, sql);
        }
    }

    #[test]
    fn mysql_queries_inside_the_list_still_run() {
        for sql in [
            "SELECT * FROM customers",
            "SELECT c.email, o.total FROM customers c JOIN orders o ON c.id = o.customer_id",
            "SELECT * FROM a, b WHERE a.id = b.id",
            "SELECT * FROM acme.customers",
            "SELECT acme.customers.email FROM acme.customers",
            "SELECT t.* FROM customers t",
            "SELECT * FROM a JOIN b ON a.x = b.x JOIN c ON b.y = c.y",
            "SELECT count(*), max(o.total) FROM orders o GROUP BY o.status",
            "SELECT * FROM customers ORDER BY customers.name, customers.id",
            "SHOW TABLES",
        ] {
            allows(Kind::Mysql, sql);
        }
    }

    #[test]
    fn postgres_can_not_read_the_list_of_databases() {
        for sql in [
            "SELECT datname FROM pg_database",
            "SELECT datname FROM pg_catalog.pg_database",
            "SELECT * FROM \"pg_database\"",
            "SELECT * FROM pg_stat_activity",
            "SELECT pg_database_size('other')",
        ] {
            refuses(Kind::Postgres, sql);
        }
        allows(Kind::Postgres, "SELECT c.name FROM public.customers c");
    }

    #[test]
    fn a_database_another_group_holds_is_refused_while_the_rest_stay_open() {
        let limit = DbLimit::new(None, vec!["other".into()]);
        assert_eq!(
            check(Kind::Mssql, "SELECT * FROM other.dbo.t", &limit),
            Err(REFUSED)
        );
        assert_eq!(
            check(Kind::Mssql, "SELECT * FROM acme.dbo.t", &limit),
            Ok(())
        );
        assert!(!limit.allows("OTHER"));
        assert!(limit.allows("anything"));
    }

    #[test]
    fn sqlite_is_one_file_so_a_limit_does_not_apply() {
        allows(Kind::Sqlite, "SELECT * FROM main.customers");
    }

    #[test]
    fn the_first_allowed_database_skips_excluded_ones() {
        let limit = DbLimit::new(Some(vec!["a".into(), "b".into()]), vec!["a".into()]);
        assert_eq!(limit.first(), Some("b"));
        assert_eq!(DbLimit::default().first(), None);
    }

    #[test]
    fn the_refusal_names_no_database() {
        assert!(!REFUSED.contains("other"));
    }
}
