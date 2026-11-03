import type { Schema, Table } from "../../api";

/**
 * Never-send entries are stored as written: `column` (every table) or `table.column`.
 * SayQL matches them by the column name alone (src-tauri/src/safety/mask/mod.rs `Masker::new`
 * keeps the part after the last dot), so `customers.email` also keeps out `email` elsewhere.
 */
export interface NeverSendEntry {
  /** As stored. */
  raw: string;
  /** `null` for a bare column name, which applies in every table. */
  table: string | null;
  column: string;
}

export function parseEntry(raw: string): NeverSendEntry {
  const parts = raw.trim().split(".");
  const column = parts[parts.length - 1] ?? "";
  return { raw: raw.trim(), table: parts.length >= 2 ? parts[parts.length - 2] : null, column };
}

const same = (a: string, b: string) => a.toLowerCase() === b.toLowerCase();

/** Whether `entry` names this table's column exactly (not via a bare name). */
export function namesColumn(entry: NeverSendEntry, table: string, column: string): boolean {
  return entry.table !== null && same(entry.table, table) && same(entry.column, column);
}

/** Whether the schema has the column an entry names: any table's for a bare name. */
export function inSchema(entry: NeverSendEntry, schema: Schema): boolean {
  return schema.tables.some(
    (t) =>
      (entry.table === null || same(entry.table, t.name)) &&
      t.columns.some((c) => same(c.name, entry.column)),
  );
}

/** The stored form for one table's column. */
export const entryFor = (table: string, column: string) => `${table}.${column}`;

/** Adds entries not already present (case-insensitive), keeping order. */
export function addEntries(list: string[], add: string[]): string[] {
  const out = [...list];
  for (const a of add.map((x) => x.trim()).filter(Boolean))
    if (!out.some((x) => same(x, a))) out.push(a);
  return out;
}

/** Splits the old typed form: one per line or comma separated. */
export const splitTyped = (text: string) => addEntries([], text.split(/[\n,]/));

// --- Likely personal or sensitive columns, by name -------------------------------------------
// Mirrors `column_kind` in src-tauri/src/safety/mask/mod.rs, plus pay and health, which aren't
// masked by shape but are worth keeping out entirely.

/** `PatientEmail`, `patient_email`, `patient email` → `patient`, `email`; `line1` → `line`, `1`. */
export function nameWords(name: string): string[] {
  const words: string[] = [];
  let cur = "";
  let prev: string | null = null;
  const isAlnum = (c: string) => /[\p{L}\p{N}]/u.test(c);
  const isAlpha = (c: string) => /\p{L}/u.test(c);
  const isLower = (c: string) => c !== c.toUpperCase() && c === c.toLowerCase();
  const isUpper = (c: string) => c !== c.toLowerCase() && c === c.toUpperCase();
  for (const c of name) {
    if (!isAlnum(c)) {
      words.push(cur);
      cur = "";
      prev = null;
      continue;
    }
    if (prev !== null && ((isLower(prev) && isUpper(c)) || isAlpha(prev) !== isAlpha(c))) {
      words.push(cur);
      cur = "";
    }
    cur += c.toLowerCase();
    prev = c;
  }
  words.push(cur);
  return words.filter(Boolean);
}

/** Words before `name` that make it a thing's name rather than a person's: `product_name`. */
const THING_NAMES = new Set([
  "table",
  "column",
  "schema",
  "product",
  "category",
  "file",
  "host",
  "db",
  "database",
  "event",
  "status",
  "role",
  "plan",
  "type",
  "country",
  "city",
  "state",
  "company",
  "org",
  "organization",
  "organisation",
  "tenant",
  "team",
  "brand",
  "app",
  "service",
  "job",
  "feature",
  "department",
  "dept",
  "device",
  "browser",
  "os",
  "model",
  "item",
  "sku",
  "field",
  "key",
  "index",
  "constraint",
  "view",
  "project",
  "group",
  "region",
  "workspace",
  "account",
  "clinic",
  "facility",
  "site",
  "store",
  "shop",
  "domain",
  "metric",
  "tag",
  "label",
  "permission",
  "queue",
  "topic",
  "channel",
  "bucket",
  "test",
  "step",
  "stage",
  "task",
  "drug",
  "medication",
  "procedure",
  "diagnosis",
]);

