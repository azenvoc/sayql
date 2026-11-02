import { beforeAll, describe, expect, it, vi } from "vitest";
import pkg from "../../package.json";
import commandsSource from "../api/commands.ts?raw";
import { isReadOnly } from "./readOnly";

type Handler = (cmd: string, args?: Record<string, unknown>) => Promise<unknown>;
let handler: Handler;

vi.mock("@tauri-apps/api/mocks", () => ({ mockIPC: (cb: Handler) => (handler = cb) }));
vi.mock("@tauri-apps/api/event", () => ({ emit: vi.fn() }));

beforeAll(async () => {
  vi.stubGlobal("window", { location: { search: "" }, open: vi.fn() });
  (await import("./mockBackend")).installMockBackend();
});

// Every command src/api/commands.ts invokes, so the dev UI never hits the throwing default.
const COMMANDS = [
  "init", "save_connection", "delete_connection", "test_connection", "open_database", "refresh_schema",
  "list_databases", "run_query", "cancel_query", "generate_sql", "cancel_generation", "check_claude_plan", "my_activity", "clear_my_activity", "list_scripts", "save_script",
  "delete_script", "list_skills", "save_skill", "delete_skill", "mcp_setup", "installed_coding_agents", "open_install_link", "mcp_activity", "mcp_apps", "set_mcp_app_blocked", "approve_mcp_app", "set_mcp_only_approved", "clear_mcp_activity", "mcp_audits", "audit_prompt", "audit_agents", "knowledge_sources", "add_knowledge_folder", "add_knowledge_repo", "add_knowledge_files", "add_knowledge_url", "remove_knowledge_source", "refresh_knowledge_source", "start_audit", "cancel_audit", "audit_model", "set_audit_model", "audit_payment", "set_audit_payment", "open_audit_in_terminal", "take_handoffs", "app_info", "check_update",
  "install_update", "save_settings", "finish_onboarding", "list_models", "voice_status", "voice_download",
  "voice_cancel_download", "voice_delete_model", "voice_start", "voice_stop", "voice_cancel",
  "agent_access", "save_agent_access", "start_agent", "reply_agent", "stop_agent", "list_notes", "list_shared_notes", "suggest_note", "save_note", "delete_note", "check_note", "check_notes", "confirm_note",
  "list_note_suggestions", "accept_note_suggestion", "dismiss_note_suggestion",
  "save_agent_settings", "choose_model", "credits_balance", "team_sign_in", "team_sso_start", "team_sso_finish", "team_status", "team_sign_out", "team_connections", "team_ai",
];

