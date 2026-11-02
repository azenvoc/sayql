// The state of one agent run in a tab, built from the backend's `agent://event`s.
import type { AgentAnswer, AgentEvent, AgentStep, AgentUsage } from "../../api";

export type AgentPhase =
  "idle" | "starting" | "running" | "plan" | "send" | "question" | "done" | "error" | "stopped";

export type AgentPending =
  | { kind: "plan"; steps: string[] }
  | { kind: "send"; stepId: number; payload: string }
  | { kind: "question"; stepId: number; question: string; choices: string[] };

export interface AgentProgress {
  turns: number;
  maxTurns: number;
  queries: number;
  maxQueries: number;
  usage: AgentUsage;
  elapsedMs: number;
}

export interface AgentRunState {
  runId: string | null;
  question: string;
  phase: AgentPhase;
  plan: string[] | null;
  steps: AgentStep[];
  /** What the run is waiting for the person to do. */
  pending: AgentPending | null;
  progress: AgentProgress | null;
  answer: AgentAnswer | null;
  error: string | null;
}

export const IDLE: AgentRunState = {
  runId: null,
  question: "",
  phase: "idle",
  plan: null,
  steps: [],
  pending: null,
  progress: null,
  answer: null,
  error: null,
};

export type AgentAction =
  | { type: "start"; runId: string; question: string; keepSteps: boolean }
  | { type: "failed"; runId: string; error: string }
  | { type: "replied" }
  | { type: "event"; event: AgentEvent }
  | { type: "restore"; state: AgentRunState }
  | { type: "clear" };

export function agentReducer(state: AgentRunState, action: AgentAction): AgentRunState {
  switch (action.type) {
    case "start":
      // A follow-up keeps the earlier steps on screen; their numbers carry on.
      return {
        ...IDLE,
        runId: action.runId,
        question: action.question,
        phase: "starting",
        steps: action.keepSteps ? state.steps : [],
      };
    case "failed":
      return action.runId === state.runId
        ? { ...state, phase: "error", error: action.error, pending: null }
        : state;
    case "replied":
      return state.pending ? { ...state, pending: null, phase: "running" } : state;
    case "restore":
      return action.state;
    case "clear":
      return IDLE;
    case "event":
      return action.event.runId === state.runId ? onEvent(state, action.event) : state;
  }
}

function onEvent(state: AgentRunState, e: AgentEvent): AgentRunState {
  switch (e.type) {
    case "plan":
      return e.waiting
        ? { ...state, plan: e.steps, phase: "plan", pending: { kind: "plan", steps: e.steps } }
        : { ...state, plan: e.steps, phase: "running" };
    case "step": {
      const steps = state.steps.some((s) => s.id === e.step.id)
        ? state.steps.map((s) => (s.id === e.step.id ? e.step : s))
        : [...state.steps, e.step];
      return { ...state, steps, phase: state.pending ? state.phase : "running" };
    }
    case "send":
      return {
        ...state,
        phase: "send",
        pending: { kind: "send", stepId: e.stepId, payload: e.payload },
      };
    case "question":
      return {
        ...state,
        phase: "question",
        pending: { kind: "question", stepId: e.stepId, question: e.question, choices: e.choices },
      };
    case "progress": {
      const { runId: _, type: __, ...progress } = e;
      return { ...state, progress };
    }
    case "done":
      return {
        ...state,
        pending: null,
        answer: e.answer,
        error: e.error,
        phase: e.stopped ? "stopped" : e.error ? "error" : "done",
        // Steps cut short by Stop show as stopped, not running forever.
        steps: state.steps.map((s) =>
          s.status === "running" ? { ...s, status: "error", error: s.error ?? "Stopped." } : s,
        ),
      };
  }
}

/** The run is going: the model or a query is working, or it waits for the person. */
export const isActive = (s: AgentRunState) =>
  ["starting", "running", "plan", "send", "question"].includes(s.phase);

/** What a tab keeps of a finished run across restarts: the queries and the answer, never rows. */
export function forStorage(s: AgentRunState): AgentRunState {
  return {
    ...s,
    pending: null,
    phase: isActive(s) ? "stopped" : s.phase,
    steps: s.steps.map((step) => ({ ...step, rows: [], sent: null })),
  };
}
