import { describe, expect, it } from "vitest";
import type { DatabaseKind } from "../api";
import { isReadOnly } from "./readOnly";

// The same cases as the tests in src-tauri/src/db/classify.rs, so the preview's check stays in
// step with the app's.
const ALL: DatabaseKind[] = ["postgres", "mysql", "sqlite", "mssql"];
const roEverywhere = (sql: string) => ALL.every((k) => isReadOnly(sql, k));
const rwEverywhere = (sql: string) => ALL.every((k) => !isReadOnly(sql, k));

describe("isReadOnly (dev copy of classify.rs)", () => {
  it("classifies read-only statements", () => {
    expect(roEverywhere("select 1")).toBe(true);
    expect(roEverywhere("  -- top customers\nSELECT * FROM customers")).toBe(true);
    expect(roEverywhere("(SELECT 1) UNION (SELECT 2)")).toBe(true);
    expect(roEverywhere("PRAGMA table_info(orders)")).toBe(true);
    expect(rwEverywhere("PRAGMA journal_mode = WAL")).toBe(true);
    expect(rwEverywhere("-- select\nUPDATE orders SET status = 'paid'")).toBe(true);
    expect(rwEverywhere("The email is on Person.\nSELECT email FROM dbo.Person")).toBe(true);
  });

  it("needs confirmation for write keywords anywhere", () => {
    for (const sql of [
      "SELECT 1; DELETE FROM orders",
      "SELECT * INTO backup_orders FROM orders",
      "WITH gone AS (DELETE FROM t RETURNING *) SELECT * FROM gone",
      "select 1; commit; drop table x",
      "SELECT * FROM t INTO OUTFILE '/tmp/x'",
      "SELECT nextval('orders_id_seq')",
      "SELECT NEXT VALUE FOR dbo.order_seq",
      "SELECT pg_create_physical_replication_slot('s')",
      "SELECT pg_stat_reset_shared('bgwriter')",
      "SELECT GET_LOCK('sayql', 10)",
    ]) {
      expect(rwEverywhere(sql), sql).toBe(true);
    }
    expect(roEverywhere("SELECT pg_try_advisory_xact_lock(42)")).toBe(true);
  });

  it("ignores write words inside strings, quoted names and comments", () => {
    expect(roEverywhere("SELECT * FROM audit WHERE action = 'DELETE'")).toBe(true);
    expect(isReadOnly('SELECT [Update], "insert" FROM t', "mssql")).toBe(true);
    expect(isReadOnly('SELECT `drop`, "insert" FROM t', "mysql")).toBe(true);
    expect(isReadOnly('SELECT "insert" FROM t', "postgres")).toBe(true);
    expect(roEverywhere("SELECT REPLACE(name, 'a', 'b') FROM t WHERE note = 'it''s DELETE'")).toBe(
      true,
    );
  });

  it("reads each dialect's comments and strings the way the database does", () => {
    expect(isReadOnly(String.raw`SELECT '\'' INTO OUTFILE '/tmp/x' -- '`, "mysql")).toBe(false);
    expect(isReadOnly("SELECT 1 # [\nINTO OUTFILE '/tmp/x' -- ]", "mysql")).toBe(false);
    expect(isReadOnly("SELECT 1 /*! INTO OUTFILE '/tmp/x' */", "mysql")).toBe(false);
    expect(isReadOnly("SELECT 1--1 INTO OUTFILE '/tmp/x'", "mysql")).toBe(false);
    expect(isReadOnly("SELECT [delete] FROM t", "mysql")).toBe(false);
    expect(isReadOnly(String.raw`SELECT E'\'' ; DROP TABLE t -- '`, "postgres")).toBe(false);
    expect(isReadOnly("SELECT $q$ ' $q$; DELETE FROM t -- '", "postgres")).toBe(false);
    expect(isReadOnly("SELECT id FROM t # delete later\nWHERE a = 1", "mysql")).toBe(true);
    expect(isReadOnly("SELECT $$it's DELETE$$, a$b FROM t", "postgres")).toBe(true);
    expect(isReadOnly("SHOW CREATE TABLE `orders`", "mysql")).toBe(true);
  });
});
