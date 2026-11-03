import type { McpApp, McpSetup } from "../../api";
import type { AuditTarget } from "./audit";

/**
 * The coding agents SayQL knows how to connect, and how each runs SayQL's audit. Every fact here
 * was checked against the agent's own docs (2026-10-28). Where a docs page doesn't say how
 * something looks in the app, the wording stays generic or falls back to pasting the prompt.
 * The ids match src-tauri/src/coding_agents.rs, which finds the installed ones.
 */

export type Os = McpSetup["os"];

export type AgentId =
  | "claude-code"
  | "cursor"
  | "vscode"
  | "codex"
  | "gemini"
  | "copilot-cli"
  | "opencode"
  | "windsurf"
  | "zed"
  | "cline"
  | "roo"
  | "continue"
  | "amp"
  | "jetbrains"
  | "claude-desktop"
  | "other";

/** Where SayQL's MCP server is on this computer, and which OS it's for. */
export interface SetupContext {
  path: string;
  os: Os;
}

/** A config file to add SayQL to. */
export interface ConfigSetup {
  /** The file, for this OS; null when the app opens it for you (see `open`). */
  file: string | null;
  /** How to open it from the app. */
  open?: string;
  format: "json" | "toml";
  text: string;
}

/** The ways to add SayQL to an agent, for this OS. At least one of command, link and config. */
export interface AgentSetup {
  /** A command to run in a terminal. */
  command?: string;
  /** The app's own link that adds an MCP server (it asks first). */
  link?: string;
  config?: ConfigSetup;
  /** Another place the config can go, in a sentence. */
  also?: string;
  /** What to do after adding it. */
  after?: string;
}

/** How the person runs the audit in the agent. */
export type AuditHow =
  /** SayQL can run it, read-only; and it has the prompt as a command. */
  | { kind: "run"; where: string; command: (target?: AuditTarget | null) => string }
  /** Documented: the MCP prompt as a slash command. */
  | { kind: "prompt"; where: string; command: (target?: AuditTarget | null) => string }
  /** Prompts are supported, but the docs don't show the menu: worded generically. */
  | { kind: "menu"; text: string }
  /** No MCP prompts documented: paste the copied audit prompt. */
  | { kind: "paste"; where: string };

export interface CodingAgent {
  id: AgentId;
  /** The product's name, exactly. */
  name: string;
  /** In sentences ("Connect VS Code to SayQL"). */
  short: string;
  setup: (ctx: SetupContext) => AgentSetup;
  audit: AuditHow;
  /** SayQL can run the audit with it, read-only (only Claude Code passes that bar). */
  runnable: boolean;
  /** The names it reports over MCP, as in Settings → AI apps → Access → Apps. */
  clients: RegExp;
  /** A note for the card, when the agent is a poor fit for the audit. */
  caveat?: string;
}

/** A command-line argument, quoted for the person's shell when it needs to be. */
export function shellQuote(arg: string, os: Os): string {
  if (/^[A-Za-z0-9_/.:+-]+$/.test(arg)) return arg;
  // Windows paths can't hold a double quote; PowerShell and cmd both read "…".
  if (os === "windows") return `"${arg}"`;
  return `'${arg.replace(/'/g, `'\\''`)}'`;
}

/** Pretty JSON, as a config file holds it. */
function json(value: unknown): string {
  return JSON.stringify(value, null, 2);
}

/** Base64 of the UTF-8 bytes (btoa alone takes only Latin-1). */
export function base64(text: string): string {
  let bin = "";
  for (const b of new TextEncoder().encode(text)) bin += String.fromCharCode(b);
  return btoa(bin);
}

/** Cursor's install link: the server's config as base64 JSON, without the name. */
export function cursorLink(path: string): string {
  return `cursor://anysphere.cursor-deeplink/mcp/install?name=sayql&config=${encodeURIComponent(base64(JSON.stringify({ command: path })))}`;
}

/** VS Code's install link: the server's config, name included, as URL-encoded JSON. */
export function vscodeLink(path: string): string {
  return `vscode:mcp/install?${encodeURIComponent(JSON.stringify({ name: "sayql", type: "stdio", command: path }))}`;
}

/**
 * The prompt's arguments after a slash command: the connection's name, or its id when the name
 * has spaces (words go in order: connection, then database), and the database when it has none.
 */
export function promptArgs(target?: AuditTarget | null): string {
  if (!target) return "";
  const name = target.name.trim();
  const connection = !name || /\s/.test(name) ? target.connectionId : name;
  const database = target.database && !/\s/.test(target.database) ? ` ${target.database}` : "";
  return ` ${connection}${database}`;
}

/** A file path per OS. */
const byOs = (os: Os, paths: Partial<Record<Os, string>>, fallback: string | null = null) =>
  paths[os] ?? fallback;

/** The usual `mcpServers` JSON, which most apps read. */
const mcpServers = (path: string, extra: Record<string, unknown> = {}) =>
  json({ mcpServers: { sayql: { command: path, args: [], ...extra } } });

