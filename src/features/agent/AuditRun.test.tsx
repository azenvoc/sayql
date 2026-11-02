// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { AuditAgent, AuditEvent, KnowledgeSource } from "../../api";
import { AuditButton, AuditDialog, auditButtonLabel, auditButtonTitle } from "./AuditRun";
import { SUGGESTIONS_CHANGED } from "./useNoteSuggestions";

const claude = (patch: Partial<AuditAgent> = {}): AuditAgent => ({
  id: "claude",
  label: "Claude Code",
  path: "/Users/me/.local/bin/claude",
  version: "2.1.267 (Claude Code)",
  supported: true,
  ready: true,
  problem: null,
  installUrl: "https://code.claude.com/docs/en/setup",
  ...patch,
});
const codex: AuditAgent = {
  id: "codex",
  label: "Codex CLI",
  path: "/opt/homebrew/bin/codex",
  version: null,
  supported: false,
  ready: false,
  problem: "Codex reads code by running shell commands.",
  installUrl: "https://developers.openai.com/codex/cli",
};

let agents: AuditAgent[] = [];
const shop: KnowledgeSource = {
  id: "src_shop",
  label: "shop",
  kind: { type: "local", path: "/Users/me/code/shop" },
  lastFetch: null,
  lastCommit: null,
  error: null,
};
let sources: KnowledgeSource[] = [];
let emit: (e: AuditEvent) => void = () => {};
const startAudit = vi.fn((_run: string, _agent: string, _id: string, _db: string | null) =>
  Promise.resolve(),
);
const cancelAudit = vi.fn((_run: string) => Promise.resolve());
const openAuditInTerminal = vi.fn((_agent: string, _id: string, _db: string | null) =>
  Promise.resolve(),
);
const openUrl = vi.fn();
let savedModel = "opus";
const setAuditModel = vi.fn((m: string) => {
  savedModel = m;
  return Promise.resolve();
});

const auditPayment = vi.fn(() => Promise.resolve({ payment: "credits", hasCreditKey: true }));
const setAuditPayment = vi.fn((_p: string) => Promise.resolve());

vi.mock("@tauri-apps/plugin-opener", () => ({ openUrl: (u: string) => openUrl(u) }));
vi.mock("../../api", async (orig) => ({
  ...(await orig<typeof import("../../api")>()),
  commands: {
    auditAgents: () => Promise.resolve(agents),
    knowledgeSources: () => Promise.resolve(sources),
    addKnowledgeFolder: () => Promise.resolve(shop),
    startAudit: (r: string, a: string, i: string, d: string | null) => startAudit(r, a, i, d),
    cancelAudit: (r: string) => cancelAudit(r),
    openAuditInTerminal: (a: string, i: string, d: string | null) => openAuditInTerminal(a, i, d),
    auditPrompt: () => Promise.resolve("Audit the database"),
    auditModel: () => Promise.resolve(savedModel),
    setAuditModel: (m: string) => setAuditModel(m),
    auditPayment: () => auditPayment(),
    setAuditPayment: (p: string) => setAuditPayment(p),
  },
  subscribeToAudit: (h: (e: AuditEvent) => void) => {
    emit = h;
    return Promise.resolve(() => {});
  },
}));

const target = { connectionId: "c1", name: "Shop", database: "sales" };

beforeEach(() => {
  agents = [claude(), codex];
  sources = [];
  auditPayment.mockClear();
  setAuditPayment.mockClear();
  startAudit.mockClear();
  cancelAudit.mockClear();
  openAuditInTerminal.mockClear();
});
afterEach(cleanup);

it("is called Run audit, and says in plain text what it runs", () => {
  expect(auditButtonLabel(null)).toBe("Run audit");
  expect(auditButtonLabel([claude(), codex])).toBe("Run audit");
  expect(auditButtonTitle([claude(), codex])).toBe(
    "Runs your installed Claude Code, read-only, over this database's code",
  );
  expect(auditButtonTitle([claude({ ready: false, problem: "too old" }), codex])).toBe(
    "Runs your coding agent, read-only, over this database's code",
  );
  expect(auditButtonTitle(null)).toBe(
    "Runs your coding agent, read-only, over this database's code",
  );
});

