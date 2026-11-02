// SQL text helpers that depend on the schema/dialect. Shared by the fast path and table previews.
import type { Dialect, Table } from "../../api";

const RESERVED = new Set([
  "user",
  "order",
  "group",
  "table",
  "select",
  "where",
  "from",
  "limit",
  "offset",
  "desc",
  "asc",
  "key",
  "index",
  "check",
  "default",
  "column",
  "values",
  "references",
  "primary",
  "case",
  "end",
]);

/** The schema that needs no qualifier in each dialect. */
const DEFAULT_SCHEMA: Partial<Record<Dialect, string>> = { PostgreSQL: "public" };

/** The name shown in the UI and used in prompts: `schema.table`, except for Postgres `public`. */
export function qualifiedName(t: Table): string {
  return t.schema && t.schema !== "public" ? `${t.schema}.${t.name}` : t.name;
}

/**
 * Quotes an identifier only when it needs it: symbols, reserved words, and — in Postgres,
 * which folds unquoted names to lowercase — capital letters.
 */
export function quoteIdent(ident: string, dialect: Dialect): string {
  const plain = dialect === "PostgreSQL" ? /^[a-z_][a-z0-9_]*$/ : /^[a-z_][a-z0-9_]*$/i;
  if (plain.test(ident) && !RESERVED.has(ident.toLowerCase())) return ident;
  switch (dialect) {
    case "MySQL":
      return `\`${ident.replace(/`/g, "``")}\``;
    case "SQL Server":
      return `[${ident.replace(/]/g, "]]")}]`;
    default:
      return `"${ident.replace(/"/g, '""')}"`;
  }
}

/** A table reference usable in FROM. */
export function tableRef(t: Table, dialect: Dialect): string {
  const name = quoteIdent(t.name, dialect);
  const qualify = t.schema && t.schema !== DEFAULT_SCHEMA[dialect] && dialect !== "MySQL";
  return qualify ? `${quoteIdent(t.schema!, dialect)}.${name}` : name;
}

/** `SELECT * FROM t [ORDER BY ...]` limited to `n` rows, in the dialect's syntax. */
export function selectRowsSql(t: Table, dialect: Dialect, n: number, orderBy?: string): string {
  const order = orderBy ? `\nORDER BY ${quoteIdent(orderBy, dialect)} DESC` : "";
  return dialect === "SQL Server"
    ? `SELECT TOP ${n} *\nFROM ${tableRef(t, dialect)}${order};`
    : `SELECT *\nFROM ${tableRef(t, dialect)}${order}\nLIMIT ${n};`;
}

export function selectAllSql(t: Table, dialect: Dialect): string {
  return selectRowsSql(t, dialect, 100);
}

export function countRowsSql(t: Table, dialect: Dialect): string {
  return `SELECT COUNT(*) AS row_count\nFROM ${tableRef(t, dialect)};`;
}

/** Statement templates offered by the navigator's "Generate SQL" menu. */
export type SqlTemplate = "select" | "insert" | "update" | "delete";

/**
 * A statement to edit before running. Values are NULL placeholders with the column's type
 * in a comment, and UPDATE/DELETE come with a WHERE that matches no rows until it's filled
 * in, so running a template unchanged never changes data.
 */
export function templateSql(t: Table, dialect: Dialect, kind: SqlTemplate): string {
  const q = (c: string) => quoteIdent(c, dialect);
  const from = tableRef(t, dialect);
  const cols = t.columns;
  const keys = cols.filter((c) => c.primary_key);
  /** `lines` joined with commas, each followed by its own comment. */
  const list = (lines: string[], comments: string[]) =>
    lines
      .map(
        (l, i) =>
          `  ${l}${i < lines.length - 1 ? "," : ""}${comments[i] ? ` -- ${comments[i]}` : ""}`,
      )
      .join("\n");
  const where = (verb: string) =>
    keys.length
      ? `WHERE ${keys.map((k) => `${q(k.name)} = NULL`).join(" AND ")}; -- the row to ${verb}`
      : `WHERE 1 = 0; -- replace with the rows to ${verb}: ${t.name} has no primary key`;

  switch (kind) {
    case "select": {
      const top = dialect === "SQL Server" ? " TOP 100" : "";
      const limit = dialect === "SQL Server" ? "" : "\nLIMIT 100";
      return `SELECT${top}\n${list(
        cols.map((c) => q(c.name)),
        [],
      )}\nFROM ${from}${limit};`;
    }
    case "insert":
      return (
        `INSERT INTO ${from} (\n${list(
          cols.map((c) => q(c.name)),
          [],
        )}\n) VALUES (\n` +
        `${list(
          cols.map(() => "NULL"),
          cols.map((c) => `${c.name}: ${c.data_type}`),
        )}\n);`
      );
    case "update": {
      const set = cols.filter((c) => !c.primary_key);
      const targets = set.length ? set : cols;
      return `UPDATE ${from}\nSET\n${list(
        targets.map((c) => `${q(c.name)} = NULL`),
        targets.map((c) => c.data_type),
      )}\n${where("change")}`;
    }
    case "delete":
      return `DELETE FROM ${from}\n${where("delete")}`;
  }
}
