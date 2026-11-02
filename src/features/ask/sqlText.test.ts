import { describe, expect, it } from "vitest";
import { cleanSql } from "./sqlText";

describe("cleanSql", () => {
  it("leaves plain SQL alone", () => {
    expect(cleanSql("SELECT 1;")).toBe("SELECT 1;");
    expect(cleanSql("-- assumes last 30 days\nSELECT * FROM t")).toBe(
      "-- assumes last 30 days\nSELECT * FROM t",
    );
    expect(cleanSql("WITH x AS (SELECT 1)\nSELECT * FROM x")).toBe(
      "WITH x AS (SELECT 1)\nSELECT * FROM x",
    );
  });

  it("strips Markdown fences", () => {
    expect(cleanSql("```sql\nSELECT 1;\n```")).toBe("SELECT 1;");
  });

  it("turns an explanation before the SQL into comments", () => {
    const reply =
      "The user's email is on the Person table, so join those.\n\nSELECT TOP 100\n    p.PERSON_EMAIL\nFROM dbo.Person p";
    expect(cleanSql(reply)).toBe(
      "-- The user's email is on the Person table, so join those.\n\nSELECT TOP 100\n    p.PERSON_EMAIL\nFROM dbo.Person p",
    );
  });

  it("comments a partial explanation while it streams", () => {
    expect(cleanSql("The user's email is")).toBe("-- The user's email is");
  });

  it("never touches lines after the SQL starts", () => {
    expect(cleanSql("SELECT a\nFROM t\nWHERE note = 'The end'")).toBe(
      "SELECT a\nFROM t\nWHERE note = 'The end'",
    );
  });
});