describe("mock backend", () => {
  it("throws for a command it doesn't know", async () => {
    await expect(handler("no_such_command")).rejects.toThrow('no mock for command "no_such_command"');
  });

  it("has a mock for every command the app invokes", async () => {
    const invoked = [...commandsSource.matchAll(/invoke<[^>]*>\("(\w+)"/g)].map((m) => m[1]);
    expect(invoked.sort()).toEqual([...COMMANDS].sort());
    const args: Record<string, Record<string, unknown>> = {
      save_connection: { config: { id: "x" } },
      run_query: { sql: "SELECT 1", allowWrite: false },
      generate_sql: { reqId: 1, question: "q", task: { kind: "draft" }, skill: null },
      save_script: { script: { id: "s" } },
      save_skill: { skill: { slug: "a", description: "d", instruction: "i" }, original: null },
      save_settings: { provider: "anthropic", typingModel: "m", expertModel: "m", baseUrl: "" },
      save_agent_access: { id: "demo", access: { enabled: true, results: "counts", apps: "counts", askBeforeSend: false, neverSend: [] } },
      start_agent: { id: "demo", runId: "r1", question: "Why did paid orders drop?" },
      reply_agent: { runId: "r1", reply: { type: "go", plan: null } },
      stop_agent: { runId: "r1" },
      choose_model: { provider: "anthropic", model: "claude-sonnet-5-5", purpose: "agent" },
      set_mcp_app_blocked: { name: "Cursor", blocked: true },
      approve_mcp_app: { name: "Windsurf" },
      set_mcp_only_approved: { on: true },
      accept_note_suggestion: { id: "sug-1" },
      dismiss_note_suggestion: { id: "sug-2" },
      audit_prompt: { id: "demo", database: null },
      open_install_link: { url: "vscode:mcp/install?%7B%7D" },
      add_knowledge_repo: { id: "demo", url: "https://github.com/acme/shop.git", branch: "", subdir: "", token: null, tokenUser: null },
      refresh_knowledge_source: { id: "pg", sourceId: "src_api" },
      add_knowledge_url: { id: "demo", url: "https://docs.acme.com/db/", crawl: true },
      start_audit: { runId: "a1", agent: "claude", id: "demo", database: null },
      cancel_audit: { runId: "a1" },
      save_note: { note: { id: "", connectionId: "demo", database: null, text: "status 'paid' = not shipped", updatedAt: 0 } },
    };
    for (const cmd of COMMANDS) {
      if (["install_update", "team_sign_in", "team_sso_start", "team_sso_finish"].includes(cmd)) continue;
      await handler(cmd, args[cmd] ?? {}); // rejects (failing the test) for an unmocked command
    }
  });

  it("deletes a connection", async () => {
    await handler("delete_connection", { id: "pg" });
    const boot = (await handler("init")) as { connections: { id: string }[] };
    expect(boot.connections.map((c) => c.id)).not.toContain("pg");
  });

  it("cancels a generation and refuses to install updates", async () => {
    await expect(handler("cancel_generation", { slot: "tab-1" })).resolves.toBeNull();
    await expect(handler("install_update")).rejects.toMatch(/browser preview/);
  });

  it("reports the app's real version", async () => {
    expect(await handler("app_info")).toMatchObject({ version: pkg.version });
  });

  it("asks before writes, like the backend", async () => {
    await expect(handler("run_query", { sql: "SELECT 1; DELETE FROM orders", allowWrite: false })).rejects.toBe("WRITE_CONFIRM");
    await expect(handler("run_query", { sql: "SELECT 1; DELETE FROM orders", allowWrite: true })).resolves.toMatchObject({ rowsAffected: 1 });
    await expect(handler("run_query", { sql: "SELECT * FROM audit WHERE action = 'DELETE'", allowWrite: false })).resolves.toMatchObject({ rowsAffected: null });
  });
});

// The cases from the tests in src-tauri/src/db/classify.rs.
describe("isReadOnly matches classify.rs", () => {
  it("classifies read-only statements", () => {
    expect(isReadOnly("select 1")).toBe(true);
    expect(isReadOnly("  -- top customers\nSELECT * FROM customers")).toBe(true);
    expect(isReadOnly("/* c */ WITH x AS (SELECT 1) SELECT * FROM x")).toBe(true);
    expect(isReadOnly("(SELECT 1) UNION (SELECT 2)")).toBe(true);
    expect(isReadOnly("PRAGMA table_info(orders)")).toBe(true);
    expect(isReadOnly("PRAGMA journal_mode = WAL")).toBe(false);
    expect(isReadOnly("DELETE FROM orders")).toBe(false);
    expect(isReadOnly("-- select\nUPDATE orders SET status = 'paid'")).toBe(false);
    expect(isReadOnly("DROP TABLE x")).toBe(false);
  });

  it("needs confirmation for write keywords anywhere", () => {
    expect(isReadOnly("SELECT 1; DELETE FROM orders")).toBe(false);
    expect(isReadOnly("SELECT * INTO backup_orders FROM orders")).toBe(false);
    expect(isReadOnly("WITH gone AS (DELETE FROM t RETURNING *) SELECT * FROM gone")).toBe(false);
    expect(isReadOnly("SELECT 1\nEXEC sp_configure")).toBe(false);
    expect(isReadOnly("select 1; commit; drop table x")).toBe(false);
    expect(isReadOnly("SELECT 1; USE master")).toBe(false);
    expect(isReadOnly("SELECT 1\nSET ROLE admin")).toBe(false);
  });

  it("accepts an explanation commented out before the query", () => {
    expect(isReadOnly("-- The user's email is on the Person table, so join those.\n\nSELECT TOP 100 email FROM dbo.Person")).toBe(true);
    expect(isReadOnly("The email is on Person.\nSELECT email FROM dbo.Person")).toBe(false);
  });

  it("ignores write words inside strings, identifiers and comments", () => {
    expect(isReadOnly("SELECT * FROM audit WHERE action = 'DELETE'")).toBe(true);
    expect(isReadOnly("SELECT [Update], \"insert\", `drop` FROM t")).toBe(true);
    expect(isReadOnly("SELECT updated_at, created_by FROM t -- delete later\n/* drop */")).toBe(true);
    expect(isReadOnly("SELECT REPLACE(name, 'a', 'b') FROM t WHERE note = 'it''s DELETE'")).toBe(true);
  });
});
