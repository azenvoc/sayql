import type { ConnectionConfig, DatabaseKind } from "../../api";
import { DATABASE_KINDS } from "./kinds";

/** What a pasted connection string fills in. The password is kept apart: it only goes to the keychain on save. */
export interface ParsedConnectionString {
  fields: Partial<ConnectionConfig> & { kind: DatabaseKind };
  password: string | null;
}

const URL_KINDS: Record<string, DatabaseKind> = {
  postgres: "postgres",
  postgresql: "postgres",
  mysql: "mysql",
  mariadb: "mysql",
  sqlserver: "mssql",
  mssql: "mssql",
  sqlite: "sqlite",
  file: "sqlite",
};

const TRUE =
  /^(true|yes|1|on|mandatory|strict|require|required|verify[-_]?ca|verify[-_]?full|verify[-_]?identity)$/i;
const FALSE = /^(false|no|0|off|optional|disable|disabled|prefer|preferred|allow)$/i;

/**
 * Reads the connection strings people copy from apps and cloud consoles: URLs (`postgres://…`, `mysql://…`,
 * `sqlserver://…`, `sqlite:…`), JDBC (`jdbc:postgresql://…`, `jdbc:sqlserver://…;databaseName=…`),
 * key=value with semicolons (SQL Server/Azure, Npgsql, MySQL Connector) and libpq's `host=… dbname=…`.
 * Returns null for anything else.
 */
export function parseConnectionString(raw: string): ParsedConnectionString | null {
  const text = raw.trim().replace(/^jdbc:/i, "");
  if (!text) return null;
  const scheme = /^([a-z][a-z0-9+.-]*):/i.exec(text)?.[1].toLowerCase();
  if (scheme && URL_KINDS[scheme]) {
    const kind = URL_KINDS[scheme];
    if (kind === "sqlite") return sqlite(text);
    if (kind === "mssql") return sqlServerUrl(text);
    return url(text, kind);
  }
  if (text.includes(";") || /^\s*\w[\w ]*=/.test(text)) return keyValue(text);
  return null;
}

function finish(
  kind: DatabaseKind,
  f: Partial<ConnectionConfig>,
  password: string | null,
  ssl?: boolean,
): ParsedConnectionString {
  const host = f.host ?? "";
  const azure = /\.database\.windows\.net$/i.test(host);
  return {
    fields: {
      ...f,
      kind,
      host,
      port: f.port ?? DATABASE_KINDS[kind].defaultPort,
      ssl: ssl ?? (azure || DATABASE_KINDS[kind].defaultSsl),
      trustCert: f.trustCert ?? false,
    },
    password,
  };
}

function decode(s: string): string {
  try {
    return decodeURIComponent(s);
  } catch {
    return s;
  }
}

function port(s: string | undefined | null): number | null {
  const n = Number((s ?? "").trim());
  return Number.isInteger(n) && n > 0 && n < 65536 ? n : null;
}

function flag(v: string | undefined | null): boolean | undefined {
  if (v == null) return undefined;
  if (TRUE.test(v.trim())) return true;
  if (FALSE.test(v.trim())) return false;
  return undefined;
}

