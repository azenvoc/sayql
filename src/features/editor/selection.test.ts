import { describe, expect, it } from "vitest";
import { resolveSelection } from "./selection";

/** Selects `part` (its first occurrence) in `sql`. */
const pick = (sql: string, part: string) => {
  const start = sql.indexOf(part);
  if (start < 0) throw new Error(`"${part}" not in sql`);
  return resolveSelection(sql, { start, end: start + part.length });
};

describe("resolveSelection", () => {
  const two =
    "-- top customers\nSELECT id, name\nFROM customers\nWHERE country = 'UK';\n\nDELETE FROM orders\nWHERE id = 7;";

  it("runs whole statements as selected", () => {
    expect(pick(two, "SELECT id, name\nFROM customers\nWHERE country = 'UK';")).toMatchObject({
      sql: "SELECT id, name\nFROM customers\nWHERE country = 'UK';",
      expanded: false,
    });
    // Without the semicolon, or with the comment above it, it's still the whole statement.
    expect(pick(two, "SELECT id, name\nFROM customers\nWHERE country = 'UK'")?.expanded).toBe(
      false,
    );
    expect(
      pick(two, "-- top customers\nSELECT id, name\nFROM customers\nWHERE country = 'UK';")
        ?.expanded,
    ).toBe(false);
  });

  it("snaps a cut statement to the whole statement", () => {
    expect(pick(two, "SELECT id, name\nFROM customers")).toMatchObject({
      sql: "SELECT id, name\nFROM customers\nWHERE country = 'UK';",
      expanded: true,
    });
  });

  it("never runs a DELETE without its WHERE", () => {
    expect(pick(two, "DELETE FROM orders")).toMatchObject({
      sql: "DELETE FROM orders\nWHERE id = 7;",
      expanded: true,
    });
  });

  it("snaps a selection across two statements to both", () => {
    const r = pick(two, "'UK';\n\nDELETE");
    expect(r).toMatchObject({ expanded: true });
    expect(r?.sql.startsWith("SELECT id") && r.sql.endsWith("WHERE id = 7;")).toBe(true);
  });

  it("runs a bracketed subquery picked on purpose", () => {
    const sql =
      "SELECT * FROM customers WHERE id IN (SELECT customer_id FROM orders WHERE total > 100);";
    expect(pick(sql, "SELECT customer_id FROM orders WHERE total > 100")).toMatchObject({
      sql: "SELECT customer_id FROM orders WHERE total > 100",
      expanded: false,
    });
    // Part of the subquery isn't enough.
    expect(pick(sql, "SELECT customer_id FROM orders")?.expanded).toBe(true);
  });

  it("ignores a blank selection", () => {
    expect(resolveSelection("SELECT 1;\n\nSELECT 2;", { start: 9, end: 11 })).toBeNull();
  });
});
