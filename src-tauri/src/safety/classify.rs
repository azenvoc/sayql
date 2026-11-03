use crate::db::Kind;
use crate::safety::sql_lex::{Tok, lex};

/// Keywords that can change data, structure, permissions, session or transaction state.
/// `INTO` covers `SELECT ... INTO new_table` and `INSERT INTO`; `USE` and `SET` change
/// the session (SQL Server's `USE` would move a pooled connection to another database).
/// `OUTFILE`/`DUMPFILE` write files on MySQL; the function names below have side effects
/// that a read-only transaction doesn't stop (or that leave the database).
const WRITE_KEYWORDS: &[&str] = &[
    "INSERT",
    "UPDATE",
    "DELETE",
    "MERGE",
    "UPSERT",
    "INTO",
    "DROP",
    "ALTER",
    "CREATE",
    "TRUNCATE",
    "RENAME",
    "GRANT",
    "REVOKE",
    "DENY",
    "EXEC",
    "EXECUTE",
    "CALL",
    "COMMIT",
    "ROLLBACK",
    "SAVEPOINT",
    "BEGIN",
    "BACKUP",
    "RESTORE",
    "DBCC",
    "KILL",
    "SHUTDOWN",
    "RECONFIGURE",
    "BULK",
    "OPENROWSET",
    "OPENQUERY",
    "OPENDATASOURCE",
    "ATTACH",
    "DETACH",
    "VACUUM",
    "REINDEX",
    "COPY",
    "LOCK",
    "USE",
    "SET",
    "CHECKPOINT",
    "RECEIVE",
    "OUTFILE",
    "DUMPFILE",
    "LOAD_EXTENSION",
    "DBLINK",
    "DBLINK_EXEC",
    "LO_IMPORT",
    "LO_EXPORT",
    "LO_UNLINK",
    "NEXTVAL",
    "SETVAL",
    "PG_TERMINATE_BACKEND",
    "PG_CANCEL_BACKEND",
    "PG_RELOAD_CONF",
    "PG_ROTATE_LOGFILE",
    "PG_PROMOTE",
    "PG_SWITCH_WAL",
    // Effects that a read-only transaction allows and a rollback doesn't undo. Session-level
    // locks stay held by the pooled connection; replication slots keep WAL until the disk fills.
    "PG_ADVISORY_LOCK",
    "PG_ADVISORY_LOCK_SHARED",
    "PG_TRY_ADVISORY_LOCK",
    "PG_TRY_ADVISORY_LOCK_SHARED",
    "GET_LOCK",
    "PG_CREATE_RESTORE_POINT",
    "PG_BACKUP_START",
    "PG_BACKUP_STOP",
    "PG_START_BACKUP",
    "PG_STOP_BACKUP",
    "PG_WAL_REPLAY_PAUSE",
    "PG_WAL_REPLAY_RESUME",
    "PG_LOGICAL_EMIT_MESSAGE",
    "PG_FILE_WRITE",
    "PG_FILE_RENAME",
    "PG_FILE_UNLINK",
    // SQL Server: `SETUSER` changes who the session acts as; `WRITETEXT` and `UPDATETEXT`
    // change text/image columns.
    "SETUSER",
    "WRITETEXT",
    "UPDATETEXT",
];

/// Function families with the same kind of lasting effect: `pg_stat_reset_shared`,
/// `pg_create_logical_replication_slot`, `pg_replication_origin_drop`, ...
const WRITE_PREFIXES: &[&str] = &[
    "PG_STAT_RESET",
    "PG_STAT_STATEMENTS_RESET",
    "PG_CREATE_PHYSICAL_REPLICATION_SLOT",
    "PG_CREATE_LOGICAL_REPLICATION_SLOT",
    "PG_COPY_PHYSICAL_REPLICATION_SLOT",
    "PG_COPY_LOGICAL_REPLICATION_SLOT",
    "PG_DROP_REPLICATION_SLOT",
    "PG_REPLICATION_SLOT_ADVANCE",
    "PG_REPLICATION_ORIGIN_",
];