export const AGENTS: CodingAgent[] = [
  {
    id: "claude-code",
    name: "Claude Code",
    short: "Claude Code",
    setup: ({ path, os }) => ({
      command: `claude mcp add --scope user sayql -- ${shellQuote(path, os)}`,
    }),
    audit: {
      kind: "run",
      where: "Claude Code",
      command: (t) => `/mcp__sayql__audit${promptArgs(t)}`,
    },
    runnable: true,
    clients: /claude[ -]code/i,
  },
  {
    id: "cursor",
    name: "Cursor",
    short: "Cursor",
    setup: ({ path }) => ({
      link: cursorLink(path),
      config: { file: "~/.cursor/mcp.json", format: "json", text: mcpServers(path) },
      also: "Or .cursor/mcp.json in your project.",
    }),
    audit: { kind: "menu", text: "In Cursor's Agent chat, type / and pick SayQL's audit." },
    runnable: false,
    clients: /cursor/i,
  },
  {
    id: "vscode",
    name: "VS Code (GitHub Copilot)",
    short: "VS Code",
    setup: ({ path, os }) => ({
      link: vscodeLink(path),
      command:
        os === "windows"
          ? undefined
          : `code --add-mcp ${shellQuote(JSON.stringify({ name: "sayql", type: "stdio", command: path }), os)}`,
      config: {
        file: null,
        open: "Run MCP: Open User Configuration from the Command Palette",
        format: "json",
        text: json({ servers: { sayql: { type: "stdio", command: path, args: [] } } }),
      },
      also: "Or .vscode/mcp.json in your project.",
    }),
    audit: {
      kind: "prompt",
      where: "Copilot Chat, in agent mode",
      command: () => "/mcp.sayql.audit",
    },
    runnable: false,
    clients: /visual studio code|vs ?code/i,
  },
  {
    id: "codex",
    name: "Codex CLI",
    short: "Codex",
    setup: ({ path, os }) => ({
      command: `codex mcp add sayql -- ${shellQuote(path, os)}`,
      config: {
        file: "~/.codex/config.toml",
        format: "toml",
        text: `[mcp_servers.sayql]\ncommand = ${JSON.stringify(path)}\nargs = []`,
      },
    }),
    audit: { kind: "paste", where: "Codex, started in your project" },
    runnable: false,
    clients: /codex/i,
  },
  {
    id: "gemini",
    name: "Gemini CLI",
    short: "Gemini CLI",
    setup: ({ path, os }) => ({
      command: `gemini mcp add -s user sayql ${shellQuote(path, os)}`,
      config: { file: "~/.gemini/settings.json", format: "json", text: mcpServers(path) },
    }),
    audit: {
      kind: "prompt",
      where: "Gemini CLI, in your project",
      command: (t) => `/audit${promptArgs(t)}`,
    },
    runnable: false,
    clients: /gemini/i,
  },
  {
    id: "copilot-cli",
    name: "GitHub Copilot CLI",
    short: "Copilot CLI",
    setup: ({ path, os }) => ({
      command: `copilot mcp add sayql -- ${shellQuote(path, os)}`,
      config: {
        file: "~/.copilot/mcp-config.json",
        format: "json",
        text: json({
          mcpServers: { sayql: { type: "local", command: path, args: [], tools: ["*"] } },
        }),
      },
    }),
    audit: { kind: "paste", where: "Copilot CLI, started in your project" },
    runnable: false,
    clients: /copilot/i,
  },
  {
    id: "opencode",
    name: "opencode",
    short: "opencode",
    setup: ({ path }) => ({
      config: {
        file: "~/.config/opencode/opencode.json",
        format: "json",
        text: json({
          $schema: "https://opencode.ai/config.json",
          mcp: { sayql: { type: "local", command: [path], enabled: true } },
        }),
      },
      also: "Or opencode.json in your project.",
    }),
    audit: {
      kind: "prompt",
      where: "opencode, in your project",
      command: (t) => `/sayql:audit${promptArgs(t)}`,
    },
    runnable: false,
    clients: /opencode/i,
  },
  {
    id: "windsurf",
    name: "Windsurf / Devin Desktop",
    short: "Devin Desktop",
    setup: ({ path, os }) => ({
      config: {
        file: byOs(
          os,
          { windows: "%APPDATA%\\devin\\mcp_config.json" },
          "~/.config/devin/mcp_config.json",
        ),
        format: "json",
        text: mcpServers(path),
      },
      also: "Older Windsurf versions read ~/.codeium/windsurf/mcp_config.json.",
    }),
    audit: { kind: "paste", where: "Cascade" },
    runnable: false,
    clients: /windsurf|devin|cascade/i,
  },
  {
    id: "zed",
    name: "Zed",
    short: "Zed",
    setup: ({ path }) => ({
      config: {
        file: null,
        open: "Run zed: open settings file from the command palette",
        format: "json",
        text: json({ context_servers: { sayql: { command: path, args: [], env: {} } } }),
      },
    }),
    audit: { kind: "paste", where: "Zed's Agent Panel" },
    runnable: false,
    clients: /^zed/i,
  },
  {
    id: "cline",
    name: "Cline",
    short: "Cline",
    setup: ({ path }) => ({
      config: {
        file: null,
        open: "In Cline, click MCP Servers, then Configure, then Configure MCP Servers",
        format: "json",
        text: mcpServers(path, { disabled: false, autoApprove: [] }),
      },
    }),
    audit: { kind: "paste", where: "Cline's chat" },
    runnable: false,
    clients: /^cline/i,
  },
  {
    id: "roo",
    name: "Roo Code",
    short: "Roo Code",
    setup: ({ path }) => ({
      config: {
        file: null,
        open: "In Roo Code's MCP settings, click Edit Global MCP",
        format: "json",
        text: mcpServers(path, { alwaysAllow: [], disabled: false }),
      },
      also: "Or .roo/mcp.json in your project.",
    }),
    audit: { kind: "paste", where: "Roo Code's chat" },
    runnable: false,
    clients: /roo/i,
  },
  {
    id: "continue",
    name: "Continue",
    short: "Continue",
    setup: ({ path }) => ({
      config: {
        file: ".continue/mcpServers/sayql.json (in your project)",
        format: "json",
        text: mcpServers(path),
      },
    }),
    audit: { kind: "menu", text: "In Continue's chat, type / and pick SayQL's audit." },
    runnable: false,
    clients: /continue/i,
  },
  {
    id: "amp",
    name: "Amp",
    short: "Amp",
    setup: ({ path, os }) => ({
      command: `amp mcp add sayql -- ${shellQuote(path, os)}`,
      config: {
        file: byOs(
          os,
          { windows: "%USERPROFILE%\\.config\\amp\\settings.json" },
          "~/.config/amp/settings.json",
        ),
        format: "json",
        text: json({ "amp.mcpServers": { sayql: { command: path, args: [] } } }),
      },
    }),
    audit: { kind: "paste", where: "Amp, started in your project" },
    runnable: false,
    clients: /^amp\b/i,
  },
  {
    id: "jetbrains",
    name: "JetBrains AI Assistant",
    short: "AI Assistant",
    setup: ({ path }) => ({
      config: {
        file: null,
        open: "Settings → Tools → AI Assistant → Model Context Protocol (MCP) → Add, choose STDIO and paste this",
        format: "json",
        text: mcpServers(path),
      },
    }),
    audit: { kind: "paste", where: "AI Assistant's chat" },
    runnable: false,
    clients: /jetbrains|intellij|pycharm|webstorm|goland|phpstorm|rider|rubymine|clion|datagrip/i,
  },
  {
    id: "claude-desktop",
    name: "Claude Desktop",
    short: "Claude Desktop",
    setup: ({ path, os }) => ({
      config: {
        file: byOs(os, {
          macos: "~/Library/Application Support/Claude/claude_desktop_config.json",
          windows: "%APPDATA%\\Claude\\claude_desktop_config.json",
        }),
        open: "Settings → Developer → Edit Config",
        format: "json",
        text: mcpServers(path),
      },
      after: "Quit Claude Desktop fully and open it again.",
    }),
    audit: {
      kind: "menu",
      text: "In a chat, pick SayQL's audit from the menu that lists your connectors.",
    },
    runnable: false,
    clients: /^claude$|claude-ai/i,
    caveat:
      "Claude Desktop can't read your project's code on its own, so a coding agent audits better.",
  },
  {
    id: "other",
    name: "Other (any MCP app)",
    short: "your app",
    setup: ({ path }) => ({
      config: {
        file: null,
        open: "Add a stdio server to your app's MCP settings",
        format: "json",
        text: mcpServers(path),
      },
    }),
    audit: { kind: "paste", where: "your app's chat" },
    runnable: false,
    clients: /$^/,
  },
];

