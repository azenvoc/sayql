import { describe, expect, it } from "vitest";
import {
  AGENTS,
  agentById,
  agentForClient,
  base64,
  cursorLink,
  preselectAgent,
  promptArgs,
  shellQuote,
  vscodeLink,
  type AgentId,
} from "./agents";

const MAC = "/Applications/SayQL.app/Contents/MacOS/sayql-mcp";
const SPACED = "/Users/me/My Apps/SayQL.app/Contents/MacOS/sayql-mcp";
const WIN = "C:\\Program Files\\SayQL\\sayql-mcp.exe";
const setup = (id: AgentId, path = MAC, os: "macos" | "windows" | "linux" = "macos") =>
  agentById(id)!.setup({ path, os });

describe("shellQuote", () => {
  it("leaves plain paths alone and quotes the rest for the shell", () => {
    expect(shellQuote(MAC, "macos")).toBe(MAC);
    expect(shellQuote(SPACED, "macos")).toBe(`'${SPACED}'`);
    expect(shellQuote("/tmp/it's here", "linux")).toBe(`'/tmp/it'\\''s here'`);
    expect(shellQuote("/tmp/$HOME`x`", "macos")).toBe("'/tmp/$HOME`x`'");
    expect(shellQuote(WIN, "windows")).toBe(`"${WIN}"`);
  });
});

describe("setup commands", () => {
  it("are the documented commands with the path quoted", () => {
    expect(setup("claude-code").command).toBe(`claude mcp add --scope user sayql -- ${MAC}`);
    expect(setup("claude-code", SPACED).command).toBe(
      `claude mcp add --scope user sayql -- '${SPACED}'`,
    );
    expect(setup("claude-code", WIN, "windows").command).toBe(
      `claude mcp add --scope user sayql -- "${WIN}"`,
    );
    expect(setup("codex").command).toBe(`codex mcp add sayql -- ${MAC}`);
    expect(setup("gemini", SPACED).command).toBe(`gemini mcp add -s user sayql '${SPACED}'`);
    expect(setup("copilot-cli").command).toBe(`copilot mcp add sayql -- ${MAC}`);
    expect(setup("amp").command).toBe(`amp mcp add sayql -- ${MAC}`);
  });

  it("gives VS Code's --add-mcp its JSON in single quotes, and no command on Windows", () => {
    expect(setup("vscode").command).toBe(
      `code --add-mcp '{"name":"sayql","type":"stdio","command":"${MAC}"}'`,
    );
    expect(setup("vscode", "/x/it's").command).toBe(
      `code --add-mcp '{"name":"sayql","type":"stdio","command":"/x/it'\\''s"}'`,
    );
    expect(setup("vscode", WIN, "windows").command).toBeUndefined();
  });

  it("has no one-line command for agents that need a config file", () => {
    for (const id of [
      "cursor",
      "opencode",
      "windsurf",
      "zed",
      "cline",
      "roo",
      "continue",
      "jetbrains",
      "claude-desktop",
      "other",
    ] as AgentId[]) {
      expect(setup(id).command, id).toBeUndefined();
      expect(setup(id).config, id).toBeDefined();
    }
  });
});

describe("config snippets", () => {
  it("escape Windows paths in JSON and TOML", () => {
    const json = JSON.parse(setup("claude-desktop", WIN, "windows").config!.text);
    expect(json).toEqual({ mcpServers: { sayql: { command: WIN, args: [] } } });
    expect(setup("codex", WIN, "windows").config!.text).toBe(
      `[mcp_servers.sayql]\ncommand = "C:\\\\Program Files\\\\SayQL\\\\sayql-mcp.exe"\nargs = []`,
    );
    expect(setup("codex", 'a"b').config!.text).toContain('command = "a\\"b"');
  });

  it("use each app's own keys", () => {
    expect(JSON.parse(setup("vscode").config!.text)).toEqual({
      servers: { sayql: { type: "stdio", command: MAC, args: [] } },
    });
    expect(JSON.parse(setup("zed").config!.text)).toEqual({
      context_servers: { sayql: { command: MAC, args: [], env: {} } },
    });
    expect(JSON.parse(setup("opencode").config!.text).mcp).toEqual({
      sayql: { type: "local", command: [MAC], enabled: true },
    });
    expect(JSON.parse(setup("copilot-cli").config!.text).mcpServers.sayql.type).toBe("local");
    expect(Object.keys(JSON.parse(setup("amp").config!.text))).toEqual(["amp.mcpServers"]);
    for (const a of AGENTS) {
      const config = a.setup({ path: SPACED, os: "linux" }).config;
      if (config?.format === "json") expect(config.text, a.id).toContain(SPACED);
    }
  });

  it("point at the file for this OS", () => {
    expect(setup("claude-desktop").config!.file).toBe(
      "~/Library/Application Support/Claude/claude_desktop_config.json",
    );
    expect(setup("claude-desktop", WIN, "windows").config!.file).toBe(
      "%APPDATA%\\Claude\\claude_desktop_config.json",
    );
    // No documented path on Linux: open it from the app.
    expect(setup("claude-desktop", MAC, "linux").config!.file).toBeNull();
    expect(setup("claude-desktop", MAC, "linux").config!.open).toContain("Edit Config");
    expect(setup("windsurf").config!.file).toBe("~/.config/devin/mcp_config.json");
    expect(setup("windsurf", WIN, "windows").config!.file).toBe(
      "%APPDATA%\\devin\\mcp_config.json",
    );
    expect(setup("windsurf").also).toContain("~/.codeium/windsurf/mcp_config.json");
    expect(setup("cursor").config!.file).toBe("~/.cursor/mcp.json");
  });
});