it("opens the run dialog from the button, or hands it off", async () => {
  const onOpen = vi.fn();
  render(<AuditButton target={target} onOpen={onOpen} />);
  const button = await screen.findByRole("button", { name: "Run audit on Shop" });
  fireEvent.click(button);
  expect(onOpen).toHaveBeenCalled();
  expect(screen.queryByRole("dialog")).toBeNull();
  cleanup();
  render(<AuditButton target={target} />);
  fireEvent.click(await screen.findByRole("button", { name: "Run audit on Shop" }));
  expect(await screen.findByRole("dialog", { name: "Run an audit" })).toBeTruthy();
  expect(
    screen.getByText(
      /Runs your installed Claude Code, read-only, over this database's knowledge sources/,
    ),
  ).toBeTruthy();
  expect(screen.getByText("Knowledge sources")).toBeTruthy();
  expect(screen.getByText(/SayQL never reads your Claude Code credentials/)).toBeTruthy();
});

it("needs a code source, then streams progress and ends with the suggestions to review", async () => {
  const onReview = vi.fn();
  const onClose = vi.fn();
  const changed = vi.fn();
  window.addEventListener(SUGGESTIONS_CHANGED, changed);
  render(<AuditDialog target={target} onReview={onReview} onClose={onClose} />);
  const start = await screen.findByRole("button", { name: "Start audit" });
  expect((start as HTMLButtonElement).disabled).toBe(true);
  expect(screen.getByText(/No commands, no file changes, no web/)).toBeTruthy();
  fireEvent.click(await screen.findByRole("button", { name: "Add folder…" }));
  expect(await screen.findByText("/Users/me/code/shop")).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: "Open in Terminal" }));
  expect(openAuditInTerminal).toHaveBeenCalledWith("claude", "c1", "sales");

  fireEvent.click(screen.getByRole("button", { name: "Start audit" }));
  await waitFor(() => expect(startAudit).toHaveBeenCalled());
  const [runId, agent, id, db] = startAudit.mock.calls[0];
  expect([agent, id, db]).toEqual(["claude", "c1", "sales"]);
  expect(screen.getByText("shop", { selector: "code" })).toBeTruthy();
  act(() => {
    emit({ type: "line", runId: "someone-else", kind: "step", text: "Not this run" });
    emit({ type: "line", runId, kind: "step", text: "Reading src/db/orders.ts" });
    emit({ type: "line", runId, kind: "note", text: "Suggested a note: orders.total is in cents" });
  });
  expect(screen.getByText("Reading src/db/orders.ts")).toBeTruthy();
  expect(screen.queryByText("Not this run")).toBeNull();
  expect(screen.getByRole("button", { name: "Cancel" })).toBeTruthy();
  act(() =>
    emit({
      type: "done",
      runId,
      outcome: "ok",
      suggested: 6,
      summary:
        "## Audit\n\nI suggested **6 notes**.\n\n- `orders.status` codes\n- see [docs](https://example.com)\n\n<img src=x onerror=alert(1)>",
      error: null,
    }),
  );
  expect(screen.getByText("6 notes suggested.")).toBeTruthy();
  // The summary is Markdown, rendered as elements: never as HTML.
  expect(screen.getByRole("heading", { name: "Audit" })).toBeTruthy();
  expect(screen.getByText("6 notes").tagName).toBe("STRONG");
  expect(screen.getByText("orders.status").tagName).toBe("CODE");
  fireEvent.click(screen.getByRole("link", { name: "docs" }));
  expect(openUrl).toHaveBeenCalledWith("https://example.com");
  expect(document.querySelector("img")).toBeNull();
  expect(screen.getByText("<img src=x onerror=alert(1)>")).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: "Review suggestions" }));
  expect(onClose).toHaveBeenCalled();
  expect(onReview).toHaveBeenCalled();
  expect(changed).toHaveBeenCalled();
  window.removeEventListener(SUGGESTIONS_CHANGED, changed);
});

