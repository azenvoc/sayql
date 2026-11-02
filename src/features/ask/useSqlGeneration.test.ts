// @vitest-environment jsdom
import { act, renderHook } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Schema } from "../../api";
import { commands } from "../../api/commands";
import type { AiEventHandlers } from "../../api/events";
import { useSqlGeneration, type SqlGenerationOptions } from "./useSqlGeneration";

vi.mock("../../api/commands", () => ({
  commands: { generateSql: vi.fn(), cancelGeneration: vi.fn() },
}));
let ai: AiEventHandlers;
vi.mock("../../api/events", () => ({
  subscribeToAi: async (h: AiEventHandlers) => {
    ai = h;
    return () => {};
  },
}));
const generateSql = vi.mocked(commands.generateSql);

const schema: Schema = {
  dialect: "SQLite",
  tables: [
    {
      schema: null,
      name: "customers",
      kind: "table",
      row_estimate: null,
      foreign_keys: [],
      columns: [
        { name: "id", data_type: "integer", nullable: false, primary_key: true, enum_values: null },
      ],
    },
  ],
};

// Each test uses its own database so the shared answer cache doesn't carry over.
let db = 0;
function setup(overrides: Partial<SqlGenerationOptions> = {}) {
  const onSql = vi.fn();
  const onRunRequested = vi.fn();
  const options: SqlGenerationOptions = {
    target: { connectionId: `c${++db}`, database: null },
    slot: "tab",
    schema,
    typingModel: "fast",
    aiReady: true,
    previous: null,
    focusTables: [],
    skills: [],
    sql: "",
    onSql,
    onRunRequested,
    ...overrides,
  };
  const { result } = renderHook(() => useSqlGeneration(options));
  return { result, onSql, onRunRequested };
}

/** The request id of the last generateSql call. */
const lastReqId = () => generateSql.mock.calls[generateSql.mock.calls.length - 1][2];

describe("useSqlGeneration", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    vi.mocked(commands.cancelGeneration).mockResolvedValue(undefined);
    generateSql.mockResolvedValue(undefined);
  });
  afterEach(() => {
    vi.useRealTimers();
    vi.clearAllMocks();
  });

  it("answers simple requests instantly on the machine", () => {
    const { result, onSql } = setup();
    act(() => result.current.setQuestion("customers"));
    expect(onSql).toHaveBeenCalledWith("SELECT *\nFROM customers\nLIMIT 100;");
    expect(result.current.status).toEqual({ kind: "instant", source: "rules" });
    expect(generateSql).not.toHaveBeenCalled();
  });

  it("drafts with the AI after a pause, streaming the SQL in", async () => {
    const { result, onSql } = setup();
    act(() => result.current.setQuestion("customers who signed up last week"));
    expect(result.current.status).toEqual({ kind: "writing", task: "draft" });
    expect(generateSql).not.toHaveBeenCalled();
    await act(async () => vi.advanceTimersByTime(300));
    expect(generateSql).toHaveBeenCalledTimes(1);
    const id = lastReqId();
    act(() => ai.delta({ reqId: id, text: "SELECT id ", firstTokenMs: 40 }));
    expect(onSql).toHaveBeenLastCalledWith("SELECT id ");
    act(() => ai.delta({ reqId: id, text: "FROM customers;", firstTokenMs: null }));
    act(() => ai.done({ reqId: id, error: null, totalMs: 120, cacheReadTokens: 0 }));
    expect(onSql).toHaveBeenLastCalledWith("SELECT id FROM customers;");
    expect(result.current.status).toEqual({
      kind: "done",
      task: "draft",
      firstTokenMs: 40,
      totalMs: 120,
    });
  });

  it("a run requested while writing waits for the SQL, then runs it", async () => {
    const { result, onRunRequested } = setup();
    act(() => result.current.setQuestion("customers with no orders"));
    let waiting = false;
    await act(async () => {
      waiting = result.current.requestRun();
    });
    expect(waiting).toBe(true);
    // Run skips the rest of the debounce.
    expect(generateSql).toHaveBeenCalledTimes(1);
    const id = lastReqId();
    act(() => ai.delta({ reqId: id, text: "SELECT 1;", firstTokenMs: 10 }));
    act(() => ai.done({ reqId: id, error: null, totalMs: 20, cacheReadTokens: 0 }));
    expect(onRunRequested).toHaveBeenCalledWith("SELECT 1;", "draft");
  });

  it("ignores events for other requests", async () => {
    const { result, onSql } = setup();
    act(() => result.current.setQuestion("customers who signed up last week"));
    await act(async () => vi.advanceTimersByTime(300));
    act(() => ai.delta({ reqId: lastReqId() + 1000, text: "DROP TABLE x;", firstTokenMs: 1 }));
    expect(onSql).not.toHaveBeenCalled();
  });

  it("asks for an AI provider when none is set up", async () => {
    const { result } = setup({ aiReady: false });
    act(() => result.current.setQuestion("customers who signed up last week"));
    await act(async () => vi.advanceTimersByTime(300));
    expect(generateSql).not.toHaveBeenCalled();
    expect(result.current.status).toMatchObject({ kind: "error", fixInSettings: true });
  });

  it("shows the backend's error", async () => {
    const { result } = setup();
    act(() => result.current.setQuestion("customers who signed up last week"));
    await act(async () => vi.advanceTimersByTime(300));
    act(() =>
      ai.done({ reqId: lastReqId(), error: "rate limited", totalMs: 5, cacheReadTokens: 0 }),
    );
    expect(result.current.status).toEqual({
      kind: "error",
      message: "rate limited",
      fixInSettings: false,
    });
  });

  it("refuses to improve an empty query", () => {
    const { result } = setup();
    act(() => result.current.improve("  "));
    expect(result.current.status).toMatchObject({ kind: "error" });
    expect(generateSql).not.toHaveBeenCalled();
  });
});
