// @vitest-environment jsdom
import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { QUERY_CANCELLED, WRITE_CONFIRM, type QueryResult } from "../../api";
import { commands } from "../../api/commands";
import { useQueryRunner } from "./useQueryRunner";

vi.mock("../../api/commands", () => ({
  commands: { runQuery: vi.fn(), cancelQuery: vi.fn(async () => {}) },
}));
const runQuery = vi.mocked(commands.runQuery);
const cancelQuery = vi.mocked(commands.cancelQuery);

const result = (n: number): QueryResult => ({
  columns: ["x"],
  rows: Array.from({ length: n }, (_, i) => [i]),
  rowsAffected: null,
  truncated: false,
  elapsedMs: 1,
});
const target = { connectionId: "c", database: null };

describe("useQueryRunner", () => {
  beforeEach(() => {
    runQuery.mockReset();
    runQuery.mockImplementation(async (_t, sql, allowWrite) => {
      if (sql.includes("bad")) throw "no such table: bad";
      if (/^delete/i.test(sql) && !allowWrite) throw WRITE_CONFIRM;
      return result(2);
    });
  });

  it("runs each statement and collects the outcomes", async () => {
    const { result: r } = renderHook(() => useQueryRunner(target));
    act(() => void r.current.run("SELECT 1; SELECT 2;"));
    expect(r.current.running).toBe(true);
    await waitFor(() => expect(r.current.running).toBe(false));
    expect(r.current.total).toBe(2);
    expect(r.current.outcomes.map((o) => o.result?.rows.length)).toEqual([2, 2]);
    expect(r.current.error).toBeNull();
  });

  it("reports a single statement's error to onFailure", async () => {
    const onFailure = vi.fn();
    const { result: r } = renderHook(() => useQueryRunner(target, onFailure));
    act(() => void r.current.run("SELECT * FROM bad"));
    await waitFor(() => expect(r.current.running).toBe(false));
    expect(r.current.error).toBe("no such table: bad");
    expect(onFailure).toHaveBeenCalledWith("SELECT * FROM bad", "no such table: bad");
  });

  it("waits for confirmation before a write, and runs it once confirmed", async () => {
    const { result: r } = renderHook(() => useQueryRunner(target));
    act(() => void r.current.run("DELETE FROM t"));
    await waitFor(() => expect(r.current.pendingWrite).toBe("DELETE FROM t"));
    act(() => r.current.confirmWrite());
    await waitFor(() => expect(r.current.running).toBe(false));
    expect(r.current.pendingWrite).toBeNull();
    expect(runQuery).toHaveBeenLastCalledWith(
      target,
      "DELETE FROM t",
      true,
      expect.any(String),
      undefined,
      undefined,
    );
    expect(r.current.error).toBeNull();
  });

  it("cancels the running statement on the server and starts no more", async () => {
    let finish: (() => void) | null = null;
    runQuery.mockImplementation(
      (_t, sql) =>
        new Promise((resolve, reject) => {
          if (sql.includes("slow")) finish = () => reject(QUERY_CANCELLED);
          else resolve(result(1));
        }),
    );
    const onFailure = vi.fn();
    const { result: r } = renderHook(() => useQueryRunner(target, onFailure));
    act(() => void r.current.run("SELECT slow; SELECT 2;"));
    await waitFor(() => expect(runQuery).toHaveBeenCalledTimes(1));
    const runId = runQuery.mock.calls[0][3];
    act(() => r.current.cancel());
    expect(cancelQuery).toHaveBeenCalledWith(runId);
    await act(async () => finish?.());
    await waitFor(() => expect(r.current.running).toBe(false));
    expect(r.current.error).toBe(QUERY_CANCELLED);
    expect(runQuery).toHaveBeenCalledTimes(1);
    // Cancelling isn't a failure to fix.
    expect(onFailure).not.toHaveBeenCalled();
  });

  it("does nothing without a database", () => {
    const { result: r } = renderHook(() => useQueryRunner(null));
    act(() => void r.current.run("SELECT 1"));
    expect(r.current.running).toBe(false);
    expect(runQuery).not.toHaveBeenCalled();
  });
});
