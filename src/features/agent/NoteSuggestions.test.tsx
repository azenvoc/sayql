// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import type { DbNote, NoteSuggestion } from "../../api";
import { SuggestedNotes } from "../settings/SuggestedNotes";
import { NoteSuggestions } from "./NoteSuggestions";
import { SUGGESTIONS_CHANGED, suggestionFor, useNoteSuggestions } from "./useNoteSuggestions";

let waiting: NoteSuggestion[] = [];
const saved: DbNote[] = [];
vi.mock("../../api", async (orig) => ({
  ...(await orig<typeof import("../../api")>()),
  commands: {
    listNoteSuggestions: () => Promise.resolve(waiting),
    acceptNoteSuggestion: (id: string) => {
      const s = waiting.find((x) => x.id === id)!;
      waiting = waiting.filter((x) => x.id !== id);
      const note = {
        id: "n1",
        connectionId: s.connectionId,
        database: s.database,
        text: s.text,
        updatedAt: 1,
      };
      saved.push(note);
      return Promise.resolve(note);
    },
    dismissNoteSuggestion: (id: string) => {
      waiting = waiting.filter((x) => x.id !== id);
      return Promise.resolve();
    },
  },
}));

afterEach(cleanup);

/** Settings → AI apps → Suggestions, with the list from the hook, as SettingsPage passes it. */
function Suggestions() {
  const api = useNoteSuggestions();
  return <SuggestedNotes connections={[{ id: "c1", name: "Shop" } as never]} {...api} />;
}

const suggestion = (id: string, text: string, app = "Claude Code"): NoteSuggestion => ({
  id,
  connectionId: "c1",
  database: "shop",
  text,
  evidence: "no references in api/ since commit a1b2c3",
  app,
  createdAt: 1,
});

it("shows the app, note and evidence, and accepts or dismisses", async () => {
  waiting = [
    suggestion("s1", "orders_legacy is unused; use orders_v2"),
    suggestion("s2", "status 3 = cancelled", "Cursor"),
  ];
  const changed = vi.fn();
  window.addEventListener(SUGGESTIONS_CHANGED, changed);
  render(<Suggestions />);

  expect(await screen.findByText("orders_legacy is unused; use orders_v2")).toBeTruthy();
  expect(screen.getAllByText(/no references in api\//)).toHaveLength(2);
  expect(screen.getByText("Cursor")).toBeTruthy();
  expect(screen.getAllByText(/Shop · shop/)).toHaveLength(2);

  fireEvent.click(screen.getAllByRole("button", { name: "Accept" })[0]);
  await waitFor(() =>
    expect(screen.queryByText("orders_legacy is unused; use orders_v2")).toBeNull(),
  );
  expect(saved.map((n) => n.text)).toEqual(["orders_legacy is unused; use orders_v2"]);
  expect(changed).toHaveBeenCalled();

  fireEvent.click(screen.getByRole("button", { name: "Dismiss" }));
  // Once nothing is waiting, the tab says so and still explains what suggestions are.
  expect(await screen.findByText("No suggested notes are waiting.")).toBeTruthy();
  expect(screen.getByText(/Nothing changes until you accept it/)).toBeTruthy();
  expect(saved).toHaveLength(1);
  window.removeEventListener(SUGGESTIONS_CHANGED, changed);
});

it("shows what a replacement or removal changes, and the empty state teaches the audit", async () => {
  waiting = [
    {
      ...suggestion("s3", "status 3 = refunded; 4 = cancelled"),
      replaces: "n9",
      replacedText: "status 3 = cancelled",
      remove: false,
    },
    {
      ...suggestion("s4", "orders_v1 was dropped from the code"),
      replaces: "n8",
      replacedText: "orders_v1 is the live orders table",
      remove: true,
    },
  ];
  render(<Suggestions />);
  expect(await screen.findByText("status 3 = cancelled")).toBeTruthy();
  expect(screen.getByText("Replaces:")).toBeTruthy();
  expect(screen.getByText("Remove note:")).toBeTruthy();
  expect(screen.getByText("orders_v1 is the live orders table")).toBeTruthy();
  expect(screen.getByText("Why:")).toBeTruthy();
  expect(screen.getByRole("button", { name: "Replace note" })).toBeTruthy();
  expect(screen.getByRole("button", { name: "Remove note" })).toBeTruthy();
  expect(screen.queryByRole("button", { name: "Accept" })).toBeNull();
  expect(screen.queryByText("Teach SayQL your database")).toBeNull();

  fireEvent.click(screen.getAllByRole("button", { name: "Dismiss" })[0]);
  await waitFor(() => expect(screen.getAllByRole("button", { name: "Dismiss" })).toHaveLength(1));
  fireEvent.click(screen.getByRole("button", { name: "Dismiss" }));
  expect(await screen.findByText("Teach SayQL your database")).toBeTruthy();
  expect(screen.getByText("/mcp__sayql__audit")).toBeTruthy();
  expect(screen.getByText(/notes go stale/)).toBeTruthy();
});

it("shows why an accept failed", async () => {
  const onAccept = vi.fn(() =>
    Promise.reject("A note describes the database, not the data in it."),
  );
  render(
    <NoteSuggestions
      suggestions={[suggestion("s1", "x")]}
      onAccept={onAccept}
      onDismiss={vi.fn()}
    />,
  );
  fireEvent.click(screen.getByRole("button", { name: "Accept" }));
  expect(await screen.findByText(/describes the database/)).toBeTruthy();
});

it("matches suggestions to the open database", () => {
  const s = suggestion("s1", "x");
  expect(suggestionFor(s, { connectionId: "c1", database: "shop" })).toBe(true);
  expect(suggestionFor(s, { connectionId: "c1", database: null })).toBe(false);
  expect(suggestionFor({ ...s, database: null }, { connectionId: "c1", database: null })).toBe(
    true,
  );
  expect(suggestionFor(s, null)).toBe(false);
});
