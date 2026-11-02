import { describe, expect, it } from "vitest";
import { WRITE_CONFIRM, type QueryResult } from "../../api";
import { runStatements, type Outcome } from "./runStatements";

const rows = (n: number): QueryResult => ({
  columns: ["x"],
  rows: Array.from({ length: n }, (_, i) => [i]),
  rowsAffected: null,
  truncated: false,
  elapsedMs: 1,
});

/** A fake database: SELECTs return rows, "bad" fails, writes need allowWrite. */
function fakeDb() {
  const calls: string[] = [];
  const exec = async (sql: string, allowWrite: boolean) => {
    calls.push(`${allowWrite ? "write" : "run"}: ${sql}`);
    if (sql.includes("bad")) throw "no such table: bad";
    if (/^(delete|update)/i.test(sql) && !allowWrite) throw WRITE_CONFIRM;
    return rows(2);
  };
  return { calls, exec };
}

const summary = (o: Outcome[]) =>
  o.map((x) =>
    x.cancelled ? "cancelled" : x.error ? `error: ${x.error}` : `${x.result?.rows.length} rows`,
  );

describe("runStatements", () => {
  it("runs every statement in order", async () => {
    const db = fakeDb();
    const progress: number[] = [];
    const out = await runStatements(
      ["SELECT 1;", "SELECT 2;"],
      db.exec,
      async () => true,
      (o) => progress.push(o.length),
    );
    expect(summary(out)).toEqual(["2 rows", "2 rows"]);
    expect(db.calls).toEqual(["run: SELECT 1;", "run: SELECT 2;"]);
    expect(progress).toEqual([1, 2]);
  });

  it("stops at the first error", async () => {
    const db = fakeDb();
    const out = await runStatements(
      ["SELECT 1;", "SELECT * FROM bad;", "SELECT 3;"],
      db.exec,
      async () => true,
      () => {},
    );
    expect(summary(out)).toEqual(["2 rows", "error: no such table: bad"]);
    expect(db.calls).toHaveLength(2);
  });

  it("asks before each write and runs it only once confirmed", async () => {
    const db = fakeDb();
    const asked: string[] = [];
    const out = await runStatements(
      ["SELECT 1;", "DELETE FROM t WHERE id = 1;"],
      db.exec,
      async (sql) => (asked.push(sql), true),
      () => {},
    );
    expect(asked).toEqual(["DELETE FROM t WHERE id = 1;"]);
    expect(db.calls).toEqual([
      "run: SELECT 1;",
      "run: DELETE FROM t WHERE id = 1;",
      "write: DELETE FROM t WHERE id = 1;",
    ]);
    expect(summary(out)).toEqual(["2 rows", "2 rows"]);
  });

  it("stops when a write is declined, without running it or what follows", async () => {
    const db = fakeDb();
    const out = await runStatements(
      ["DELETE FROM t;", "SELECT 2;"],
      db.exec,
      async () => false,
      () => {},
    );
    expect(summary(out)).toEqual(["cancelled"]);
    expect(db.calls).toEqual(["run: DELETE FROM t;"]);
  });

  it("marks a team server's refusal, without asking to confirm the write first", async () => {
    const confirm = async () => true;
    const refusal = {
      message:
        "Your team's SayQL server runs only read-only queries, so this statement was not run.",
      teamRefused: true,
    };
    const out = await runStatements(
      ["DELETE FROM t"],
      async () => Promise.reject(refusal),
      confirm,
      () => {},
    );
    expect(out).toEqual([
      {
        sql: "DELETE FROM t",
        result: null,
        error: refusal.message,
        cancelled: false,
        refused: true,
      },
    ]);
    const db = await runStatements(
      ["SELECT bad"],
      async () => Promise.reject("no such column: bad"),
      confirm,
      () => {},
    );
    expect(db[0].refused).toBe(false);
  });
});
