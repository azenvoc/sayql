import { describe, expect, it } from "vitest";
import type { Dialect } from "../api";
import { splitForRun, statementRanges } from "./sqlStatements";

const texts = (sql: string, dialect?: Dialect) =>
  statementRanges(sql, dialect).map((r) => sql.slice(r.start, r.end));

describe("statementRanges", () => {
  it("splits at semicolons", () => {
    expect(texts("SELECT 1;\n\nSELECT 2;\nSELECT 3")).toEqual([
      "SELECT 1;",
      "SELECT 2;",
      "SELECT 3",
    ]);
  });

  it("ignores semicolons in strings, quoted names and comments", () => {
    const sql = "SELECT ';' AS \"a;b\", [c;d] -- x;y\nFROM t /* ; */;\nSELECT $$;$$;";
    expect(texts(sql)).toEqual([
      "SELECT ';' AS \"a;b\", [c;d] -- x;y\nFROM t /* ; */;",
      "SELECT $$;$$;",
    ]);
  });

  it("reads MySQL's # comments, -- rule and backslash escapes", () => {
    const sql = "SELECT 'it\\'s; fine', \"a\\\";b\" # c;d\nFROM t;\nSELECT 5--1;\nSELECT 2";
    expect(texts(sql, "MySQL")).toEqual([
      "SELECT 'it\\'s; fine', \"a\\\";b\" # c;d\nFROM t;",
      "SELECT 5--1;",
      "SELECT 2",
    ]);
    // Elsewhere `#` is an operator and a backslash is an ordinary character.
    expect(texts("SELECT 5 # 3; SELECT 'a\\'; SELECT 2", "PostgreSQL")).toEqual([
      "SELECT 5 # 3;",
      "SELECT 'a\\';",
      "SELECT 2",
    ]);
  });

  it("reads Postgres E'' strings and $tag$ quoting", () => {
    const sql = "SELECT E'it\\'s;'; SELECT $f1$ ; $f1$; SELECT 3";
    expect(texts(sql, "PostgreSQL")).toEqual([
      "SELECT E'it\\'s;';",
      "SELECT $f1$ ; $f1$;",
      "SELECT 3",
    ]);
  });

  it("returns nothing for blank text", () => {
    expect(texts("  \n ")).toEqual([]);
  });
});

describe("splitForRun", () => {
  it("runs each statement on its own, without comment-only parts", () => {
    expect(
      splitForRun("-- customers\nSELECT * FROM customers;\nSELECT * FROM orders;\n-- done"),
    ).toEqual(["SELECT * FROM customers;", "SELECT * FROM orders;"]);
    expect(splitForRun("SELECT 1")).toEqual(["SELECT 1"]);
    expect(splitForRun("  -- nothing\n")).toEqual([]);
  });

  it("sends a procedure or trigger definition whole", () => {
    const sql = "CREATE PROCEDURE p AS BEGIN SELECT 1; SELECT 2; END;";
    expect(splitForRun(sql)).toEqual([sql]);
    expect(
      splitForRun("create or replace function f() returns int as $$ select 1; $$ language sql;"),
    ).toHaveLength(1);
  });
});
