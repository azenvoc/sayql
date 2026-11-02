// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { AgentAccess, McpApp, NoteSuggestion } from "../../api";
import type { NoteSuggestionsApi } from "../agent";
import { ClaudeIntegration } from "./ClaudeIntegration";

const PATH = "/Applications/SayQL.app/Contents/MacOS/sayql-mcp";
let setupFails = false;
let apps: McpApp[] = [];
let onlyApproved = false;
const list = () => Promise.resolve({ onlyApproved, apps });
const setMcpAppBlocked = vi.fn((name: string, blocked: boolean) => {
  apps = apps.map((a) =>
    a.name === name ? { ...a, blocked, approved: a.approved || !blocked } : a,
  );
  return list();
});
const approveMcpApp = vi.fn((name: string) => {
  apps = apps.map((a) => (a.name === name ? { ...a, approved: true } : a));
  return list();
});
const setMcpOnlyApproved = vi.fn((on: boolean) => {
  if (on && !onlyApproved) apps = apps.map((a) => ({ ...a, approved: a.approved || !a.blocked }));
  onlyApproved = on;
  return list();
});
const access: AgentAccess = {
  enabled: false,
  results: "off",
  apps: "counts",
  askBeforeSend: true,
  neverSend: [],
  team: null,
};
vi.mock("../../api", async (orig) => ({
  ...(await orig<typeof import("../../api")>()),
  commands: {
    mcpSetup: () =>
      setupFails
        ? Promise.reject("no path")
        : Promise.resolve({ path: PATH, exists: true, appimage: false, os: "macos" }),
    installedCodingAgents: () => Promise.resolve([]),
    mcpApps: () => list(),
    setMcpAppBlocked: (n: string, b: boolean) => setMcpAppBlocked(n, b),
    approveMcpApp: (n: string) => approveMcpApp(n),
    setMcpOnlyApproved: (on: boolean) => setMcpOnlyApproved(on),
    mcpActivity: () => Promise.resolve([]),
    agentAccess: () => Promise.resolve(access),
    mcpAudits: () => Promise.resolve(audits),
    auditPrompt: (id: string, database: string | null) => auditPrompt(id, database),
  },
}));
let audits: { connectionId: string; database: string | null; app: string; at: number }[] = [];
const auditPrompt = vi.fn((id: string, _database: string | null) => Promise.resolve(`Audit ${id}`));

const suggestion: NoteSuggestion = {
  id: "s1",
  connectionId: "c1",
  database: null,
  text: "status 3 = cancelled",
  evidence: "enum in api/",
  app: "Cursor",
  createdAt: 1,
  replaces: null,
  replacedText: null,
  remove: false,
};
const api = (suggestions: NoteSuggestion[]): NoteSuggestionsApi => ({
  suggestions,
  accept: vi.fn(),
  dismiss: vi.fn(),
  reload: vi.fn(),
});
const connections = [{ id: "c1", name: "Shop", kind: "sqlite" } as never];
const provider = { label: "Anthropic", local: false };

beforeEach(() => {
  setupFails = false;
  localStorage.clear();
  apps = [];
  audits = [];
  onlyApproved = false;
  setMcpAppBlocked.mockClear();
  approveMcpApp.mockClear();
  setMcpOnlyApproved.mockClear();
});
afterEach(cleanup);

it("opens on Access, switches tabs, and comes back to the last one", async () => {
  const { unmount } = render(
    <ClaudeIntegration connections={connections} provider={provider} suggestions={api([])} />,
  );
  const access = screen.getByRole("tab", { name: "Access" });
  expect(access.getAttribute("aria-selected")).toBe("true");
  expect(screen.getByRole("tabpanel").getAttribute("aria-labelledby")).toBe(access.id);
  expect(await screen.findByText(/No AI app has connected yet/)).toBeTruthy();

  fireEvent.click(screen.getByRole("tab", { name: "Activity" }));
  expect(screen.getByRole("tab", { name: "Activity" }).getAttribute("aria-selected")).toBe("true");
  expect(await screen.findByText(/Nothing yet\. Queries from Claude/)).toBeTruthy();

  // Arrow keys move along the tab list.
  fireEvent.keyDown(screen.getByRole("tab", { name: "Activity" }), { key: "ArrowRight" });
  expect(screen.getByRole("tab", { name: "Suggestions" }).getAttribute("aria-selected")).toBe(
    "true",
  );
  expect(screen.getByText("No suggested notes are waiting.")).toBeTruthy();
  unmount();

  // Settings opened again in this session: back on Suggestions.
  render(<ClaudeIntegration connections={connections} provider={provider} suggestions={api([])} />);
  expect(screen.getByRole("tab", { name: "Suggestions" }).getAttribute("aria-selected")).toBe(
    "true",
  );
  fireEvent.click(screen.getByRole("tab", { name: "Access" }));
});

