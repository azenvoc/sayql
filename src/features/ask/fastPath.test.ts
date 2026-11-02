import { describe, expect, it } from "vitest";
import type { Schema, Table } from "../../api";
import { fastPath } from "./fastPath";

const table = (name: string, cols: string[], schema: string | null = null): Table => ({
  schema,
  name,
  kind: "table",
  row_estimate: null,
  foreign_keys: [],
  columns: cols.map((c, i) => ({
    name: c,
    data_type: "text",
    nullable: true,
    primary_key: i === 0,
    enum_values: null,
  })),
});

const sqlite: Schema = {
  dialect: "SQLite",
  tables: [
    table("customers", ["id", "email", "signed_up_at"]),
    table("orders", ["id", "customer_id", "ordered_at"]),
    table("order_items", ["id", "order_id"]),
    table("categories", ["id", "name"]),
  ],
};

describe("fastPath", () => {
  it("lists a table by name, singular or plural", () => {
    expect(fastPath("customers", sqlite)).toBe("SELECT *\nFROM customers\nLIMIT 100;");
    // "all" means all of them; the app caps the rows it shows.
    expect(fastPath("Show all customer", sqlite)).toBe("SELECT *\nFROM customers;");
    expect(fastPath("every order item", sqlite)).toBe("SELECT *\nFROM order_items;");
    expect(fastPath("list the order items", sqlite)).toBe("SELECT *\nFROM order_items\nLIMIT 100;");
    expect(fastPath("show category", sqlite)).toBe("SELECT *\nFROM categories\nLIMIT 100;");
  });

  it("counts rows", () => {
    expect(fastPath("how many orders are there?", sqlite)).toBe(
      "SELECT COUNT(*) AS total_orders\nFROM orders;",
    );
    expect(fastPath("count customers", sqlite)).toBe(
      "SELECT COUNT(*) AS total_customers\nFROM customers;",
    );
  });

  it("handles first/latest N using a timestamp column", () => {
    expect(fastPath("first 5 orders", sqlite)).toBe("SELECT *\nFROM orders\nLIMIT 5;");
    expect(fastPath("latest 20 orders", sqlite)).toBe(
      "SELECT *\nFROM orders\nORDER BY ordered_at DESC\nLIMIT 20;",
    );
    expect(fastPath("show me the 3 newest customers", sqlite)).toBe(
      "SELECT *\nFROM customers\nORDER BY signed_up_at DESC\nLIMIT 3;",
    );
  });

  it("leaves anything that needs judgment to the AI", () => {
    expect(fastPath("top 10 customers", sqlite)).toBeNull();
    expect(fastPath("customers from India", sqlite)).toBeNull();
    expect(fastPath("revenue by month", sqlite)).toBeNull();
    expect(fastPath("how many orders per customer", sqlite)).toBeNull();
    expect(fastPath("orders", null)).toBeNull();
  });

  it("quotes reserved words and keeps non-public schemas", () => {
    const pg: Schema = {
      dialect: "PostgreSQL",
      tables: [table("user", ["id"]), table("events", ["id"], "analytics")],
    };
    expect(fastPath("users", pg)).toBe('SELECT *\nFROM "user"\nLIMIT 100;');
    expect(fastPath("show all users", pg)).toBe('SELECT *\nFROM "user";');
    expect(fastPath("events", pg)).toBe("SELECT *\nFROM analytics.events\nLIMIT 100;");
    const my: Schema = { dialect: "MySQL", tables: [table("order", ["id"])] };
    expect(fastPath("count orders", my)).toBe("SELECT COUNT(*) AS total_order\nFROM `order`;");
  });

  it("uses TOP and [brackets] for SQL Server", () => {
    const ms: Schema = {
      dialect: "SQL Server",
      tables: [table("Orders", ["OrderID", "CreatedAt"], "dbo"), table("user", ["id"], "sales")],
    };
    expect(fastPath("show orders", ms)).toBe("SELECT TOP 100 *\nFROM dbo.Orders;");
    expect(fastPath("latest 5 orders", ms)).toBe(
      "SELECT TOP 5 *\nFROM dbo.Orders\nORDER BY CreatedAt DESC;",
    );
    expect(fastPath("how many orders", ms)).toBe(
      "SELECT COUNT(*) AS total_orders\nFROM dbo.Orders;",
    );
    expect(fastPath("sales users", ms)).toBe("SELECT TOP 100 *\nFROM sales.[user];");
  });
});
