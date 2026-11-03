import { describe, expect, it } from "vitest";
import { agentStarters } from "./starters";

const table = (name: string, row_estimate: number | null, kind: "table" | "view" = "table") => ({
  schema: null,
  name,
  kind,
  columns: [],
  foreign_keys: [],
  row_estimate,
});

describe("agentStarters", () => {
  it("asks about the biggest table", () => {
    const s = agentStarters({
      dialect: "SQLite",
      tables: [table("users", 10), table("order_items", 900), table("big_view", 5000, "view")],
    });
    expect(s).toHaveLength(3);
    expect(s[1]).toBe("How has the number of order items changed month by month?");
  });

  it("offers nothing without a schema", () => {
    expect(agentStarters(null)).toEqual([]);
    expect(agentStarters({ dialect: "SQLite", tables: [] })).toEqual([]);
  });
});
