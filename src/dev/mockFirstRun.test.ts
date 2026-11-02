import { beforeAll, describe, expect, it, vi } from "vitest";
import type { Bootstrap } from "../api";

type Handler = (cmd: string, args?: Record<string, unknown>) => Promise<unknown>;
let handler: Handler;

vi.mock("@tauri-apps/api/mocks", () => ({ mockIPC: (cb: Handler) => (handler = cb) }));
vi.mock("@tauri-apps/api/event", () => ({ emit: vi.fn() }));

beforeAll(async () => {
  vi.stubGlobal("window", { location: { search: "?firstrun" }, open: vi.fn() });
  (await import("./mockBackend")).installMockBackend();
});

describe("mock backend ?firstrun", () => {
  it("starts like a real first run: just the demo, AI not set up, setup not done", async () => {
    const boot = (await handler("init")) as Bootstrap;
    expect(boot.connections.map((c) => c.id)).toEqual(["demo"]);
    expect(boot.settings.aiReady).toBe(false);
    expect(boot.settings.onboarded).toBe(false);
    expect(boot.settings.providers.find((p) => p.id === "anthropic")?.hasKey).toBe(false);
  });

  it("has no notes, suggestions, audits or AI-app activity", async () => {
    expect(await handler("list_notes", { id: "demo" })).toEqual([]);
    expect(await handler("list_note_suggestions")).toEqual([]);
    expect(await handler("mcp_audits")).toEqual([]);
    expect(await handler("mcp_activity")).toEqual([]);
    expect(((await handler("mcp_apps")) as { apps: unknown[] }).apps).toEqual([]);
  });

  it("offers SayQL AI credits, and AI is ready once a key is saved", async () => {
    const boot = (await handler("init")) as Bootstrap;
    expect(boot.settings.providers.some((p) => p.id === "sayql")).toBe(true);
    const saved = (await handler("save_settings", {
      provider: "anthropic",
      typingModel: "claude-haiku-5-5",
      expertModel: "claude-opus-5-5",
      baseUrl: "",
      apiKey: "sk-ant-test",
      workspaceId: "",
    })) as Bootstrap["settings"];
    expect(saved.aiReady).toBe(true);
  });
});