describe("install links", () => {
  it("Cursor's carries the server config as base64 JSON", () => {
    const link = cursorLink(MAC);
    expect(
      link.startsWith("cursor://anysphere.cursor-deeplink/mcp/install?name=sayql&config="),
    ).toBe(true);
    const config = new URL(link).searchParams.get("config")!;
    // The docs' own example for this path.
    expect(config).toBe(
      "eyJjb21tYW5kIjoiL0FwcGxpY2F0aW9ucy9TYXlRTC5hcHAvQ29udGVudHMvTWFjT1Mvc2F5cWwtbWNwIn0=",
    );
    expect(JSON.parse(atob(config))).toEqual({ command: MAC });
  });

  it("Cursor's survives spaces, backslashes and non-Latin paths", () => {
    for (const path of [SPACED, WIN, "/Users/jürgen/Programme/sayql-mcp"]) {
      const config = new URL(cursorLink(path)).searchParams.get("config")!;
      expect(config).toBe(base64(JSON.stringify({ command: path })));
      const bytes = Uint8Array.from(atob(config), (c) => c.charCodeAt(0));
      expect(JSON.parse(new TextDecoder().decode(bytes))).toEqual({ command: path });
      expect(cursorLink(path)).not.toMatch(/[ +]/);
    }
  });

  it("VS Code's carries URL-encoded JSON", () => {
    const link = vscodeLink(SPACED);
    expect(link.startsWith("vscode:mcp/install?%7B%22name%22%3A%22sayql%22")).toBe(true);
    expect(link).not.toMatch(/[ "{}]/);
    expect(JSON.parse(decodeURIComponent(link.slice("vscode:mcp/install?".length)))).toEqual({
      name: "sayql",
      type: "stdio",
      command: SPACED,
    });
  });

  it("only Cursor and VS Code have one", () => {
    expect(AGENTS.filter((a) => a.setup({ path: MAC, os: "macos" }).link).map((a) => a.id)).toEqual(
      ["cursor", "vscode"],
    );
  });
});

describe("running the audit", () => {
  const target = { connectionId: "c1", name: "Shop", database: "main" };
  it("only Claude Code can be run by SayQL", () => {
    expect(AGENTS.filter((a) => a.runnable).map((a) => a.id)).toEqual(["claude-code"]);
    expect(AGENTS.filter((a) => a.audit.kind === "run").map((a) => a.id)).toEqual(["claude-code"]);
  });

  it("uses each agent's documented prompt command", () => {
    const command = (id: AgentId) => {
      const how = agentById(id)!.audit;
      return how.kind === "run" || how.kind === "prompt" ? how.command(target) : null;
    };
    expect(command("claude-code")).toBe("/mcp__sayql__audit Shop main");
    expect(command("vscode")).toBe("/mcp.sayql.audit");
    expect(command("opencode")).toBe("/sayql:audit Shop main");
    expect(command("gemini")).toBe("/audit Shop main");
    expect(command("cursor")).toBeNull();
  });

  it("has agents without documented prompts paste the prompt", () => {
    const paste = AGENTS.filter((a) => a.audit.kind === "paste").map((a) => a.id);
    expect(paste).toEqual([
      "codex",
      "copilot-cli",
      "windsurf",
      "zed",
      "cline",
      "roo",
      "amp",
      "jetbrains",
      "other",
    ]);
  });

  it("names a connection with spaces by id", () => {
    expect(promptArgs(null)).toBe("");
    expect(promptArgs({ connectionId: "c9", name: "My shop", database: null })).toBe(" c9");
    expect(promptArgs({ connectionId: "c9", name: "Shop", database: "my db" })).toBe(" Shop");
  });
});

describe("agentForClient", () => {
  it("matches the names apps report over MCP", () => {
    expect(agentForClient("Claude Code")?.id).toBe("claude-code");
    expect(agentForClient("Claude")?.id).toBe("claude-desktop");
    expect(agentForClient("Cursor")?.id).toBe("cursor");
    expect(agentForClient("Visual Studio Code")?.id).toBe("vscode");
    expect(agentForClient("codex-mcp-client")?.id).toBe("codex");
    expect(agentForClient("gemini-cli-mcp-client")?.id).toBe("gemini");
    expect(agentForClient("Windsurf")?.id).toBe("windsurf");
    expect(agentForClient("SayQL Support")).toBeUndefined();
  });
});

describe("preselectAgent", () => {
  const app = (name: string, lastUsed: number | null) => ({ name, lastUsed });
  it("keeps the person's pick", () => {
    expect(preselectAgent({ saved: "zed", installed: ["cursor"], apps: [app("Cursor", 5)] })).toBe(
      "zed",
    );
  });
  it("ignores a pick SayQL no longer knows", () => {
    expect(preselectAgent({ saved: "gone", installed: [], apps: [] })).toBe("claude-code");
  });
  it("prefers the installed agent that connected last", () => {
    const apps = [app("Cursor", 5), app("Visual Studio Code", 9), app("Codex", 20)];
    expect(preselectAgent({ installed: ["claude-code", "cursor", "vscode"], apps })).toBe("vscode");
  });
  it("else the first installed, in the list's order", () => {
    expect(preselectAgent({ installed: ["claude-desktop", "vscode", "cursor"], apps: [] })).toBe(
      "cursor",
    );
  });
  it("else the app that connected last", () => {
    expect(
      preselectAgent({
        installed: [],
        apps: [app("Cursor", 5), app("Zed", 7), app("Unknown", 9), app("Codex", null)],
      }),
    ).toBe("zed");
  });
  it("else Claude Code", () => {
    expect(preselectAgent({ installed: [], apps: [] })).toBe("claude-code");
  });
});
