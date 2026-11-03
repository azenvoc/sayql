import { describe, expect, it } from "vitest";
import type { AgentEvent, AgentStep } from "../../api";
import { IDLE, agentReducer, forStorage, isActive, type AgentRunState } from "./agentRun";

const step = (id: number, patch: Partial<AgentStep> = {}): AgentStep => ({
  id,
  tool: "run_query",
  text: "Counting",
  status: "running",
  sql: "SELECT 1",
  rowCount: null,
  truncated: false,
  elapsedMs: null,
  columns: [],
  rows: [],
  sent: null,
  sentSummary: null,
  checks: [],
  error: null,
  ...patch,
});

const ev = (e: AgentEvent) => ({ type: "event" as const, event: e });

describe("agentReducer", () => {
  const started = agentReducer(IDLE, {
    type: "start",
    runId: "r1",
    question: "Who owns it?",
    keepSteps: false,
  });

  it("follows a run from plan to answer", () => {
    let s: AgentRunState = started;
    s = agentReducer(s, ev({ type: "plan", runId: "r1", steps: ["a", "b"], waiting: true }));
    expect(s.phase).toBe("plan");
    expect(s.pending).toEqual({ kind: "plan", steps: ["a", "b"] });
    s = agentReducer(s, { type: "replied" });
    expect(s.pending).toBeNull();
    s = agentReducer(s, ev({ type: "step", runId: "r1", step: step(1) }));
    s = agentReducer(
      s,
      ev({
        type: "step",
        runId: "r1",
        step: step(1, { status: "done", rows: [[1]], rowCount: 1 }),
      }),
    );
    expect(s.steps).toHaveLength(1);
    expect(s.steps[0].status).toBe("done");
    s = agentReducer(s, ev({ type: "send", runId: "r1", stepId: 1, payload: "Step 1: 1 rows." }));
    expect(s.phase).toBe("send");
    expect(isActive(s)).toBe(true);
    s = agentReducer(s, { type: "replied" });
    s = agentReducer(
      s,
      ev({
        type: "done",
        runId: "r1",
        error: null,
        stopped: false,
        answer: {
          text: "Dana.",
          sentences: [],
          confidence: "high",
          assumptions: [],
          notes: [],
          proposedSql: null,
          proposedChangesData: false,
          notesQueued: false,
          notesShared: false,
          cannotAnswer: false,
          unverified: [],
          model: "m",
          usage: { input: 1, cached: 2, output: 3 },
        },
      }),
    );
    expect(s.phase).toBe("done");
    expect(s.answer?.text).toBe("Dana.");
    expect(isActive(s)).toBe(false);
  });

  it("ignores other runs' events and marks cut-off steps on stop", () => {
    let s = agentReducer(started, ev({ type: "step", runId: "other", step: step(9) }));
    expect(s.steps).toHaveLength(0);
    s = agentReducer(s, ev({ type: "step", runId: "r1", step: step(1) }));
    s = agentReducer(
      s,
      ev({ type: "done", runId: "r1", answer: null, error: null, stopped: true }),
    );
    expect(s.phase).toBe("stopped");
    expect(s.steps[0].status).toBe("error");
  });

  it("keeps steps for a follow-up and stores no rows", () => {
    const s = agentReducer(
      started,
      ev({
        type: "step",
        runId: "r1",
        step: step(1, { status: "done", rows: [["Dana"]], sent: "x" }),
      }),
    );
    expect(
      agentReducer(s, { type: "start", runId: "r2", question: "and?", keepSteps: true }).steps,
    ).toHaveLength(1);
    const stored = forStorage(s);
    expect(stored.steps[0].rows).toEqual([]);
    expect(stored.steps[0].sent).toBeNull();
    expect(stored.phase).toBe("stopped");
  });
});
