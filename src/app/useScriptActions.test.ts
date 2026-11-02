// @vitest-environment jsdom
import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { Script } from "../api";
import { commands } from "../api/commands";
import { useScripts } from "../features/scripts";
import { useTabs } from "../features/tabs";
import { useScriptActions } from "./useScriptActions";

vi.mock("../api/commands", () => ({
  commands: { listScripts: vi.fn(), saveScript: vi.fn(), deleteScript: vi.fn() },
}));
const api = vi.mocked(commands);

const stored: Script = {
  id: "s1",
  name: "Top customers",
  connectionId: "c",
  database: "shop",
  question: "top",
  sql: "SELECT 1;",
  updatedAt: 1,
};

async function setup() {
  const { result } = renderHook(() => {
    const scripts = useScripts();
    const tabs = useTabs();
    return { scripts, tabs, actions: useScriptActions(scripts, tabs) };
  });
  await waitFor(() => expect(result.current.scripts.scripts).toHaveLength(1));
  return result;
}

describe("useScriptActions", () => {
  beforeEach(() => {
    localStorage.clear();
    vi.resetAllMocks();
    api.listScripts.mockResolvedValue([stored]);
    api.deleteScript.mockResolvedValue(undefined);
    api.saveScript.mockImplementation(async (s) => ({ ...s, id: s.id || "new", updatedAt: 2 }));
  });

  it("opens a script in a tab, and focuses that tab the second time", async () => {
    const r = await setup();
    act(() => r.current.actions.openScript("s1"));
    expect(r.current.tabs.tabs).toHaveLength(1);
    const tab = r.current.tabs.tabs[0];
    expect(tab).toMatchObject({
      scriptId: "s1",
      title: "Top customers",
      sql: "SELECT 1;",
      savedSql: "SELECT 1;",
      target: { connectionId: "c", database: "shop" },
    });

    act(() => {
      r.current.tabs.open({ connectionId: "c", database: null });
    });
    act(() => r.current.actions.openScript("s1"));
    expect(r.current.tabs.tabs).toHaveLength(2);
    expect(r.current.tabs.active?.id).toBe(tab.id);
  });

  it("saves a new tab under the given name and marks it saved", async () => {
    const r = await setup();
    act(() => {
      r.current.tabs.open({ connectionId: "c", database: null }, { sql: "SELECT 2;" });
    });
    await act(() => r.current.actions.saveTab(r.current.tabs.tabs[0], "Mine"));
    expect(api.saveScript).toHaveBeenCalledWith(
      expect.objectContaining({ id: "", name: "Mine", sql: "SELECT 2;" }),
    );
    expect(r.current.tabs.tabs[0]).toMatchObject({
      scriptId: "new",
      title: "Mine",
      savedSql: "SELECT 2;",
    });
  });

  it("re-saves a script tab under its own id and name", async () => {
    const r = await setup();
    act(() => r.current.actions.openScript("s1"));
    act(() => r.current.tabs.update(r.current.tabs.tabs[0].id, { sql: "SELECT 3;" }));
    await act(() => r.current.actions.saveTab(r.current.tabs.tabs[0]));
    expect(api.saveScript).toHaveBeenCalledWith(
      expect.objectContaining({ id: "s1", name: "Top customers", sql: "SELECT 3;" }),
    );
  });

  it("deleting a script keeps open tabs' SQL, as unsaved", async () => {
    const r = await setup();
    act(() => r.current.actions.openScript("s1"));
    await act(() => r.current.actions.deleteScript(stored));
    expect(api.deleteScript).toHaveBeenCalledWith("s1");
    expect(r.current.scripts.scripts).toEqual([]);
    expect(r.current.tabs.tabs[0]).toMatchObject({
      sql: "SELECT 1;",
      scriptId: undefined,
      savedSql: undefined,
    });
  });
});
