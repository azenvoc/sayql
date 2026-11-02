// @vitest-environment jsdom
import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import type { NoteSuggestion, Settings } from "../../api";
import { SettingsPage, privacyNote } from "./SettingsPage";

afterEach(cleanup);

const provider = {
  id: "anthropic",
  label: "Anthropic",
  api: "anthropic",
  key: "required",
  local: false,
  baseUrl: "",
  typingModel: "",
  expertModel: "",
};
const settings = {
  provider: "anthropic",
  providers: [provider],
  models: [],
  workspaceId: "",
  team: null,
} as unknown as Settings;
const waiting = (n: number) =>
  Array.from(
    { length: n },
    (_, i) =>
      ({
        id: `s${i}`,
        connectionId: "c1",
        database: null,
        text: "t",
        evidence: "e",
        app: "Cursor",
        createdAt: 1,
      }) as NoteSuggestion,
  );

function renderPage(n: number) {
  return render(
    <SettingsPage
      settings={settings}
      connections={[]}
      section="about"
      noteSuggestions={{
        suggestions: waiting(n),
        accept: vi.fn(),
        dismiss: vi.fn(),
        reload: vi.fn(),
      }}
      skills={{} as never}
      onClose={vi.fn()}
      onSaved={vi.fn()}
      onShowSetup={vi.fn()}
      onShowShortcuts={vi.fn()}
      onShowWhatsNew={vi.fn()}
      onTeamChange={vi.fn()}
    />,
  );
}

it("calls the section AI apps and counts suggested notes waiting on it", () => {
  renderPage(3);
  const item = screen.getByRole("button", { name: /^AI apps/ });
  expect(item.textContent).toContain("AI apps");
  expect(item.textContent).not.toContain("MCP");
  expect(screen.getByLabelText("3 suggested notes waiting").textContent).toBe("3");
});

it("shows no count when nothing is waiting", () => {
  renderPage(0);
  expect(screen.queryByLabelText(/suggested notes? waiting/)).toBeNull();
});

it("says when data from tables goes to the AI provider, and what the Ask box sends", () => {
  const claude = privacyNote({
    id: "anthropic",
    key: "required",
    label: "Anthropic Claude",
    local: false,
  });
  expect(claude).toMatch(/sent to Anthropic Claude\./);
  expect(claude).toMatch(/database notes/);
  expect(claude).toMatch(/only if you turn on Investigate for a connection and allow it/);
  expect(claude).not.toMatch(/never the data/);
  expect(
    privacyNote({ id: "sayql", key: "required", label: "SayQL AI credits", local: false }),
  ).toMatch(/sent to SayQL, which forwards it to Anthropic and keeps only token counts/);
  expect(privacyNote({ id: "ollama", key: "none", label: "Ollama", local: true })).toMatch(
    /sent to Ollama, on this computer/,
  );
});