it("runs on the saved model, Sonnet by default, and remembers a new choice", async () => {
  sources = [shop];
  savedModel = "opus";
  setAuditModel.mockClear();
  render(<AuditDialog target={target} onClose={() => {}} />);
  const select = (await screen.findByRole("combobox", {
    name: "Model for the audit",
  })) as HTMLSelectElement;
  await waitFor(() => expect(select.value).toBe("opus"));
  expect([...select.options].map((o) => o.text)).toEqual([
    "Sonnet (cheaper)",
    "Opus",
    "My Claude Code default",
  ]);
  expect(screen.getByText(/counts against your own Claude plan/)).toBeTruthy();
  fireEvent.change(select, { target: { value: "sonnet" } });
  await waitFor(() => expect(setAuditModel).toHaveBeenCalledWith("sonnet"));
  expect(select.value).toBe("sonnet");
  expect(screen.getByText(/Sonnet is cheaper/)).toBeTruthy();
});

it("always runs on the person's own sign-in: no choice to pay with SayQL AI credits", async () => {
  sources = [shop];
  savedModel = "default";
  render(<AuditDialog target={target} onClose={() => {}} />);
  const model = (await screen.findByRole("combobox", {
    name: "Model for the audit",
  })) as HTMLSelectElement;
  await waitFor(() => expect(model.value).toBe("default"));
  // Every model is offered, "my default" included, and nothing about paying with credits.
  expect([...model.options].map((o) => o.text)).toEqual([
    "Sonnet (cheaper)",
    "Opus",
    "My Claude Code default",
  ]);
  expect(screen.queryByRole("combobox", { name: /Pay/ })).toBeNull();
  expect(screen.queryByText(/SayQL AI credits/)).toBeNull();
  expect(screen.getByText(/uses your own Claude Code sign-in/)).toBeTruthy();
  expect(auditPayment).not.toHaveBeenCalled();
  await screen.findByText("/Users/me/code/shop");
  const terminal = screen.getByRole("button", { name: "Open in Terminal" }) as HTMLButtonElement;
  expect(terminal.disabled).toBe(false);
  fireEvent.click(screen.getByRole("button", { name: "Start audit" }));
  await waitFor(() => expect(startAudit).toHaveBeenCalled());
  expect(setAuditPayment).not.toHaveBeenCalled();
});

it("cancels a run, and shows why one couldn't start", async () => {
  sources = [shop];
  render(<AuditDialog target={target} onClose={() => {}} />);
  await screen.findByText("/Users/me/code/shop");
  fireEvent.click(screen.getByRole("button", { name: "Start audit" }));
  await waitFor(() => expect(startAudit).toHaveBeenCalled());
  const runId = startAudit.mock.calls[0][0];
  fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
  expect(cancelAudit).toHaveBeenCalledWith(runId);
  act(() =>
    emit({
      type: "done",
      runId,
      outcome: "error",
      suggested: 0,
      summary: null,
      error:
        "Claude Code isn't signed in: run `claude` once in a terminal and sign in, then try again.",
    }),
  );
  expect(screen.getByRole("alert").textContent).toContain("isn't signed in");

  startAudit.mockImplementationOnce(() =>
    Promise.reject("Claude Code is blocked from using SayQL."),
  );
  fireEvent.click(screen.getByRole("button", { name: "Run again" }));
  fireEvent.click(screen.getByRole("button", { name: "Start audit" }));
  expect(await screen.findByText("Claude Code is blocked from using SayQL.")).toBeTruthy();
  expect(screen.getByRole("button", { name: "Start audit" })).toBeTruthy();
});

it("without an agent it can run, explains, links to the install page and keeps the prompt", async () => {
  agents = [claude({ path: null, version: null, ready: false }), codex];
  render(<AuditDialog target={target} onClose={() => {}} />);
  expect(await screen.findByText(/Codex reads code by running shell commands/)).toBeTruthy();
  expect(screen.queryByRole("button", { name: "Start audit" })).toBeNull();
  fireEvent.click(screen.getByRole("button", { name: "Install Claude Code" }));
  expect(openUrl).toHaveBeenCalledWith("https://code.claude.com/docs/en/setup");
  expect(screen.getByRole("button", { name: "Copy audit prompt" })).toBeTruthy();
});

