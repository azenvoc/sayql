// Instant SQL for simple requests ("show customers", "how many orders", "latest 20 reviews"),
// built from the cached schema with no network call. Anything less certain goes to the AI.
import type { Schema, Table } from "../../api";
import { qualifiedName, selectAllSql, selectRowsSql, tableRef } from "../schema";

function normalize(s: string): string {
  return s
    .toLowerCase()
    .replace(/[_\s]+/g, " ")
    .trim();
}

/** Singular/plural spellings of a table name, e.g. "categories" ↔ "category". */
function variants(name: string): string[] {
  const n = normalize(name);
  const out = new Set([n]);
  if (n.endsWith("ies")) out.add(n.slice(0, -3) + "y");
  else if (n.endsWith("s")) out.add(n.slice(0, -1));
  else out.add(n.endsWith("y") ? n.slice(0, -1) + "ies" : n + "s");
  return [...out];
}

/** The single table whose name matches `phrase`, or null when none or several match. */
function findTable(schema: Schema, phrase: string): Table | null {
  const p = normalize(phrase);
  const hits = schema.tables.filter(
    (t) => variants(t.name).includes(p) || variants(qualifiedName(t).replace(".", " ")).includes(p),
  );
  return hits.length === 1 ? hits[0] : null;
}

/** The column to sort by for "latest": a creation timestamp if there is one, else the primary key. */
function recencyColumn(t: Table): string | null {
  const timestamp = t.columns.find((c) =>
    /^(created|inserted|ordered|signed_?up|updated)(_?at|_?on|_?date)?$/i.test(c.name),
  );
  return (timestamp ?? t.columns.find((c) => c.primary_key))?.name ?? null;
}

const COUNT =
  /^(?:count(?: of)?(?: all)?|how many|number of|total number of) (?:the )?(.+?)(?: are there| do we have| exist| in total| total)?$/;
const ORDER_WORDS = "first|last|latest|newest|recent|most recent";
// "top N" is deliberately absent: it implies a ranking, which needs the AI.
const FIRST_N = new RegExp(
  `^(?:show |list |get |display )?(?:me )?(?:the )?(${ORDER_WORDS})? ?(\\d{1,5}) (?:(${ORDER_WORDS}) )?(.+)$`,
);
const LIST =
  /^(?:show|list|get|display|see|view|open|select)? ?(?:me )?(?:all |every |the )*(?:rows (?:in|from) )?(.+?)(?: table)?$/;

export function fastPath(question: string, schema: Schema | null): string | null {
  if (!schema) return null;
  const q = question
    .trim()
    .toLowerCase()
    .replace(/[?.!]+$/, "")
    .replace(/\s+/g, " ");
  if (!q) return null;
  const { dialect } = schema;
  let m: RegExpMatchArray | null;

  if ((m = q.match(COUNT))) {
    const t = findTable(schema, m[1]);
    if (t) {
      const alias = `total_${t.name.toLowerCase().replace(/[^a-z0-9_]/g, "_")}`;
      return `SELECT COUNT(*) AS ${alias}\nFROM ${tableRef(t, dialect)};`;
    }
  }

  if ((m = q.match(FIRST_N))) {
    const t = findTable(schema, m[4]);
    const n = Math.min(parseInt(m[2], 10), 10000);
    if (t && n > 0) {
      const order = m[1] ?? m[3];
      const newest = !!order && /last|latest|newest|recent/.test(order);
      const col = newest ? recencyColumn(t) : null;
      if (newest && !col) return null;
      return selectRowsSql(t, dialect, n, col ?? undefined);
    }
  }

  if ((m = q.match(LIST))) {
    const t = findTable(schema, m[1]);
    // "all users" means all of them (the app caps what it shows); "users" alone gets a 100-row preview.
    if (t)
      return /\b(all|every)\b/.test(q)
        ? `SELECT *\nFROM ${tableRef(t, dialect)};`
        : selectAllSql(t, dialect);
  }
  return null;
}
