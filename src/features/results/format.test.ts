import { describe, expect, it } from "vitest";
import type { QueryResult } from "../../api";
import { columnWidths, formatCell, plural, statementLabel, toCsv } from "./format";

const result = (columns: string[], rows: QueryResult["rows"]): QueryResult => ({
  columns,
  rows,
  rowsAffected: null,
  truncated: false,
  elapsedMs: 1,
});

describe("formatCell", () => {
  it("renders each value type", () => {
    expect(formatCell(null)).toBe("NULL");
    expect(formatCell(42)).toBe("42");
    expect(formatCell(true)).toBe("true");
    expect(formatCell({ a: 1 })).toBe('{"a":1}');
  });
});

describe("toCsv", () => {
  it("escapes quotes, commas and newlines, and leaves NULL empty", () => {
    const csv = toCsv(
      result(
        ["name", "note"],
        [
          ["Ann", 'said "hi", left'],
          ["Bo", null],
          ["Cy", "a\nb"],
        ],
      ),
    );
    expect(csv).toBe('name,note\nAnn,"said ""hi"", left"\nBo,\nCy,"a\nb"');
  });
});

describe("columnWidths", () => {
  it("clamps between the minimum and maximum width", () => {
    const [narrow, wide] = columnWidths(result(["a", "b"], [["x", "y".repeat(500)]]));
    expect(narrow).toBe(72);
    expect(wide).toBe(360);
  });
});

describe("plural", () => {
  it("pluralises by count", () => {
    expect(plural(1, "row")).toBe("1 row");
    expect(plural(1200, "row")).toBe("1,200 rows");
  });
});

describe("statementLabel", () => {
  it("names the table a statement reads or changes", () => {
    expect(statementLabel("SELECT * FROM customers;")).toBe("customers");
    expect(statementLabel('SELECT id FROM "sales"."orders" o JOIN x ON 1=1')).toBe("orders");
    expect(statementLabel("UPDATE [dbo].[clients] SET a = 1")).toBe("clients");
    expect(statementLabel("INSERT INTO logs VALUES (1)")).toBe("logs");
  });

  it("falls back to the first word", () => {
    expect(statementLabel("-- a note\nSELECT 1;")).toBe("select");
    expect(statementLabel("PRAGMA table_info")).toBe("pragma");
  });
});
