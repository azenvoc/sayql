import type { Cell, QueryResult } from "../../api";

/** How a cell value is shown in the grid. */
export function formatCell(v: Cell): string {
  if (v === null) return "NULL";
  if (typeof v === "object") return JSON.stringify(v);
  return String(v);
}

/** RFC 4180 CSV. NULL becomes an empty field. */
export function toCsv(r: QueryResult): string {
  const escape = (s: string) => (/[",\n\r]/.test(s) ? `"${s.replace(/"/g, '""')}"` : s);
  const lines = [r.columns.map(escape).join(",")];
  for (const row of r.rows)
    lines.push(row.map((v) => (v === null ? "" : escape(formatCell(v)))).join(","));
  return lines.join("\n");
}

/** Column widths in px, sized to the header and the first rows of content. */
export function columnWidths(r: QueryResult, sampleRows = 200): number[] {
  const sample = r.rows.slice(0, sampleRows);
  return r.columns.map((name, i) => {
    const longest = Math.max(name.length + 2, ...sample.map((row) => formatCell(row[i]).length));
    return Math.round(Math.min(360, Math.max(72, longest * 7.2 + 20)));
  });
}

export function plural(n: number, word: string): string {
  return `${n.toLocaleString()} ${word}${n === 1 ? "" : "s"}`;
}

/** A short name for a statement's result tab: the table it reads or changes, else its first word. */
export function statementLabel(sql: string): string {
  const s = sql.replace(/--[^\n]*/g, " ").replace(/\/\*[\s\S]*?\*\//g, " ");
  const name = s.match(/\b(?:from|into|update|table)\s+([\w."`[\]]+)/i)?.[1];
  if (name)
    return (
      name
        .replace(/["`[\]]/g, "")
        .split(".")
        .pop() || name
    );
  return (s.trim().split(/\s+/)[0] ?? "").toLowerCase();
}