/** A short label for the kind of personal data a column holds, judged by its name; `null` if none. */
export function sensitiveKind(name: string): string | null {
  const words = nameWords(name);
  const joined = words.join("");
  const is = (w: string, list: string[] | Set<string>) => {
    const has = (x: string) => (list instanceof Set ? list.has(x) : list.includes(x));
    return has(w) || (w.endsWith("s") && has(w.slice(0, -1)));
  };
  const has = (list: string[]) => words.some((w) => is(w, list));
  const contains = (list: string[]) => list.some((w) => joined.includes(w));

  if (joined.includes("email") || has(["mail"])) return "email";
  if (
    joined.includes("phone") ||
    has(["mobile", "cell", "tel", "telephone", "fax", "msisdn", "whatsapp"])
  )
    return "phone";
  if (joined.includes("ipaddress") || has(["ip", "ipv4", "ipv6"])) return "IP address";
  if (
    contains(["password", "passwd", "passphrase", "apikey", "secret", "credential"]) ||
    has(["pwd", "token", "hash", "salt", "otp", "totp", "session", "cvv", "cvc", "pin"]) ||
    (has(["api", "private", "access", "refresh", "signing", "encryption"]) && has(["key"]))
  )
    return "secret";
  if (
    joined.includes("socialsecurity") ||
    has(["ssn", "sin", "nin", "passport", "license", "licence", "nhs", "aadhaar", "pan"]) ||
    (has(["national", "tax", "government", "gov"]) && has(["id", "number", "no", "num"]))
  )
    return "government ID";
  if (
    contains(["creditcard", "cardnumber"]) ||
    has(["iban", "routing", "swift", "bic"]) ||
    (has(["card", "account", "bank"]) && has(["number", "no", "num"]))
  )
    return "card or bank";
  if (has(["birth", "dob", "birthday", "birthdate", "born"])) return "birth date";
  if (
    has([
      "mrn",
      "medical",
      "insurance",
      "chart",
      "patient",
      "diagnosis",
      "health",
      "condition",
      "allergy",
      "prescription",
      "symptom",
    ])
  )
    return "health";
  if (
    joined.includes("address") ||
    has([
      "addr",
      "street",
      "zip",
      "zipcode",
      "postcode",
      "postal",
      "geo",
      "latitude",
      "longitude",
      "lat",
      "lng",
      "lon",
    ])
  )
    return "address";
  if (
    has([
      "firstname",
      "lastname",
      "fullname",
      "surname",
      "forename",
      "nickname",
      "username",
      "displayname",
      "maidenname",
      "handle",
      "signature",
      "initials",
    ]) ||
    joined === "firstname" ||
    joined === "lastname" ||
    joined === "fullname" ||
    (has(["name"]) && !words.some((w) => is(w, THING_NAMES))) ||
    (has(["login"]) && (words.length === 1 || has(["name", "id"])))
  )
    return "name";
  if (has(["salary", "wage", "wages", "income", "compensation", "payroll", "bonus"])) return "pay";
  if (has(["note", "comment", "remark", "message", "memo", "narrative", "transcript"]))
    return "free text";
  return null;
}

/** The singular-ish form of a table name, to tell `products.name` from `customers.name`. */
const singular = (t: string) => t.toLowerCase().replace(/ies$/, "y").replace(/s$/, "");

export interface Suggestion {
  table: string;
  column: string;
  type: string;
  kind: string;
}

/** Columns that likely hold personal or sensitive data, in schema order. */
export function suggestedColumns(schema: Schema): Suggestion[] {
  const out: Suggestion[] = [];
  for (const t of schema.tables) {
    for (const c of t.columns) {
      const kind = sensitiveKind(c.name);
      if (!kind || (kind === "name" && isThingTable(t, c.name))) continue;
      out.push({ table: t.name, column: c.name, type: c.data_type, kind });
    }
  }
  return out;
}

/** `categories.name`, `products.name`: a bare `name` column on a table of things. */
function isThingTable(t: Table, column: string): boolean {
  if (nameWords(column).join("") !== "name") return false;
  const words = nameWords(t.name).map(singular);
  return words.some((w) => THING_NAMES.has(w));
}