it("asks for the connection when it has none", async () => {
  sources = [shop];
  const connections = [
    { id: "c1", name: "Shop" },
    { id: "c2", name: "Analytics" },
    { id: "team:t1", name: "Team DB" },
  ] as never;
  render(<AuditDialog connections={connections} onClose={() => {}} />);
  const select = (await screen.findByRole("combobox", {
    name: "Connection to audit",
  })) as HTMLSelectElement;
  expect([...select.options].map((o) => o.text)).toEqual(["Shop", "Analytics"]);
  fireEvent.change(select, { target: { value: "c2" } });
  await screen.findByText("/Users/me/code/shop");
  fireEvent.click(screen.getByRole("button", { name: "Start audit" }));
  await waitFor(() => expect(startAudit).toHaveBeenCalled());
  expect(startAudit.mock.calls[0].slice(1)).toEqual(["claude", "c2", null]);
});

const builtIn = (patch: Partial<AuditAgent> = {}): AuditAgent => ({
  id: "opencode",
  label: "SayQL's built-in OpenCode",
  path: null,
  version: "1.19.5",
  supported: true,
  ready: true,
  problem: null,
  installUrl: "https://opencode.ai",
  builtIn: true,
  downloadBytes: 66_056_854,
  detail: "SayQL AI credits · claude-sonnet-5-5",
  ...patch,
});

it("runs Claude Code when it's installed, and offers the built-in OpenCode beside it", async () => {
  agents = [claude(), codex, builtIn()];
  sources = [shop];
  expect(auditButtonTitle(agents)).toBe(
    "Runs your installed Claude Code, read-only, over this database's code",
  );
  render(<AuditDialog target={target} onClose={() => {}} />);
  const select = (await screen.findByRole("combobox", { name: "Audit with" })) as HTMLSelectElement;
  expect(select.value).toBe("claude");
  expect([...select.options].map((o) => o.text)).toEqual([
    "Claude Code",
    "SayQL's built-in OpenCode",
  ]);
  await screen.findByText("/Users/me/code/shop");
  fireEvent.click(screen.getByRole("button", { name: "Start audit" }));
  await waitFor(() => expect(startAudit).toHaveBeenCalled());
  expect(startAudit.mock.calls[0][1]).toBe("claude");
});

it("without Claude Code it runs the built-in OpenCode, saying what it downloads and showing the download", async () => {
  agents = [claude({ path: null, version: null, ready: false }), codex, builtIn()];
  sources = [shop];
  expect(auditButtonTitle(agents)).toBe(
    "Runs SayQL's built-in OpenCode, read-only, over this database's code",
  );
  render(<AuditDialog target={target} onClose={() => {}} />);
  expect(await screen.findByText(/Runs OpenCode 1.19.5, built into SayQL, read-only/)).toBeTruthy();
  expect(screen.getByText(/SayQL AI credits · claude-sonnet-5-5/)).toBeTruthy();
  expect(
    screen.getByText(/Downloads OpenCode 1.19.5 and ripgrep, about 66 MB, once, from GitHub/),
  ).toBeTruthy();
  expect(screen.queryByRole("combobox", { name: "Model for the audit" })).toBeNull();
  expect(screen.queryByRole("button", { name: "Open in Terminal" })).toBeNull();
  await screen.findByText("/Users/me/code/shop");
  fireEvent.click(screen.getByRole("button", { name: "Download and start" }));
  await waitFor(() => expect(startAudit).toHaveBeenCalled());
  const [runId, agent] = startAudit.mock.calls[0];
  expect(agent).toBe("opencode");
  act(() => emit({ type: "download", runId, received: 24_000_000, total: 66_056_854 }));
  expect(screen.getByText(/Downloading OpenCode: 24 of 66 MB/)).toBeTruthy();
  act(() => emit({ type: "download", runId, received: 66_056_854, total: 66_056_854 }));
  expect(screen.queryByText(/Downloading OpenCode:/)).toBeNull();
});

it("says why the built-in OpenCode can't run when nothing can", async () => {
  agents = [
    claude({ path: null, version: null, ready: false }),
    codex,
    builtIn({
      ready: false,
      problem: "Choose a model for OpenAI in Settings → AI provider to run the audit with it.",
    }),
  ];
  render(<AuditDialog target={target} onClose={() => {}} />);
  expect(await screen.findByText(/Choose a model for OpenAI/)).toBeTruthy();
  expect(screen.getByRole("button", { name: "Install Claude Code" })).toBeTruthy();
  expect(screen.queryByRole("button", { name: /Install SayQL/ })).toBeNull();
});
