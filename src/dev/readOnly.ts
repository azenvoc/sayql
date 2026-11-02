// Development only: a copy of the backend's read-only check, so the browser preview asks
// for write confirmation on the same statements the app does. The source of truth is
// `is_read_only` in src-tauri/src/db/classify.rs; keep this in step with it (readOnly.test.ts
// repeats its test cases).

import type { DatabaseKind } from "../api";

/** Same list as `WRITE_KEYWORDS` in classify.rs. */
const WRITE_KEYWORDS = new Set([
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
  "SETUSER",
  "WRITETEXT",
  "UPDATETEXT",
]);

/** Same list as `WRITE_PREFIXES` in classify.rs. */
const WRITE_PREFIXES = [
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

const READ_STARTS = new Set([
  "SELECT",
  "WITH",
  "EXPLAIN",
  "SHOW",
  "DESCRIBE",
  "DESC",
  "VALUES",
  "TABLE",
]);

/** A bare word, or any other character outside strings and comments. */
type Token = { word: string } | { sym: string };

/**
 * True when the statement only reads data: it starts with a reading keyword and contains no
 * write keyword outside strings, quoted names and comments, read the way `kind`'s database
 * reads them. Whether `\` escapes a quote depends on server settings in MySQL and Postgres, so
 * there the statement must read as read-only both ways.
 */
export function isReadOnly(sql: string, kind: DatabaseKind = "sqlite"): boolean {
  const ways = kind === "mysql" || kind === "postgres" ? [true, false] : [false];
  return ways.every((backslash) => readsOnly(sql, tokens(sql, kind, backslash)));
}

function readsOnly(sql: string, toks: Token[]): boolean {
  const first = toks.find((t) => !("sym" in t && (t.sym === "(" || t.sym === ";")));
  if (!first || !("word" in first)) return false;
  const start = first.word.toUpperCase();
  // `PRAGMA x = y` changes settings; `PRAGMA x` / `PRAGMA x(y)` reads them.
  const reads = READ_STARTS.has(start) || (start === "PRAGMA" && !sql.includes("="));
  if (!reads) return false;
  const words = toks.flatMap((t) => ("word" in t ? [t.word.toUpperCase()] : []));
  // SQL Server's `NEXT VALUE FOR seq` advances a sequence, and a rollback doesn't undo it.
  if (words.some((w, i) => w === "NEXT" && words[i + 1] === "VALUE" && words[i + 2] === "FOR"))
    return false;
  return !words.some((w, i) => {
    // MySQL's `SHOW CREATE TABLE t` only shows the statement.
    if (w === "CREATE" && words[i - 1] === "SHOW") return false;
    return WRITE_KEYWORDS.has(w) || WRITE_PREFIXES.some((p) => w.startsWith(p));
  });
}

/** The statement's words (and `null` for other characters), skipping strings, quoted names and comments. */
function tokens(sql: string, kind: DatabaseKind, backslash: boolean): Token[] {
  const mysql = kind === "mysql";
  const postgres = kind === "postgres";
  const out: Token[] = [];
  // Index just past the next `end` at or after `from`, or the end of the text.
  const skipTo = (from: number, end: string) => {
    const p = sql.indexOf(end, from);
    return p < 0 ? sql.length : p + end.length;
  };
  // Index just past the closing `q`, where `\` escapes the next character and `qq` is a literal quote.
  const skipEscaped = (from: number, q: string) => {
    let j = from;
    while (j < sql.length) {
      if (sql[j] === "\\") j += 2;
      else if (sql[j] === q && sql[j + 1] === q) j += 2;
      else if (sql[j] === q) return j + 1;
      else j++;
    }
    return sql.length;
  };
  // MySQL and Postgres allow `$` inside names (`a$b`).
  const wordChar = (c: string | undefined) =>
    !!c && (/[A-Za-z0-9_]/.test(c) || (c === "$" && (mysql || postgres)));
  let i = 0;
  while (i < sql.length) {
    const c = sql[i];
    const next = sql[i + 1];
    if (c === "'") {
      // Postgres E'...' strings always take backslash escapes.
      const eString =
        postgres && i > 0 && /[eE]/.test(sql[i - 1]) && (i === 1 || !wordChar(sql[i - 2]));
      i = backslash || eString ? skipEscaped(i + 1, "'") : skipTo(i + 1, "'");
    } else if (c === '"' && mysql && backslash) i = skipEscaped(i + 1, '"');
    else if (c === '"') i = skipTo(i + 1, '"');
    else if (c === "`" && (mysql || kind === "sqlite")) i = skipTo(i + 1, "`");
    else if (c === "[" && !mysql && !postgres) i = skipTo(i + 1, "]");
    else if (c === "$" && postgres) {
      const tag = sql.slice(i).match(/^\$(?:[A-Za-z_][A-Za-z0-9_]*)?\$/)?.[0];
      i = tag ? skipTo(i + tag.length, tag) : i + 1;
      if (!tag) out.push({ sym: c });
    } else if (
      c === "-" &&
      next === "-" &&
      (!mysql || i + 2 >= sql.length || /\s/.test(sql[i + 2]))
    )
      i = skipTo(i, "\n");
    else if (c === "#" && mysql) i = skipTo(i, "\n");
    else if (c === "/" && next === "*") {
      // MySQL runs the inside of `/*! ... */` (and MariaDB `/*M! ... */`) as SQL.
      const marker = sql.startsWith("!", i + 2) ? 1 : sql.startsWith("M!", i + 2) ? 2 : 0;
      if (mysql && marker > 0) {
        i += 2 + marker;
        while (i < sql.length && /[0-9]/.test(sql[i])) i++;
      } else i = skipTo(i + 2, "*/");
    } else if (/[A-Za-z_]/.test(c)) {
      const start = i;
      while (i < sql.length && wordChar(sql[i])) i++;
      out.push({ word: sql.slice(start, i) });
    } else if (/\s/.test(c)) i++;
    else {
      out.push({ sym: c });
      i++;
    }
  }
  return out;
}