it("opens on the tab asked for, with the waiting count on Suggestions", () => {
  render(
    <ClaudeIntegration
      connections={connections}
      provider={provider}
      suggestions={api([suggestion])}
      tab="suggestions"
    />,
  );
  const tab = screen.getByRole("tab", { name: /Suggestions/ });
  expect(tab.getAttribute("aria-selected")).toBe("true");
  expect(tab.textContent).toBe("Suggestions1");
  expect(screen.getByText("status 3 = cancelled")).toBeTruthy();
  fireEvent.click(screen.getByRole("tab", { name: "Access" }));
});

it("shows the setup open when no app has connected, closed once one has", async () => {
  const { unmount } = render(
    <ClaudeIntegration connections={connections} provider={provider} suggestions={api([])} />,
  );
  const toggle = await screen.findByRole("button", { name: "Connect an app" });
  await waitFor(() => expect(toggle.getAttribute("aria-expanded")).toBe("true"));
  // First on the tab, above the apps and the per-connection table.
  const heading = screen.getByRole("heading", { name: "Connections" });
  expect(toggle.compareDocumentPosition(heading) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  // Nothing installed or connected: Claude Code.
  expect(await screen.findByText(`claude mcp add --scope user sayql -- ${PATH}`)).toBeTruthy();
  unmount();

  apps = [{ name: "Cursor", blocked: false, approved: false, queries: 4, lastUsed: 1 }];
  render(<ClaudeIntegration connections={connections} provider={provider} suggestions={api([])} />);
  expect(await screen.findByText(/4 requests in the log/)).toBeTruthy();
  const closed = await screen.findByRole("button", { name: "Connect an app" });
  expect(closed.getAttribute("aria-expanded")).toBe("false");
  expect(screen.queryByRole("combobox", { name: "Your coding agent" })).toBeNull();
  fireEvent.click(closed);
  // The app that connected is picked: Cursor's link and its config file.
  expect(await screen.findByRole("button", { name: "Open in Cursor" })).toBeTruthy();
  expect(screen.getByText(/Add to ~\/\.cursor\/mcp\.json/)).toBeTruthy();
});

it("shows the setup for the agent picked, Claude Desktop included", async () => {
  render(<ClaudeIntegration connections={connections} provider={provider} suggestions={api([])} />);
  const picker = await screen.findByRole("combobox", { name: "Your coding agent" });
  fireEvent.change(picker, { target: { value: "claude-desktop" } });
  expect(
    await screen.findByText(
      /Add to ~\/Library\/Application Support\/Claude\/claude_desktop_config\.json/,
    ),
  ).toBeTruthy();
  expect(JSON.parse(screen.getByText(/"mcpServers"/).textContent!)).toEqual({
    mcpServers: { sayql: { command: PATH, args: [] } },
  });
  expect(screen.getByText(/Quit Claude Desktop fully/)).toBeTruthy();

  fireEvent.change(picker, { target: { value: "codex" } });
  expect(screen.getByText(`codex mcp add sayql -- ${PATH}`)).toBeTruthy();
  expect(screen.getByText(/\[mcp_servers\.sayql\]/)).toBeTruthy();
});

it("blocks and allows an app", async () => {
  apps = [{ name: "Cursor", blocked: false, approved: false, queries: 1, lastUsed: 2 }];
  render(<ClaudeIntegration connections={connections} provider={provider} suggestions={api([])} />);
  expect(await screen.findByText("Allowed")).toBeTruthy();

  fireEvent.click(screen.getByRole("button", { name: "Block Cursor" }));
  await waitFor(() => expect(setMcpAppBlocked).toHaveBeenCalledWith("Cursor", true));
  expect(await screen.findByRole("button", { name: "Allow Cursor" })).toBeTruthy();
  expect(screen.getByText("Blocked")).toBeTruthy();
});

it("only lets in approved apps once turned on, and approves the ones in use", async () => {
  apps = [{ name: "Claude Code", blocked: false, approved: false, queries: 3, lastUsed: 5 }];
  render(<ClaudeIntegration connections={connections} provider={provider} suggestions={api([])} />);
  const toggle = await screen.findByRole("checkbox", { name: "Only allow apps I've approved" });
  expect((toggle as HTMLInputElement).checked).toBe(false);
  expect(
    screen.getByText(/isn't a security boundary against software that lies about its name/),
  ).toBeTruthy();

  fireEvent.click(toggle);
  await waitFor(() => expect(setMcpOnlyApproved).toHaveBeenCalledWith(true));
  await waitFor(() => expect((toggle as HTMLInputElement).checked).toBe(true));
  // The app in use stays allowed.
  expect(screen.getByText("Allowed")).toBeTruthy();
  expect(screen.queryByText("Waiting for approval")).toBeNull();
});

it("shows an unapproved app as waiting, with Approve and Block", async () => {
  onlyApproved = true;
  apps = [
    { name: "Windsurf", blocked: false, approved: false, queries: 0, lastUsed: 9 },
    { name: "Cursor", blocked: true, approved: true, queries: 1, lastUsed: 2 },
  ];
  render(<ClaudeIntegration connections={connections} provider={provider} suggestions={api([])} />);
  expect(await screen.findByText("Waiting for approval")).toBeTruthy();
  expect(
    (
      (await screen.findByRole("checkbox", {
        name: "Only allow apps I've approved",
      })) as HTMLInputElement
    ).checked,
  ).toBe(true);
  // Blocked wins over approved: no Approve for Cursor.
  expect(screen.getByText("Blocked")).toBeTruthy();
  expect(screen.queryByRole("button", { name: "Approve Cursor" })).toBeNull();
  expect(screen.getByRole("button", { name: "Block Windsurf" })).toBeTruthy();

  fireEvent.click(screen.getByRole("button", { name: "Approve Windsurf" }));
  await waitFor(() => expect(approveMcpApp).toHaveBeenCalledWith("Windsurf"));
  expect(await screen.findByText("Allowed")).toBeTruthy();
  expect(screen.queryByText("Waiting for approval")).toBeNull();
  expect(screen.queryByRole("button", { name: "Approve Windsurf" })).toBeNull();
});

it("shows when each connection was last audited, nudges, and copies the audit prompt", async () => {
  const writeText = vi.fn(() => Promise.resolve());
  Object.assign(navigator, { clipboard: { writeText } });
  const two = [...connections, { id: "c2", name: "Warehouse", kind: "postgres" } as never];
  audits = [
    { connectionId: "c2", database: "sales", app: "Cursor", at: Date.now() - 3 * 86_400_000 },
  ];
  render(
    <ClaudeIntegration connections={two} provider={provider} suggestions={api([])} tab="access" />,
  );
  expect(await screen.findByRole("columnheader", { name: "Last audit" })).toBeTruthy();
  expect(await screen.findByText("3 days ago")).toBeTruthy();
  expect(screen.getByText("Never")).toBeTruthy();
  // Only the connection never audited is nudged.
  expect(screen.getAllByText("Run an audit")).toHaveLength(1);

  fireEvent.click(screen.getByRole("button", { name: "Copy the audit prompt for Shop" }));
  await waitFor(() => expect(writeText).toHaveBeenCalledWith("Audit c1"));
  expect(auditPrompt).toHaveBeenCalledWith("c1", null);
  expect(await screen.findByText("Copied")).toBeTruthy();
});

it("teaches the audit when no suggestions wait, with step 1 done once an app connected", async () => {
  const { unmount } = render(
    <ClaudeIntegration
      connections={connections}
      provider={provider}
      suggestions={api([])}
      tab="suggestions"
    />,
  );
  expect(screen.getByText("Teach SayQL your database")).toBeTruthy();
  expect(await screen.findByText("Connect Claude Code to SayQL.")).toBeTruthy();
  expect(await screen.findByText(`claude mcp add --scope user sayql -- ${PATH}`)).toBeTruthy();
  unmount();

  apps = [{ name: "Claude Code", blocked: false, approved: false, queries: 2, lastUsed: 5 }];
  render(
    <ClaudeIntegration
      connections={connections}
      provider={provider}
      suggestions={api([])}
      tab="suggestions"
    />,
  );
  expect(await screen.findByText("Claude Code is connected.")).toBeTruthy();
  expect(screen.queryByText(/claude mcp add/)).toBeNull();
  fireEvent.click(screen.getByRole("tab", { name: "Access" }));
});

it("links to the setup in Settings when the server's path is unknown", async () => {
  setupFails = true;
  render(
    <ClaudeIntegration
      connections={connections}
      provider={provider}
      suggestions={api([])}
      tab="suggestions"
    />,
  );
  // "Set it up" goes to the Access tab, where the setup is.
  fireEvent.click(await screen.findByRole("button", { name: "Set it up" }));
  expect(screen.getByRole("tab", { name: "Access" }).getAttribute("aria-selected")).toBe("true");
  expect(await screen.findByText(/Couldn't find the MCP server's path/)).toBeTruthy();
});
