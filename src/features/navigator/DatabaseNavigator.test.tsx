// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { ConnectionConfig, Schema, Table } from "../../api";
import { DatabaseNavigator } from "./DatabaseNavigator";

const conn: ConnectionConfig = {
  id: "c1",
  name: "Shop",
  kind: "sqlite",
  host: "",
  port: null,
  database: "",
  user: "",
  file: "/shop.sqlite",
  ssl: false,
  trustCert: false,
};

const table = (name: string, kind: Table["kind"]): Table => ({
  schema: null,
  name,
  kind,
  columns: [],
  foreign_keys: [],
  row_estimate: null,
});

describe("DatabaseNavigator", () => {
  afterEach(cleanup);

  it("lists materialized views with the views", () => {
    const schema: Schema = {
      dialect: "SQLite",
      tables: [table("events", "table"), table("daily_totals", "materialized view")],
    };
    render(
      <DatabaseNavigator
        connections={[conn]}
        activeId="c1"
        databases={{ c1: null }}
        databaseName={null}
        schema={schema}
        loading={false}
        connected
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
        notes={[
          {
            id: "n1",
            connectionId: "c1",
            database: null,
            text: "status 3 = cancelled",
            updatedAt: 0,
          },
        ]}
        onEditNote={vi.fn()}
        onDeleteNote={vi.fn()}
        onAgentSettings={vi.fn()}
        onOverview={vi.fn()}
      />,
    );
    expect(screen.getByText("Notes")).toBeTruthy();
    expect(screen.getByText("status 3 = cancelled")).toBeTruthy();
    expect(screen.getByText("Views")).toBeTruthy();
    expect(screen.getByText("daily_totals")).toBeTruthy();
    expect(screen.getByText("events")).toBeTruthy();
  });

  it("marks notes that may be out of date, and opens one to review", () => {
    const schema: Schema = { dialect: "SQLite", tables: [table("users", "table")] };
    const onEditNote = vi.fn();
    const onCheckNotes = vi.fn();
    const doubtful = {
      id: "n2",
      connectionId: "c1",
      database: null,
      text: "users.last_login_at is set on sign-in",
      updatedAt: 0,
      suspect: { reason: "the column users.last_login_at no longer exists", since: 0 },
    };
    render(
      <DatabaseNavigator
        {...props({ schema, onEditNote, onCheckNotes })}
        notes={[
          {
            id: "n1",
            connectionId: "c1",
            database: null,
            text: "status 3 = cancelled",
            updatedAt: 0,
            verifiedAt: Date.now(),
          },
          doubtful,
        ]}
      />,
    );
    expect(screen.getByLabelText("Checked")).toBeTruthy();
    expect(screen.getByLabelText("May be out of date")).toBeTruthy();
    expect(screen.getByTitle("1 may be out of date").textContent).toBe("1");
    fireEvent.click(screen.getByText("users.last_login_at is set on sign-in"));
    expect(onEditNote).toHaveBeenCalledWith(doubtful);
    fireEvent.click(screen.getByLabelText("Check notes against the database now"));
    expect(onCheckNotes).toHaveBeenCalled();
  });

  it("shows a team's shared notes read-only, with the same marks, and offers only a suggestion", () => {
    const schema: Schema = { dialect: "PostgreSQL", tables: [table("orders", "table")] };
    const team: ConnectionConfig = {
      ...conn,
      id: "team:c_1",
      name: "Production",
      kind: "postgres",
      team: { database: "app", notes: "" },
    };
    const onEditNote = vi.fn();
    const onCheckNotes = vi.fn();
    const doubtful = {
      id: "kn_2",
      connectionId: "team:c_1",
      database: null,
      text: "users.last_login_at is set on sign-in",
      updatedAt: 0,
      suspect: { reason: "the column users.last_login_at no longer exists", since: 0 },
    };
    render(
      <DatabaseNavigator
        {...props({
          connections: [team],
          activeId: "team:c_1",
          databases: { "team:c_1": null },
          schema,
          onEditNote,
          onCheckNotes,
        })}
        sharedNotes
        notes={[
          {
            id: "kn_1",
            connectionId: "team:c_1",
            database: null,
            text: "orders.status 3 means refunded",
            updatedAt: 0,
            verifiedAt: Date.now(),
          },
          doubtful,
        ]}
      />,
    );
    expect(
      screen.getByText("Shared by your team. Owners approve changes on the team server."),
    ).toBeTruthy();
    expect(screen.getByLabelText("Checked")).toBeTruthy();
    expect(screen.getByLabelText("May be out of date")).toBeTruthy();
    // No checking or editing here.
    expect(screen.queryByLabelText("Check notes against the database now")).toBeNull();
    fireEvent.click(screen.getByText("users.last_login_at is set on sign-in"));
    fireEvent.doubleClick(screen.getByText("orders.status 3 means refunded"));
    expect(onEditNote).not.toHaveBeenCalled();
    fireEvent.contextMenu(screen.getByText("orders.status 3 means refunded"));
    expect(screen.queryByText("Edit note…")).toBeNull();
    expect(screen.queryByText("Delete note")).toBeNull();
    fireEvent.click(screen.getAllByText("Suggest a note…")[0]);
    expect(onEditNote).toHaveBeenCalledWith(null);
  });

  const props = (over: Partial<Parameters<typeof DatabaseNavigator>[0]> = {}) => ({
    connections: [conn],
    activeId: "c1",
    databases: { c1: null },
    databaseName: null,
    schema: null,
    loading: false,
    connected: true,
    onConnect: vi.fn(),
    onOpen: vi.fn(),
    onOpenDatabase: vi.fn(),
    onAdd: vi.fn(),
    onEdit: vi.fn(),
    onDeleteConnection: vi.fn(),
    onRefresh: vi.fn(),
    onPreviewTable: vi.fn(),
    onNewTab: vi.fn(),
    onOpenSql: vi.fn(),
    selectedTables: [],
    onSelectTables: vi.fn(),
    scripts: [],
    onOpenScript: vi.fn(),
    onDeleteScript: vi.fn(),
    notes: [],
    onEditNote: vi.fn(),
    onDeleteNote: vi.fn(),
    onAgentSettings: vi.fn(),
    onOverview: vi.fn(),
    ...over,
  });

  it("shows a restored, unconnected connection with a Connect button instead of its tables", () => {
    const onConnect = vi.fn();
    const schema: Schema = { dialect: "SQLite", tables: [table("events", "table")] };
    render(<DatabaseNavigator {...props({ connected: false, onConnect, schema })} />);
    expect(screen.getByText("Not connected")).toBeTruthy();
    expect(screen.queryByText("events")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Connect" }));
    expect(onConnect).toHaveBeenCalledTimes(1);
  });

  it("badges a production connection PROD, and only that one", () => {
    const prod = { ...conn, id: "p1", name: "Orders prod", production: true };
    render(<DatabaseNavigator {...props({ connections: [conn, prod], activeId: null })} />);
    expect(screen.getAllByText("PROD")).toHaveLength(1);
    expect(screen.getByText("Orders prod").parentElement?.textContent).toContain("PROD");
  });
});
