import { describe, expect, it } from "vitest";
import type { Schema, Table } from "../../api";
import { exampleQuestions } from "./examples";

const table = (name: string, rows: number, extra: Partial<Table> = {}): Table => ({
  schema: null,
  name,
  kind: "table",
  columns: [
    {
      name: "id",
      data_type: "integer",
      nullable: false,
      primary_key: true,
      enum_values: null,
      comment: null,
    },
  ],
  foreign_keys: [],
  row_estimate: rows,
  comment: null,
  checks: [],
  ...extra,
});

describe("exampleQuestions", () => {
  it("uses hand-picked questions for the demo database", () => {
    expect(exampleQuestions(null, true)).toHaveLength(3);
  });

  it("builds questions from the largest tables of the person's own database", () => {
    const schema: Schema = {
      dialect: "PostgreSQL",
      tables: [
        table("regions", 12),
        table("invoices", 90_000, {
          columns: [
            {
              name: "id",
              data_type: "integer",
              nullable: false,
              primary_key: true,
              enum_values: null,
              comment: null,
            },
            {
              name: "issued_at",
              data_type: "timestamp",
              nullable: false,
              primary_key: false,
              enum_values: null,
              comment: null,
            },
          ],
          foreign_keys: [{ column: "account_id", ref_table: "accounts", ref_column: "id" }],
        }),
        table("accounts", 4_000),
      ],
    };
    expect(exampleQuestions(schema, false)).toEqual([
      "Show the 10 most recent invoices by issued_at",
      "How many accounts are there?",
      "List invoices with their accounts",
    ]);
  });

  it("offers nothing for an empty database", () => {
    expect(exampleQuestions({ dialect: "SQLite", tables: [] }, false)).toEqual([]);
  });
});
