// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { McpApp } from "../../api";
import { TeachSayql } from "./TeachSayql";
import { CODING_AGENT_KEY, forgetInstalledAgents } from "./useCodingAgent";

const PATH = "/Applications/SayQL.app/Contents/MacOS/sayql-mcp";
let apps: McpApp[] = [];
let installed: string[] = [];
const openInstallLink = vi.fn((_url: string) => Promise.resolve());
const auditPrompt = vi.fn((id: string, _db: string | null) => Promise.resolve(`Audit ${id}`));
vi.mock("../../api", async (orig) => ({
  ...(await orig<typeof import("../../api")>()),
  commands: {
    mcpApps: () => Promise.resolve({ onlyApproved: false, apps }),
    mcpSetup: () => Promise.resolve({ path: PATH, exists: true, appimage: false, os: "macos" }),
    installedCodingAgents: () => Promise.resolve(installed),
    openInstallLink: (url: string) => openInstallLink(url),
    auditPrompt: (id: string, db: string | null) => auditPrompt(id, db),
    auditAgents: () =>
      Promise.resolve([
        {
          id: "claude",
          label: "Claude Code",
          path: "/bin/claude",
          version: "2.1.267",
          supported: true,
          ready: true,
          problem: null,
          installUrl: "",
        },
      ]),
  },
}));

const writeText = vi.fn((_t: string) => Promise.resolve());
const target = { connectionId: "c1", name: "Shop", database: null };

beforeEach(() => {
  apps = [];
  installed = [];
  localStorage.clear();
  forgetInstalledAgents();
  writeText.mockClear();
  openInstallLink.mockClear();
  Object.assign(navigator, { clipboard: { writeText } });
});
afterEach(cleanup);

const pick = (id: string) =>
  fireEvent.change(screen.getByRole("combobox", { name: "Your coding agent" }), {
    target: { value: id },
  });

it("defaults to Claude Code: the setup command, Run audit and the slash command", async () => {
  render(<TeachSayql target={target} />);
  expect(await screen.findByText("Connect Claude Code to SayQL.")).toBeTruthy();
  expect(await screen.findByText(`claude mcp add --scope user sayql -- ${PATH}`)).toBeTruthy();
  expect(await screen.findByRole("button", { name: /^Run audit/ })).toBeTruthy();
  expect(screen.getByText("/mcp__sayql__audit Shop")).toBeTruthy();
  expect(screen.getByRole("button", { name: "Copy audit prompt" })).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: "Copy the setup command" }));
  await waitFor(() =>
    expect(writeText).toHaveBeenCalledWith(`claude mcp add --scope user sayql -- ${PATH}`),
  );
});

it("preselects an installed agent and remembers a pick", async () => {
  installed = ["cursor", "vscode"];
  const { unmount } = render(<TeachSayql target={target} />);
  expect(await screen.findByText("Connect Cursor to SayQL.")).toBeTruthy();
  pick("codex");
  expect(screen.getByText("Connect Codex to SayQL.")).toBeTruthy();
  expect(localStorage.getItem(CODING_AGENT_KEY)).toBe('"codex"');
  unmount();
  render(<TeachSayql target={target} />);
  expect(await screen.findByText("Connect Codex to SayQL.")).toBeTruthy();
});

it("Cursor: Open in Cursor, the config behind a disclosure, and the / menu", async () => {
  localStorage.setItem(CODING_AGENT_KEY, '"cursor"');
  render(<TeachSayql target={target} />);
  fireEvent.click(await screen.findByRole("button", { name: "Open in Cursor" }));
  await waitFor(() =>
    expect(openInstallLink).toHaveBeenCalledWith(
      expect.stringMatching(
        /^cursor:\/\/anysphere\.cursor-deeplink\/mcp\/install\?name=sayql&config=/,
      ),
    ),
  );
  expect(screen.queryByText(/"mcpServers"/)).toBeNull();
  fireEvent.click(screen.getByRole("button", { name: "Or edit the config file" }));
  expect(screen.getByText(/"mcpServers"/)).toBeTruthy();
  expect(screen.getByText(/type \/ and pick SayQL's audit/)).toBeTruthy();
  // Only Claude Code is run by SayQL.
  expect(screen.queryByRole("button", { name: /^Run audit/ })).toBeNull();
  expect(screen.queryByText(/mcp__sayql__audit/)).toBeNull();
  expect(screen.getByRole("button", { name: "Copy audit prompt" })).toBeTruthy();
});

it("VS Code: its own slash command for the prompt", async () => {
  localStorage.setItem(CODING_AGENT_KEY, '"vscode"');
  render(<TeachSayql target={target} />);
  expect(await screen.findByRole("button", { name: "Open in VS Code" })).toBeTruthy();
  expect(screen.getByText("/mcp.sayql.audit")).toBeTruthy();
  expect(screen.getByText(/In Copilot Chat, in agent mode, type/)).toBeTruthy();
});

it("Codex: no MCP prompts, so the audit prompt is pasted", async () => {
  localStorage.setItem(CODING_AGENT_KEY, '"codex"');
  render(<TeachSayql target={target} />);
  expect(await screen.findByText(`codex mcp add sayql -- ${PATH}`)).toBeTruthy();
  expect(screen.getByText(/Copy the audit prompt and paste it into Codex/)).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: "Copy audit prompt" }));
  await waitFor(() => expect(writeText).toHaveBeenCalledWith("Audit c1"));
  expect(screen.queryByRole("button", { name: /^Run audit/ })).toBeNull();
});

it("Other: the generic config to copy, and paste the prompt", async () => {
  localStorage.setItem(CODING_AGENT_KEY, '"other"');
  render(<TeachSayql target={target} />);
  expect(await screen.findByText("Connect your app to SayQL.")).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: "Copy config" }));
  await waitFor(() =>
    expect(JSON.parse(writeText.mock.calls[0][0])).toEqual({
      mcpServers: { sayql: { command: PATH, args: [] } },
    }),
  );
  expect(screen.getByText(/paste it into your app's chat/)).toBeTruthy();
});

it("says so when the picked agent is the one that connected", async () => {
  apps = [{ name: "Cursor", blocked: false, approved: false, queries: 1, lastUsed: 5 }];
  render(<TeachSayql target={target} />);
  expect(await screen.findByText("Cursor is connected.")).toBeTruthy();
  pick("zed");
  expect(screen.getByText("An AI app is connected.")).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: "Connect Zed too" }));
  expect(screen.getByText(/zed: open settings file/)).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: "Show the config" }));
  expect(screen.getByText(/context_servers/)).toBeTruthy();
});
