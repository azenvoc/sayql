// Splitting SQL text into statements, for running a selection and running a script.

import type { Dialect } from "../api";

export interface Range {
  start: number;
  end: number;
}

/**
 * The statements in `sql`, as ranges split at `;` outside strings, quoted names and
 * comments, read the way `dialect`'s database reads them (MySQL's `#` comments and `\'`
 * escapes, Postgres' `E'...'` and `$tag$` strings). Each range ends after its `;` (or at the
 * end of the text) and starts at its first SQL (after blank space and comments). A part that
 * is only comments isn't a statement.
 */
export function statementRanges(sql: string, dialect?: Dialect): Range[] {
  const mysql = dialect === "MySQL";
  const postgres = dialect === "PostgreSQL";
  const out: Range[] = [];
  let start = 0;
  let i = 0;
  const push = (end: number) => {
    const text = sql.slice(start, end);
    const lead = text.length - stripLeading(text).length;
    if (stripLeading(text).replace(/^;$/, "")) out.push({ start: start + lead, end });
    start = end;
  };
  /** Index just past the closing `q` of a run where `\` escapes the next character. */
  const skipEscaped = (from: number, q: string) => {
    for (let j = from; j < sql.length; j++) {
      if (sql[j] === "\\") j++;
      else if (sql[j] === q) return j + 1;
    }
    return sql.length;
  };
  while (i < sql.length) {
    const c = sql[i];
    const next = sql[i + 1];
    // MySQL needs blank space after `--` for a comment (`1--1` is arithmetic), and also has `#`.
    if (
      (c === "-" && next === "-" && (!mysql || !next || /\s/.test(sql[i + 2] ?? " "))) ||
      (c === "#" && mysql)
    ) {
      const nl = sql.indexOf("\n", i);
      i = nl < 0 ? sql.length : nl + 1;
    } else if (c === "/" && next === "*") {
      const close = sql.indexOf("*/", i + 2);
      i = close < 0 ? sql.length : close + 2;
    } else if ((c === "'" || c === '"') && mysql) {
      i = skipEscaped(i + 1, c);
    } else if (
      c === "'" &&
      postgres &&
      /[eE]/.test(sql[i - 1] ?? "") &&
      !/[A-Za-z0-9_]/.test(sql[i - 2] ?? "")
    ) {
      i = skipEscaped(i + 1, c);
    } else if (c === "'" || c === '"' || c === "`") {
      // A doubled quote inside is an escaped quote; the loop just reopens the string.
      const close = sql.indexOf(c, i + 1);
      i = close < 0 ? sql.length : close + 1;
    } else if (c === "[") {
      const close = sql.indexOf("]", i + 1);
      i = close < 0 ? sql.length : close + 1;
    } else if (c === "$") {
      // PostgreSQL dollar quoting: $$…$$ or $tag$…$tag$.
      const tag = sql.slice(i).match(/^\$(?:[A-Za-z_][A-Za-z0-9_]*)?\$/)?.[0];
      if (tag) {
        const close = sql.indexOf(tag, i + tag.length);
        i = close < 0 ? sql.length : close + tag.length;
      } else i++;
    } else if (c === ";") {
      i++;
      push(i);
    } else i++;
  }
  push(sql.length);
  return out;
}

/** `sql` without the blank space and comments it starts with. */
export function stripLeading(sql: string): string {
  let s = sql.trimStart();
  for (let prev = ""; prev !== s;) {
    prev = s;
    s = s
      .replace(/^--[^\n]*(\n|$)/, "")
      .replace(/^\/\*[\s\S]*?\*\//, "")
      .trimStart();
  }
  return s;
}

/**
 * The statements to run, in order, as separate requests. Procedure, function and trigger
 * definitions keep semicolons in their bodies, so text with one of those is sent whole.
 */
export function splitForRun(sql: string, dialect?: Dialect): string[] {
  if (/\bcreate\s+(or\s+(replace|alter)\s+)?(procedure|proc|function|trigger)\b/i.test(sql))
    return sql.trim() ? [sql.trim()] : [];
  return statementRanges(sql, dialect).map((r) => sql.slice(r.start, r.end).trim());
}