/** postgres://user:pass@host:5432/db?sslmode=require, and the same for mysql://. */
function url(text: string, kind: DatabaseKind): ParsedConnectionString | null {
  // A second host (failover lists) would make this an invalid URL; keep the first.
  const firstHost = text.replace(/@([^/?,]+),[^/?]*/, "@$1");
  let u: URL;
  try {
    u = new URL(firstHost);
  } catch {
    return null;
  }
  const q = u.searchParams;
  const host = decode(u.hostname || q.get("host") || "");
  if (!host) return null;
  const ssl = flag(q.get("sslmode") ?? q.get("ssl-mode") ?? q.get("ssl") ?? q.get("tls"));
  return finish(
    kind,
    {
      host,
      port: port(u.port),
      database: decode(u.pathname.replace(/^\//, "") || q.get("dbname") || q.get("database") || ""),
      user: decode(u.username || q.get("user") || ""),
    },
    u.password ? decode(u.password) : q.get("password"),
    ssl,
  );
}

/** sqlserver://host\instance:1433;databaseName=x;user=y;password=z;encrypt=true (JDBC), or sqlserver://u:p@host/db. */
function sqlServerUrl(text: string): ParsedConnectionString | null {
  const body = text.replace(/^[a-z]+:\/\//i, "");
  if (!body.includes(";") && body.includes("@"))
    return url(text.replace(/^[a-z]+:/i, "postgres:"), "mssql");
  const [hostPart, ...rest] = body.split(";");
  const kv = pairs(rest.join(";"), ";");
  const [hostAndInstance, p] = hostPart.split(":");
  const parsed = keyValueFields(kv);
  return finish(
    "mssql",
    {
      ...parsed.fields,
      host: parsed.fields.host || hostAndInstance.split("\\")[0],
      port: parsed.fields.port ?? port(p),
    },
    parsed.password,
    parsed.ssl,
  );
}

/** sqlite:///path/to/db.sqlite, sqlite:/path, file:/path. */
function sqlite(text: string): ParsedConnectionString | null {
  const path = decode(text.replace(/^(sqlite|file):(\/\/)?/i, "").split("?")[0]);
  if (!path) return null;
  return {
    fields: { kind: "sqlite", file: path, host: "", port: null, ssl: false, trustCert: false },
    password: null,
  };
}

function pairs(text: string, sep: string): Map<string, string> {
  const map = new Map<string, string>();
  // libpq quotes values with spaces: password='a b'. Semicolon strings may brace them: Password={a;b}.
  const parts =
    sep === " "
      ? (text.match(/[^\s=]+\s*=\s*(?:'(?:[^'\\]|\\.)*'|\S*)/g) ?? [])
      : (text.match(/[^;]*?=\s*(?:\{[^}]*\}|[^;]*)/g) ?? []);
  for (const part of parts.map((p) => p.replace(/^;/, ""))) {
    const i = part.indexOf("=");
    if (i <= 0) continue;
    const key = part.slice(0, i).trim().toLowerCase().replace(/[\s_]/g, "");
    let value = part.slice(i + 1).trim();
    if (/^(['"{]).*(['"}])$/.test(value)) value = value.slice(1, -1);
    if (key) map.set(key, value);
  }
  return map;
}

const HOST = ["server", "datasource", "address", "addr", "networkaddress", "host", "hostname"];
const DB = ["database", "initialcatalog", "databasename", "dbname"];
const USER = ["userid", "uid", "user", "username"];
const PASSWORD = ["password", "pwd"];

function get(kv: Map<string, string>, keys: string[]): string | undefined {
  for (const k of keys) if (kv.has(k)) return kv.get(k);
  return undefined;
}

function keyValueFields(kv: Map<string, string>) {
  let host = get(kv, HOST) ?? "";
  let p = port(kv.get("port"));
  // SQL Server writes "tcp:host,1433"; MySQL sometimes "host:3306".
  host = host.replace(/^tcp:/i, "");
  const comma = host.match(/^(.*),(\d+)$/);
  if (comma) [host, p] = [comma[1], p ?? port(comma[2])];
  host = host.split("\\")[0];
  return {
    fields: {
      host,
      port: p,
      database: get(kv, DB) ?? "",
      user: get(kv, USER) ?? "",
      trustCert: flag(kv.get("trustservercertificate")) ?? false,
    },
    password: get(kv, PASSWORD) ?? null,
    ssl: flag(kv.get("encrypt") ?? kv.get("sslmode") ?? kv.get("ssl")),
  };
}

/** Semicolon pairs (SQL Server, Npgsql, MySQL Connector) or libpq's space-separated host=… dbname=…. */
function keyValue(text: string): ParsedConnectionString | null {
  const semicolons = text.includes(";");
  const kv = pairs(text, semicolons ? ";" : " ");
  const parsed = keyValueFields(kv);
  if (!parsed.fields.host && !parsed.fields.database) return null;
  const kind = guessKind(kv, semicolons, parsed.fields.port);
  return finish(kind, parsed.fields, parsed.password, parsed.ssl);
}

function guessKind(kv: Map<string, string>, semicolons: boolean, p: number | null): DatabaseKind {
  if (!semicolons || kv.has("dbname")) return "postgres"; // libpq
  if (p === 5432) return "postgres";
  if (p === 3306) return "mysql";
  if (
    [
      "initialcatalog",
      "datasource",
      "trustservercertificate",
      "encrypt",
      "integratedsecurity",
    ].some((k) => kv.has(k))
  )
    return "mssql";
  if (/^tcp:/i.test(get(kv, HOST) ?? "") || /\.database\.windows\.net/i.test(get(kv, HOST) ?? ""))
    return "mssql";
  if (kv.has("host") || kv.has("username")) return "postgres"; // Npgsql
  if (kv.has("uid") || kv.has("pwd") || kv.has("sslmode")) return "mysql"; // MySQL Connector/NET
  return "mssql";
}
