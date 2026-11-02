import { describe, expect, it } from "vitest";
import type { Column, Table } from "../../api";
import { countRowsSql, templateSql } from "./sql";

const col = (name: string, data_type: string, primary_key = false): Column => ({
  name,
  data_type,
  nullable: !primary_key,
  primary_key,
  enum_values: null,
});

const customers: Table = {
  schema: "public",
  name: "customers",
  kind: "table",
  columns: [col("id", "integer", true), col("name", "text"), col("order", "text")],
  foreign_keys: [],
  row_estimate: 10,
};

describe("templateSql", () => {
  it("writes a limited SELECT of every column, quoting reserved names", () => {
    expect(templateSql(customers, "PostgreSQL", "select")).toBe(
      'SELECT\n  id,\n  name,\n  "order"\nFROM customers\nLIMIT 100;',
    );
    expect(templateSql({ ...customers, schema: "dbo" }, "SQL Server", "select")).toBe(
      "SELECT TOP 100\n  id,\n  name,\n  [order]\nFROM dbo.customers;",
    );
  });

  it("fills INSERT values with typed NULL placeholders", () => {
    expect(templateSql({ ...customers, schema: null }, "SQLite", "insert")).toBe(
      'INSERT INTO customers (\n  id,\n  name,\n  "order"\n) VALUES (\n  NULL, -- id: integer\n  NULL, -- name: text\n  NULL -- order: text\n);',
    );
  });

  it("targets UPDATE and DELETE at the primary key, matching nothing until edited", () => {
    const update = templateSql(customers, "MySQL", "update");
    expect(update).toContain("SET\n  name = NULL, -- text\n  `order` = NULL -- text");
    expect(update.endsWith("WHERE id = NULL; -- the row to change")).toBe(true);
    expect(templateSql(customers, "MySQL", "delete")).toBe(
      "DELETE FROM customers\nWHERE id = NULL; -- the row to delete",
    );
  });

  it("uses a WHERE that matches no rows when there's no primary key", () => {
    const noKey = { ...customers, columns: [col("name", "text")] };
    expect(templateSql(noKey, "PostgreSQL", "delete")).toBe(
      "DELETE FROM customers\nWHERE 1 = 0; -- replace with the rows to delete: customers has no primary key",
    );
  });
});

it("counts rows", () => {
  expect(countRowsSql(customers, "PostgreSQL")).toBe(
    "SELECT COUNT(*) AS row_count\nFROM customers;",
  );
});
