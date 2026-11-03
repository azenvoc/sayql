// @vitest-environment jsdom
import { act, renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ConnectionConfig } from "../api";
import { commands } from "../api/commands";
import { useConnections } from "../features/connections";
import { useTabs } from "../features/tabs";
import { useConnectionActions } from "./useConnectionActions";

vi.mock("../api/commands", () => ({ commands: { deleteConnection: vi.fn() } }));
const deleteConnection = vi.mocked(commands.deleteConnection);

const config = (id: string): ConnectionConfig => ({
  id,
  name: id,
  kind: "sqlite",
  host: "",
  port: null,
  database: "",
  user: "",
  file: `/${id}.db`,
  ssl: false,
  trustCert: false,
  production: false,
});

function setup() {
  const { result } = renderHook(() => {
    const conn = useConnections([config("a"), config("b")]);
    const tabs = useTabs();
    return { conn, tabs, actions: useConnectionActions(conn, tabs) };
  });
  act(() => {
    result.current.tabs.open({ connectionId: "a", database: null });
    result.current.tabs.open({ connectionId: "b", database: null });
  });
  return result;
}

const ids = (r: ReturnType<typeof setup>) => r.current.conn.connections.map((c) => c.id);
const tabConnections = (r: ReturnType<typeof setup>) =>
  r.current.tabs.tabs.map((t) => t.target.connectionId);

describe("useConnectionActions", () => {
  beforeEach(() => {
    localStorage.clear();
    deleteConnection.mockReset();
  });

  it("deletes the connection and closes its tabs", async () => {
    deleteConnection.mockResolvedValue(null);
    const r = setup();
    let ok: boolean | undefined;
    await act(async () => {
      ok = await r.current.actions.remove("a");
    });
    expect(ok).toBe(true);
    expect(deleteConnection).toHaveBeenCalledWith("a");
    expect(ids(r)).toEqual(["b"]);
    expect(tabConnections(r)).toEqual(["b"]);
    expect(r.current.actions.error).toBeNull();
  });

  it("keeps the connection and its tabs, and shows why, when the delete fails", async () => {
    deleteConnection.mockRejectedValue("keychain is locked");
    const r = setup();
    let ok: boolean | undefined;
    await act(async () => {
      ok = await r.current.actions.remove("a");
    });
    expect(ok).toBe(false);
    expect(ids(r)).toEqual(["a", "b"]);
    expect(tabConnections(r)).toEqual(["a", "b"]);
    expect(r.current.actions.error).toBe("Could not delete the connection: keychain is locked");

    act(() => r.current.actions.dismissError());
    expect(r.current.actions.error).toBeNull();
  });

  it("forgets a connection the dialog already deleted, without calling the backend", () => {
    const r = setup();
    act(() => r.current.actions.forget("b"));
    expect(deleteConnection).not.toHaveBeenCalled();
    expect(ids(r)).toEqual(["a"]);
    expect(tabConnections(r)).toEqual(["a"]);
  });
});