/// True when the statement only reads data: it starts with a reading keyword and
/// contains no write keyword outside strings, quoted identifiers and comments, read the
/// way `kind`'s database reads them (`sql_lex`, which also covers SQL Server's nested
/// comments and `]]` escapes, and MySQL's `/*! … */`, whose inside runs as SQL).
///
/// The database enforces this too, where it can: read-only transactions on Postgres and
/// MySQL, a read-only connection on SQLite, and an always-rolled-back transaction on
/// SQL Server (which has no read-only mode, so there this check matters most).
pub fn is_read_only(sql: &str, kind: Kind) -> bool {
    // Some settings change where strings end (MySQL's NO_BACKSLASH_ESCAPES, Postgres'
    // standard_conforming_strings), so the statement must read as read-only every way.
    lex(sql, crate::safety::never_send::dialect(kind))
        .iter()
        .all(|l| reads_only(sql, &l.toks))
}

fn reads_only(sql: &str, toks: &[Tok]) -> bool {
    let first = toks.iter().find(|t| !(t.is_punct('(') || t.is_punct(';')));
    let reads = match first {
        Some(Tok::Word(w)) => match w.to_ascii_uppercase().as_str() {
            "SELECT" | "WITH" | "EXPLAIN" | "SHOW" | "DESCRIBE" | "DESC" | "VALUES" | "TABLE" => {
                true
            }
            // `PRAGMA x = y` changes settings; `PRAGMA x` / `PRAGMA x(y)` reads them.
            "PRAGMA" => !sql.contains('='),
            _ => false,
        },
        _ => false,
    };
    if !reads {
        return false;
    }
    let words: Vec<&str> = toks
        .iter()
        .filter_map(|t| {
            if let Tok::Word(w) = t {
                Some(w.as_str())
            } else {
                None
            }
        })
        .collect();
    let is = |w: &str, k: &str| w.eq_ignore_ascii_case(k);
    // SQL Server's `NEXT VALUE FOR seq` advances a sequence, and a rollback doesn't undo it.
    let next_value = words
        .windows(3)
        .any(|w| is(w[0], "NEXT") && is(w[1], "VALUE") && is(w[2], "FOR"));
    let writes = |i: usize, w: &str| {
        // MySQL's `SHOW CREATE TABLE t` only shows the statement.
        let shown = is(w, "CREATE") && i > 0 && is(words[i - 1], "SHOW");
        let upper = w.to_ascii_uppercase();
        !shown
            && (WRITE_KEYWORDS.contains(&upper.as_str())
                || WRITE_PREFIXES.iter().any(|p| upper.starts_with(p)))
    };
    !next_value && !words.iter().enumerate().any(|(i, w)| writes(i, w))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [Kind; 4] = [Kind::Postgres, Kind::Mysql, Kind::Sqlite, Kind::Mssql];

    fn ro_everywhere(sql: &str) -> bool {
        ALL.iter().all(|&k| is_read_only(sql, k))
    }

    fn rw_everywhere(sql: &str) -> bool {
        ALL.iter().all(|&k| !is_read_only(sql, k))
    }

    #[test]
    fn classifies_read_only_statements() {
        assert!(ro_everywhere("select 1"));
        assert!(ro_everywhere("  -- top customers\nSELECT * FROM customers"));
        assert!(ro_everywhere(
            "/* c */ WITH x AS (SELECT 1) SELECT * FROM x"
        ));
        assert!(ro_everywhere("(SELECT 1) UNION (SELECT 2)"));
        assert!(ro_everywhere("PRAGMA table_info(orders)"));
        assert!(rw_everywhere("PRAGMA journal_mode = WAL"));
        assert!(rw_everywhere("DELETE FROM orders"));
        assert!(rw_everywhere(
            "-- select\nUPDATE orders SET status = 'paid'"
        ));
        assert!(rw_everywhere("DROP TABLE x"));
    }

    #[test]
    fn write_keywords_anywhere_need_confirmation() {
        assert!(rw_everywhere("SELECT 1; DELETE FROM orders"));
        assert!(rw_everywhere("SELECT * INTO backup_orders FROM orders"));
        assert!(rw_everywhere(
            "WITH gone AS (DELETE FROM t RETURNING *) SELECT * FROM gone"
        ));
        assert!(rw_everywhere("SELECT 1\nEXEC sp_configure"));
        assert!(rw_everywhere("select 1; commit; drop table x"));
        assert!(rw_everywhere("SELECT 1; USE master"));
        assert!(rw_everywhere("SELECT 1\nSET ROLE admin"));
    }

    #[test]
    fn an_explanation_commented_out_before_the_query_is_fine() {
        assert!(ro_everywhere(
            "-- The user's email is on the Person table, so join those.\n\nSELECT TOP 100 email FROM dbo.Person"
        ));
        // Prose that isn't commented out is not SQL: it must not count as read-only.
        assert!(rw_everywhere(
            "The email is on Person.\nSELECT email FROM dbo.Person"
        ));
    }

    #[test]
    fn write_words_inside_strings_identifiers_and_comments_are_fine() {
        assert!(ro_everywhere("SELECT * FROM audit WHERE action = 'DELETE'"));
        assert!(is_read_only(
            "SELECT [Update], \"insert\", `drop` FROM t",
            Kind::Sqlite
        ));
        assert!(is_read_only(
            "SELECT [Update], \"insert\" FROM t",
            Kind::Mssql
        ));
        assert!(is_read_only(
            "SELECT `drop`, \"insert\" FROM t",
            Kind::Mysql
        ));
        assert!(is_read_only("SELECT \"insert\" FROM t", Kind::Postgres));
        assert!(ro_everywhere(
            "SELECT updated_at, created_by FROM t -- delete later\n/* drop */"
        ));
        assert!(ro_everywhere(
            "SELECT REPLACE(name, 'a', 'b') FROM t WHERE note = 'it''s DELETE'"
        ));
    }

    #[test]
    fn mysql_backslash_escapes_and_hash_comments_cannot_hide_writes() {
        // The audit's two bypasses.
        assert!(!is_read_only(
            r"SELECT '\'' INTO OUTFILE '/tmp/x' -- '",
            Kind::Mysql
        ));
        assert!(!is_read_only(
            "SELECT 1 # [\nINTO OUTFILE '/tmp/x' -- ]",
            Kind::Mysql
        ));
        // With NO_BACKSLASH_ESCAPES, `'\'` is a whole string.
        assert!(!is_read_only(
            r"SELECT '\' INTO OUTFILE '/tmp/x' -- '",
            Kind::Mysql
        ));
        assert!(!is_read_only(
            r#"SELECT "\"" INTO OUTFILE '/tmp/x' -- ""#,
            Kind::Mysql
        ));
        // `/*! ... */` runs as SQL on MySQL, and `--` without a space isn't a comment.
        assert!(!is_read_only(
            "SELECT 1 /*! INTO OUTFILE '/tmp/x' */",
            Kind::Mysql
        ));
        assert!(!is_read_only(
            "SELECT 1 /*!50000 INTO DUMPFILE '/tmp/x' */",
            Kind::Mysql
        ));
        assert!(!is_read_only(
            "SELECT 1 /*M! INTO OUTFILE '/tmp/x' */",
            Kind::Mysql
        ));
        assert!(!is_read_only(
            "SELECT 1--1 INTO OUTFILE '/tmp/x'",
            Kind::Mysql
        ));
        // Brackets don't quote in MySQL.
        assert!(!is_read_only("SELECT [delete] FROM t", Kind::Mysql));
    }

    #[test]
    fn postgres_escape_and_dollar_strings_cannot_hide_writes() {
        assert!(!is_read_only(
            r"SELECT E'\'' ; DROP TABLE t -- '",
            Kind::Postgres
        ));
        assert!(!is_read_only(
            r"SELECT e'\''; DROP TABLE t -- '",
            Kind::Postgres
        ));
        // With standard_conforming_strings off, plain strings take backslash escapes too.
        assert!(!is_read_only(
            r"SELECT '\''; DROP TABLE t -- '",
            Kind::Postgres
        ));
        assert!(!is_read_only(
            "SELECT $$'$$; DROP TABLE t; -- '",
            Kind::Postgres
        ));
        assert!(!is_read_only(
            "SELECT $q$ ' $q$; DELETE FROM t -- '",
            Kind::Postgres
        ));
        // A backtick is an operator character in Postgres, not a quote.
        assert!(!is_read_only("SELECT 1 ` DELETE FROM t `", Kind::Postgres));
    }

    #[test]
    fn file_writes_new_tables_and_side_effects_need_confirmation() {
        assert!(rw_everywhere("SELECT * FROM t INTO OUTFILE '/tmp/x'"));
        assert!(rw_everywhere("SELECT * FROM t INTO DUMPFILE '/tmp/x'"));
        assert!(rw_everywhere("SELECT * INTO TEMP t2 FROM t"));
        assert!(rw_everywhere("SELECT * INTO #t2 FROM t"));
        assert!(rw_everywhere("COPY t TO '/tmp/x'"));
        assert!(rw_everywhere("COPY (SELECT * FROM t) TO PROGRAM 'sh'"));
        assert!(rw_everywhere("SELECT pg_terminate_backend(123)"));
        assert!(rw_everywhere("SELECT nextval('orders_id_seq')"));
        assert!(rw_everywhere(
            "SELECT * FROM dblink('dbname=x', 'DELETE FROM t') AS r(n int)"
        ));
        assert!(rw_everywhere("SELECT lo_export(16409, '/tmp/x')"));
        assert!(rw_everywhere("SELECT load_extension('evil')"));
        assert!(rw_everywhere(
            "SELECT * FROM OPENDATASOURCE('SQLNCLI', 'Data Source=x').db.dbo.t"
        ));
        assert!(rw_everywhere("SELECT NEXT VALUE FOR dbo.order_seq"));
        // Reading a file is a read.
        assert!(is_read_only("SELECT LOAD_FILE('/etc/hosts')", Kind::Mysql));
    }

    #[test]
    fn effects_a_rollback_does_not_undo_need_confirmation() {
        for sql in [
            "SELECT pg_create_physical_replication_slot('s')",
            "SELECT * FROM pg_create_logical_replication_slot('s', 'pgoutput')",
            "SELECT pg_drop_replication_slot('s')",
            "SELECT pg_stat_reset()",
            "SELECT pg_stat_reset_shared('bgwriter')",
            "SELECT pg_stat_statements_reset()",
            "SELECT pg_advisory_lock(42)",
            "SELECT pg_try_advisory_lock_shared(42)",
            "SELECT pg_replication_origin_create('o')",
            "SELECT pg_create_restore_point('r')",
            "SELECT GET_LOCK('sayql', 10)",
        ] {
            assert!(rw_everywhere(sql), "{sql}");
        }
        // Transaction-level advisory locks end with the read-only transaction.
        assert!(ro_everywhere("SELECT pg_try_advisory_xact_lock(42)"));
        assert!(ro_everywhere("SELECT * FROM pg_stat_activity"));
        assert!(ro_everywhere("SELECT slot_name FROM pg_replication_slots"));
    }

    #[test]
    fn tsql_nested_comments_and_bracket_escapes_cannot_hide_writes() {
        // The 2026-10-29 audit: the old lexer ended a comment at the first `*/`, so the
        // DELETE and COMMIT below were read as part of a string.
        assert!(!is_read_only(
            "SELECT 1 /* /* */ '*/ DELETE FROM x; COMMIT --'",
            Kind::Mssql
        ));
        assert!(!is_read_only(
            "SELECT [a]]' ] ; DELETE FROM x --'",
            Kind::Mssql
        ));
        // Postgres comments nest too.
        assert!(!is_read_only(
            "SELECT 1 /* /* */ '*/ DELETE FROM x; -- '",
            Kind::Postgres
        ));
        // Backticks quote nothing on SQL Server either.
        assert!(!is_read_only("SELECT 1 ` DELETE FROM t `", Kind::Mssql));
        // Ordinary nested comments and escaped brackets still read fine.
        assert!(is_read_only(
            "/* outer /* inner */ still a comment */ SELECT [a]]b] FROM t",
            Kind::Mssql
        ));
    }

    #[test]
    fn tsql_setuser_and_text_writes_need_confirmation() {
        for sql in [
            "SELECT 1; SETUSER 'dbo'",
            "SELECT 1\nSETUSER",
            "SELECT 1; WRITETEXT t.notes @ptr 'x'",
            "SELECT 1; UPDATETEXT t.notes @ptr 0 NULL 'x'",
        ] {
            assert!(rw_everywhere(sql), "{sql}");
        }
        assert!(is_read_only("SELECT setuser_id FROM t", Kind::Mssql));
    }

    #[test]
    fn show_create_is_a_read() {
        assert!(is_read_only("SHOW CREATE TABLE `orders`", Kind::Mysql));
        assert!(is_read_only("show create view v", Kind::Mysql));
        assert!(!is_read_only(
            "SHOW TABLES; CREATE TABLE x (a int)",
            Kind::Mysql
        ));
        assert!(!is_read_only(
            "SELECT 1; CREATE TABLE x (a int)",
            Kind::Mysql
        ));
    }

    #[test]
    fn normal_queries_stay_read_only_in_each_dialect() {
        let mysql = [
            r#"SELECT * FROM t WHERE name = 'O\'Brien' AND note = "say \"hi\"""#,
            "SELECT id FROM t # delete later\nWHERE a = 1",
            "# top customers\nSELECT * FROM customers",
            "SELECT 5--1",
            "SELECT `order`, a$b FROM `my db`.t LIMIT 10",
            "SELECT /*+ MAX_EXECUTION_TIME(1000) */ * FROM t",
            r"SELECT 'C:\\path\\update' AS p",
        ];
        for sql in mysql {
            assert!(is_read_only(sql, Kind::Mysql), "{sql}");
        }
        let postgres = [
            r"SELECT E'it\'s', E'line\nnext' FROM t",
            "SELECT $$it's DELETE$$, $fn$ drop $fn$",
            "SELECT a$b, arr[1], arr[1:2] FROM t WHERE c = $1 AND d = $2",
            "SELECT 'x' || '[' || ']' FROM t",
            "SELECT 5 # 3, x::text[] FROM t",
            "WITH r AS (SELECT * FROM orders) SELECT count(*) FROM r",
        ];
        for sql in postgres {
            assert!(is_read_only(sql, Kind::Postgres), "{sql}");
        }
        let mssql = [
            "SELECT TOP 10 [Order Date], [Delete] FROM dbo.[Orders] WHERE n = N'O''Brien'",
            "SELECT next_value FROM t",
            "SELECT 'a\\' AS p, 1 FROM t",
        ];
        for sql in mssql {
            assert!(is_read_only(sql, Kind::Mssql), "{sql}");
        }
        let sqlite = [
            "SELECT [update], `drop`, \"insert\" FROM t WHERE x = 'a\\'",
            "PRAGMA table_info('t')",
        ];
        for sql in sqlite {
            assert!(is_read_only(sql, Kind::Sqlite), "{sql}");
        }
    }
}