export const DEFAULT_AGENT: AgentId = "claude-code";

export function agentById(id: string | null | undefined): CodingAgent | undefined {
  return AGENTS.find((a) => a.id === id);
}

/** The agent an MCP app's reported name belongs to, if SayQL knows it. */
export function agentForClient(name: string): CodingAgent | undefined {
  return AGENTS.find((a) => a.clients.test(name.trim()));
}

/**
 * The agent to show first: the one the person picked before; else the most recent app that
 * connected over MCP and is installed; else the first installed (in the list's order); else the
 * most recent app that connected; else Claude Code.
 */
export function preselectAgent({
  saved,
  installed,
  apps,
}: {
  saved?: string | null;
  installed: readonly string[];
  apps: readonly Pick<McpApp, "name" | "lastUsed">[];
}): AgentId {
  const picked = agentById(saved);
  if (picked) return picked.id;
  const connected = [...apps]
    .filter((a) => a.lastUsed != null)
    .sort((x, y) => (y.lastUsed ?? 0) - (x.lastUsed ?? 0))
    .map((a) => agentForClient(a.name)?.id)
    .filter((id): id is AgentId => !!id);
  const found = AGENTS.filter((a) => installed.includes(a.id)).map((a) => a.id);
  return connected.find((id) => found.includes(id)) ?? found[0] ?? connected[0] ?? DEFAULT_AGENT;
}
