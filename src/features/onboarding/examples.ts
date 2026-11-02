import type { Schema, Table } from "../../api";

/** Questions known to work well on the bundled demo store. */
const DEMO_QUESTIONS = [
  "Top 5 customers by total spend",
  "How many orders were placed each month this year?",
  "Which products have never been ordered?",
];

const looksLikeDate = (type: string) => /date|time/i.test(type);

/**
 * Up to three starter questions for the first-run setup: hand-picked ones for the demo
 * database, otherwise built from the database's largest tables.
 */
export function exampleQuestions(schema: Schema | null, isDemo: boolean): string[] {
  if (isDemo) return DEMO_QUESTIONS;
  const tables = (schema?.tables ?? [])
    .filter((t) => t.kind === "table" && t.columns.length > 0)
    .sort((a, b) => (b.row_estimate ?? 0) - (a.row_estimate ?? 0));
  if (tables.length === 0) return [];

  const [first, second] = tables;
  const out: string[] = [];
  const dated = first.columns.find((c) => looksLikeDate(c.data_type));
  out.push(
    dated
      ? `Show the 10 most recent ${first.name} by ${dated.name}`
      : `Show 10 rows from ${first.name}`,
  );
  if (second) out.push(`How many ${second.name} are there?`);
  const linked = tables.find((t: Table) => t.foreign_keys.length > 0);
  if (linked) out.push(`List ${linked.name} with their ${linked.foreign_keys[0].ref_table}`);
  return out;
}
