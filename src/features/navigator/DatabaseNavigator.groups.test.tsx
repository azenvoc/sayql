// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import type { ComponentProps } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { ConnectionConfig } from "../../api";
import { DatabaseNavigator } from "./DatabaseNavigator";

const connection = (id: string, name: string): ConnectionConfig => ({
  id,
  name,
  kind: "mssql",
  host: "sql.example.com",
  port: 1433,
  database: "",
  user: "",
  file: "",
  ssl: false,
  trustCert: false,
  production: false,
});

const groupOf: Record<string, string> = { billing: "portside", shared: "meridian" };

function show(extra: Partial<ComponentProps<typeof DatabaseNavigator>> = {}) {
  render(
    <DatabaseNavigator
      groups={[
        { id: "portside", name: "Portside Freight" },
        { id: "meridian", name: "Meridian Foods" },
        { id: "empty", name: "New client" },
      ]}
      groupOf={(id) => groupOf[id] ?? null}
      databasesIn={(id) =>
        id === "portside" ? [{ connectionId: "shared", database: "portside_orders" }] : []
      }
      isPlacedApart={(id, db) => id === "shared" && db === "portside_orders"}
      onOpenAt={vi.fn()}
      onNewGroup={vi.fn()}
      connections={[
        connection("billing", "billing-prod"),
        connection("shared", "sql-shared"),
        connection("loose", "scratch"),
      ]}
      activeId={null}
      databases={{}}
      databaseName={null}
      schema={null}
      loading={false}
      connected={false}
      onConnect={vi.fn()}
      onOpen={vi.fn()}
      onOpenDatabase={vi.fn()}
      onAdd={vi.fn()}
      onEdit={vi.fn()}
      onDeleteConnection={vi.fn()}
      onRefresh={vi.fn()}
      onPreviewTable={vi.fn()}
      onNewTab={vi.fn()}
      onOpenSql={vi.fn()}
      selectedTables={[]}
      onSelectTables={vi.fn()}
      scripts={[]}
      onOpenScript={vi.fn()}
      onDeleteScript={vi.fn()}
      notes={[]}
      onEditNote={vi.fn()}
      onDeleteNote={vi.fn()}
      onAgentSettings={vi.fn()}
      onOverview={vi.fn()}
      {...extra}
    />,
  );
}

const rows = () => screen.getAllByRole("treeitem").map((r) => r.textContent ?? "");

describe("DatabaseNavigator with groups", () => {
  afterEach(cleanup);

  it("puts connections and single databases in their groups, and the rest below", () => {
    show();
    const list = rows();
    const at = (text: string) => list.findIndex((r) => r.startsWith(text));
    expect(at("Portside Freight")).toBeLessThan(at("billing-prod"));
    expect(at("billing-prod")).toBeLessThan(at("portside_orders"));
    expect(at("portside_orders")).toBeLessThan(at("Meridian Foods"));
    expect(at("Meridian Foods")).toBeLessThan(at("sql-shared"));
    expect(at("New client")).toBeLessThan(at("scratch"));
  });

  it("says how to fill an empty group", () => {
    show();
    expect(screen.getByText("Right-click a connection to move it here")).toBeTruthy();
  });

  it("collapses a group", () => {
    show();
    fireEvent.click(screen.getByText("Portside Freight"));
    expect(rows().some((r) => r.includes("billing-prod"))).toBe(false);
    expect(rows().some((r) => r.includes("sql-shared"))).toBe(true);
  });

  it("opens a single database from its group", () => {
    const onOpenAt = vi.fn();
    show({ onOpenAt });
    fireEvent.click(screen.getByText("portside_orders"));
    expect(onOpenAt).toHaveBeenCalledWith("shared", "portside_orders");
  });

  it("finds groups, connections and databases", () => {
    show();
    const find = (text: string) =>
      fireEvent.change(screen.getByRole("textbox"), { target: { value: text } });
    find("billing");
    expect(rows().some((r) => r.includes("billing-prod"))).toBe(true);
    expect(rows().some((r) => r.includes("Meridian Foods"))).toBe(false);
    find("meridian");
    expect(rows().some((r) => r.includes("sql-shared"))).toBe(true);
    expect(rows().some((r) => r.includes("billing-prod"))).toBe(false);
    find("portside_orders");
    expect(rows().some((r) => r.includes("Portside Freight"))).toBe(true);
    expect(rows().some((r) => r.includes("billing-prod"))).toBe(false);
  });

  it("does not list a database placed apart under its connection too", () => {
    show({
      activeId: "shared",
      connected: true,
      databases: { shared: { databases: ["meridian_orders", "portside_orders"], current: null } },
    });
    const names = rows().filter((r) => r.includes("portside_orders"));
    expect(names).toHaveLength(1);
    expect(rows().some((r) => r.includes("meridian_orders"))).toBe(true);
  });

  it("shows nothing about groups to someone with none", () => {
    show({ groups: undefined, groupOf: undefined, databasesIn: undefined, onNewGroup: undefined });
    expect(screen.getByPlaceholderText("Filter tables and columns")).toBeTruthy();
    expect(screen.queryByRole("button", { name: "New group" })).toBeNull();
    expect(rows().some((r) => r.includes("scratch"))).toBe(true);
  });
});
