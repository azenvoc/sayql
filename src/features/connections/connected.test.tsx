// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, renderHook, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ConnectionConfig } from "../../api";
import { commands } from "../../api/commands";
import { ConnectionDialog, newConnection } from "./ConnectionDialog";
import { useConnections } from "./useConnections";

vi.mock("../../api/commands", () => ({
  commands: {
    openDatabase: vi.fn(),
    listDatabases: vi.fn(),
    refreshSchema: vi.fn(),
    saveConnection: vi.fn(),
  },
}));
const openDatabase = vi.mocked(commands.openDatabase);
const listDatabases = vi.mocked(commands.listDatabases);
const saveConnection = vi.mocked(commands.saveConnection);

const prod: ConnectionConfig = {
  ...newConnection(),
  id: "pg",
  name: "analytics@prod",
  host: "db.internal",
  database: "analytics",
  production: true,
};

describe("connecting is the person's choice", () => {
  beforeEach(() => {
    openDatabase.mockReset().mockResolvedValue({ dialect: "PostgreSQL", tables: [] });
    listDatabases.mockReset().mockResolvedValue({ databases: ["analytics"], current: "analytics" });
  });

  it("starts with nothing connected, and connects only when asked", async () => {
    const { result } = renderHook(() => useConnections([prod]));
    // Restored on start: known, not connected, nothing opened.
    expect(result.current.isConnected("pg")).toBe(false);
    expect(openDatabase).not.toHaveBeenCalled();
    expect(listDatabases).not.toHaveBeenCalled();

    await act(async () => result.current.connect({ connectionId: "pg", database: null }));
    expect(result.current.isConnected("pg")).toBe(true);
    expect(openDatabase).toHaveBeenCalledWith({ connectionId: "pg", database: null });
  });

  it("forgets a deleted connection's connected state", async () => {
    const { result } = renderHook(() => useConnections([prod]));
    await act(async () => result.current.connect({ connectionId: "pg", database: null }));
    act(() => result.current.remove("pg"));
    expect(result.current.isConnected("pg")).toBe(false);
  });
});

describe("the production tag", () => {
  afterEach(cleanup);

  it("is off for a new connection and saved with it when ticked", async () => {
    expect(newConnection().production).toBe(false);
    saveConnection.mockImplementation(async (c) => ({ ...c, id: "new" }));
    const onSaved = vi.fn();
    render(
      <ConnectionDialog
        initial={newConnection()}
        onClose={() => {}}
        onSaved={onSaved}
        onDeleted={() => {}}
      />,
    );
    fireEvent.click(screen.getByLabelText("This is a production database"));
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    await vi.waitFor(() => expect(onSaved).toHaveBeenCalled());
    expect(saveConnection.mock.calls[0][0].production).toBe(true);
  });

  it("reads a connection saved before the tag existed as not production", () => {
    const { production: _, ...old } = prod;
    render(
      <ConnectionDialog initial={old} onClose={() => {}} onSaved={() => {}} onDeleted={() => {}} />,
    );
    expect(
      (screen.getByLabelText("This is a production database") as HTMLInputElement).checked,
    ).toBe(false);
  });
});
