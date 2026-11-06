// @vitest-environment jsdom
import { act, renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { commands, type WorkspaceLayout } from "../../api";
import { useTabs } from "../tabs";
import { initialsOf, useWorkspaces } from "./useWorkspaces";

const layout = (): WorkspaceLayout => ({
  workspaces: [
    { id: "personal", name: "Personal", color: 0 },
    { id: "acme", name: "Acme Logistics", color: 1 },
    { id: "harbor", name: "Harbor", color: 2 },
  ],
  groups: [
    { id: "portside", workspaceId: "acme", name: "Portside" },
    { id: "clinics", workspaceId: "harbor", name: "Clinics" },
  ],
  placement: {
    billing: { workspaceId: "acme", groupId: "portside", allowedDatabases: null },
    shared: { workspaceId: "acme", groupId: null, allowedDatabases: ["a", "b"] },
    claims: { workspaceId: "harbor", groupId: null, allowedDatabases: null },
  },
  databaseGroups: [{ connectionId: "shared", database: "a", groupId: "portside" }],
  teamWorkspace: "harbor",
});

describe("useWorkspaces", () => {
  beforeEach(() => localStorage.clear());

  it("opens on Personal, and a connection with no place is in Personal", () => {
    const { result } = renderHook(() => useWorkspaces(layout()));
    expect(result.current.activeId).toBe("personal");
    expect(result.current.workspaceOf("saved-before-workspaces")).toBe("personal");
    expect(result.current.workspaceOf("billing")).toBe("acme");
  });

  it("puts a team's connections in the workspace chosen for them", () => {
    const { result } = renderHook(() => useWorkspaces(layout()));
    expect(result.current.workspaceOf("team:c1")).toBe("harbor");
  });

  it("shows only the open workspace's groups", () => {
    const { result } = renderHook(() => useWorkspaces(layout()));
    act(() => result.current.switchTo("acme"));
    expect(result.current.groupsHere.map((g) => g.id)).toEqual(["portside"]);
  });

  it("falls back to Personal when the remembered workspace is gone", () => {
    localStorage.setItem("sayql.workspace", JSON.stringify("deleted"));
    const { result } = renderHook(() => useWorkspaces(layout()));
    expect(result.current.activeId).toBe("personal");
  });

  it("names the group of the open database, which may not be its connection's", () => {
    const { result } = renderHook(() => useWorkspaces(layout()));
    expect(result.current.groupAt("billing", null)?.id).toBe("portside");
    expect(result.current.groupAt("shared", "a")?.id).toBe("portside");
    expect(result.current.groupAt("shared", "b")).toBeNull();
    expect(result.current.isPlacedApart("shared", "a")).toBe(true);
    expect(result.current.isPlacedApart("shared", "b")).toBe(false);
  });

  it("is on the new workspace after creating it", async () => {
    const next = layout();
    next.workspaces.push({ id: "coastal", name: "Coastal", color: 3 });
    vi.spyOn(commands, "createWorkspace").mockResolvedValue(next);
    const { result } = renderHook(() => useWorkspaces(layout()));
    await act(() => result.current.createWorkspace("Coastal"));
    expect(result.current.activeId).toBe("coastal");
  });

  it("makes a tile's letters from the name", () => {
    expect(initialsOf("Acme Logistics")).toBe("AL");
    expect(initialsOf("Personal")).toBe("P");
    expect(initialsOf("株式会社")).toBe("株");
  });
});

describe("tabs across workspaces", () => {
  beforeEach(() => localStorage.clear());

  const setup = () =>
    renderHook(() => {
      const workspaces = useWorkspaces(layout());
      const tabs = useTabs({
        id: workspaces.activeId,
        workspaceOf: workspaces.workspaceOf,
        reveal: workspaces.reveal,
      });
      return { workspaces, tabs };
    });

  it("shows a workspace's own tabs, keeps the others open, and comes back to the last one", () => {
    const { result } = setup();
    let first = "";
    act(() => {
      first = result.current.tabs.open({ connectionId: "billing", database: null });
    });
    act(() => {
      result.current.tabs.open({ connectionId: "shared", database: "a" });
    });
    act(() => result.current.tabs.activate(first));
    act(() => result.current.workspaces.switchTo("harbor"));
    expect(result.current.tabs.here).toEqual([]);
    expect(result.current.tabs.active).toBeNull();
    // Still open and mounted, so what runs in them keeps running.
    expect(result.current.tabs.tabs).toHaveLength(2);
    act(() => result.current.workspaces.switchTo("acme"));
    expect(result.current.tabs.here).toHaveLength(2);
    expect(result.current.tabs.active?.id).toBe(first);
  });

  it("shows the workspace a new tab opened in, as when an AI app hands over SQL", () => {
    const { result } = setup();
    expect(result.current.workspaces.activeId).toBe("personal");
    act(() => {
      result.current.tabs.open({ connectionId: "claims", database: null });
    });
    expect(result.current.workspaces.activeId).toBe("harbor");
    expect(result.current.tabs.active?.target.connectionId).toBe("claims");
  });

  it("closes only what it's told to, in any workspace", () => {
    const { result } = setup();
    act(() => {
      result.current.tabs.open({ connectionId: "billing", database: null });
    });
    act(() => {
      result.current.tabs.open({ connectionId: "claims", database: null });
    });
    act(() => result.current.tabs.closeWhere((t) => t.target.connectionId === "billing"));
    expect(result.current.tabs.tabs.map((t) => t.target.connectionId)).toEqual(["claims"]);
  });
});
