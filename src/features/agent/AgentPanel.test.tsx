// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { IDLE, type AgentRunState } from "./agentRun";
import { AgentPanel } from "./AgentPanel";

const handlers = () => ({
  onSelect: vi.fn(),
  onGo: vi.fn(),
  onSend: vi.fn(),
  onAuto: vi.fn(),
  onSkip: vi.fn(),
  onAnswer: vi.fn(),
  onStop: vi.fn(),
  onOpenQuery: vi.fn(),
  onSaveScript: vi.fn(),
  onRerun: vi.fn(),
  onFollowUp: vi.fn(),
  onSaveNote: vi.fn(() => Promise.resolve()),
  onSteer: vi.fn(),
});

describe("AgentPanel", () => {
  afterEach(cleanup);

  it("pauses on what would be sent", () => {
    const h = handlers();
    const run: AgentRunState = {
      ...IDLE,
      runId: "r",
      question: "Why?",
      phase: "send",
      pending: { kind: "send", stepId: 1, payload: "Step 1: 2 rows." },
    };
    render(<AgentPanel run={run} selected={null} {...h} />);
    expect(screen.getByText("Step 1: 2 rows.")).toBeTruthy();
    fireEvent.click(screen.getByText("Skip (send the row count only)"));
    expect(h.onSkip).toHaveBeenCalled();
    fireEvent.click(screen.getByText("Stop"));
    expect(h.onStop).toHaveBeenCalled();
  });

  it("marks numbers no step backs and never runs a proposed write", () => {
    const h = handlers();
    const run: AgentRunState = {
      ...IDLE,
      runId: "r",
      question: "Delete test customers?",
      phase: "done",
      answer: {
        text: "There are 12 test customers.",
        sentences: [{ text: "There are 12 test customers.", steps: [1], unverified: ["12"] }],
        confidence: "low",
        assumptions: [],
        notes: [],
        proposedSql: "DELETE FROM customers WHERE email LIKE '%test%'",
        proposedChangesData: true,
        cannotAnswer: false,
        unverified: ["12"],
        model: "m",
        usage: { input: 1, cached: 0, output: 1 },
      },
    };
    render(<AgentPanel run={run} selected={null} {...h} />);
    expect(screen.getByTitle(/Not verified/).textContent).toBe("12");
    expect(screen.getByText("not run")).toBeTruthy();
    expect(screen.getByText(/This changes data/)).toBeTruthy();
    fireEvent.click(screen.getByText("Open in editor"));
    expect(h.onOpenQuery).toHaveBeenCalledWith("DELETE FROM customers WHERE email LIKE '%test%'");
    fireEvent.click(screen.getByTitle("Show step 1"));
    expect(h.onSelect).toHaveBeenCalledWith(1);
  });
});
