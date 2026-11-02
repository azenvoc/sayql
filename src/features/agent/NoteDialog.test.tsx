// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { DbNote, NoteSuggestion } from "../../api";
import { NoteDialog } from "./NoteDialog";

const doubtful: DbNote = {
  id: "n1",
  connectionId: "c1",
  database: null,
  text: "users.last_login_at is set on every sign-in",
  updatedAt: 0,
  createdAt: 0,
  source: "Claude Code",
  evidence: "auth/session.ts sets it",
  anchors: [{ table: "users" }, { table: "users", column: "last_login_at" }],
  suspect: {
    reason: "the column users.last_login_at no longer exists",
    since: Date.UTC(2026, 9, 29),
  },
  check: {
    sql: "SELECT count(*) > 0 FROM users WHERE last_login_at IS NOT NULL",
    last: { ok: false, at: Date.UTC(2026, 9, 29) },
  },
  history: [{ text: "users.last_login is set on sign-in", at: Date.UTC(2026, 0, 2), source: "" }],
};

const fix: NoteSuggestion = {
  id: "s1",
  connectionId: "c1",
  database: null,
  text: "users.last_seen_at is set on every sign-in",
  evidence: "The schema changed: the column users.last_login_at was renamed to last_seen_at.",
  app: "SayQL",
  createdAt: 0,
  replaces: "n1",
  replacedText: null,
  remove: false,
};

const base = { databaseName: "shop", onSave: vi.fn(), onClose: vi.fn() };

describe("NoteDialog", () => {
  afterEach(cleanup);

  it("shows why a note may be out of date, and lets the person keep it", async () => {
    const onConfirm = vi.fn().mockResolvedValue(undefined);
    render(<NoteDialog {...base} note={doubtful} onConfirm={onConfirm} />);
    expect(screen.getByText("Review database note")).toBeTruthy();
    expect(screen.getByText(/the column users.last_login_at no longer exists/)).toBeTruthy();
    expect(screen.getByText("users.last_login_at")).toBeTruthy();
    expect(screen.getByText(/Failed/)).toBeTruthy();
    expect(screen.getByText(/From Claude Code: auth\/session.ts sets it/)).toBeTruthy();
    expect(screen.getByText("Earlier versions (1)")).toBeTruthy();
    fireEvent.click(screen.getByText("It's still true"));
    await waitFor(() => expect(onConfirm).toHaveBeenCalled());
  });

  it("offers the suggested fix", async () => {
    const onAcceptFix = vi.fn().mockResolvedValue(undefined);
    render(<NoteDialog {...base} note={doubtful} fix={fix} onAcceptFix={onAcceptFix} />);
    expect(screen.getByText("users.last_seen_at is set on every sign-in")).toBeTruthy();
    expect(screen.getByText("from SayQL")).toBeTruthy();
    fireEvent.click(screen.getByText("Use this text"));
    await waitFor(() => expect(onAcceptFix).toHaveBeenCalledWith("s1"));
  });

  it("saves the text with its check, and runs the check on request", async () => {
    const onSave = vi.fn().mockResolvedValue(undefined);
    const passed = {
      ...doubtful,
      suspect: null,
      check: { ...doubtful.check!, last: { ok: true, at: Date.now() } },
    };
    const onCheck = vi.fn().mockResolvedValue(passed);
    render(
      <NoteDialog
        {...base}
        note={{ ...doubtful, suspect: undefined }}
        onSave={onSave}
        onCheck={onCheck}
      />,
    );
    fireEvent.click(screen.getByText("Check now"));
    await waitFor(() => expect(screen.getByText(/Passed/)).toBeTruthy());
    expect((screen.getByText("Save") as HTMLButtonElement).disabled).toBe(true);
    fireEvent.change(screen.getByLabelText("Note"), {
      target: { value: "users.last_seen_at is set on sign-in" },
    });
    fireEvent.click(screen.getByText("Save"));
    await waitFor(() =>
      expect(onSave).toHaveBeenCalledWith(
        "users.last_seen_at is set on sign-in",
        doubtful.check!.sql,
      ),
    );
  });
});
