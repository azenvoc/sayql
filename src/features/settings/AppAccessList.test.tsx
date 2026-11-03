// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { AgentAccess, ConnectionConfig, Schema } from "../../api";
import { AppAccessList } from "./AppAccessList";

const saved: AgentAccess = {
  enabled: false,
  results: "rows",
  apps: "counts",
  askBeforeSend: false,
  neverSend: ["notes"],
  team: null,
};
// What the list saves: the choices, without the team admin's rules.
const { team: _team, ...choices } = saved;
const agentAccess = vi.fn((_id: string) => Promise.resolve(saved));
const saveAgentAccess = vi.fn((_id: string, a: AgentAccess) => Promise.resolve(a));
const schema: Schema = {
  dialect: "PostgreSQL",
  tables: [
    {
      schema: "public",
      name: "customers",
      kind: "table",
      foreign_keys: [],
      row_estimate: 10,
      comment: null,
      checks: [],
      columns: [
        {
          name: "id",
          data_type: "integer",
          nullable: false,
          primary_key: true,
          enum_values: null,
          comment: null,
        },
        {
          name: "email",
          data_type: "text",
          nullable: true,
          primary_key: false,
          enum_values: null,
          comment: null,
        },
      ],
    },
  ],
};
vi.mock("../../api", async (orig) => ({
  ...(await orig<typeof import("../../api")>()),
  commands: {
    agentAccess: (id: string) => agentAccess(id),
    saveAgentAccess: (id: string, a: AgentAccess) => saveAgentAccess(id, a),
    openDatabase: () => Promise.resolve(schema),
    listDatabases: () => Promise.resolve(null),
  },
}));

const conn = (id: string, name: string): ConnectionConfig => ({
  id,
  name,
  kind: "postgres",
  host: "",
  port: null,
  database: "",
  user: "",
  file: "",
  ssl: false,
  trustCert: false,
  production: false,
});
const provider = { label: "Anthropic", local: false };

beforeEach(() => {
  agentAccess.mockClear();
  saveAgentAccess.mockClear();
});
afterEach(cleanup);

it("shows counts by default and saves a new choice without touching agent mode", async () => {
  render(
    <AppAccessList
      connections={[conn("c1", "Production"), conn("team:x", "Team DB")]}
      provider={provider}
    />,
  );
  const select = await screen.findByLabelText("What AI apps may see on Production");
  await waitFor(() => expect((select as HTMLSelectElement).value).toBe("counts"));
  // Team connections aren't served over MCP, so they aren't listed.
  expect(screen.queryByLabelText("What AI apps may see on Team DB")).toBeNull();
  expect(agentAccess).toHaveBeenCalledTimes(1);
  expect(screen.getByText("PostgreSQL")).toBeTruthy();

  fireEvent.change(select, { target: { value: "all" } });
  await waitFor(() => expect(saveAgentAccess).toHaveBeenCalled());
  expect(saveAgentAccess).toHaveBeenCalledWith("c1", { ...choices, apps: "all" });
});

it("explains each level once, above the table, not per row", async () => {
  render(
    <AppAccessList
      connections={[conn("c1", "Production"), conn("c2", "Staging")]}
      provider={provider}
    />,
  );
  await screen.findByLabelText("What AI apps may see on Staging");
  expect(screen.getAllByText(/Rows as they are, and CSV export/)).toHaveLength(1);
});

it("shows never-send columns as chips and edits them without turning agent mode on", async () => {
  render(<AppAccessList connections={[conn("c1", "Production")]} provider={provider} />);
  expect(await screen.findByText("notes")).toBeTruthy();

  fireEvent.click(screen.getByRole("button", { name: "Edit never-send columns on Production" }));
  const dialog = await screen.findByRole("dialog", { name: "Never send on Production" });
  expect(dialog).toBeTruthy();
  // Only the columns: no results choice and no "Turn off agent mode".
  expect(screen.queryByRole("radiogroup", { name: "Results sent to the AI" })).toBeNull();
  expect(screen.queryByRole("button", { name: "Turn off agent mode" })).toBeNull();

  // Picked from the tables; the saved bare name stays, shown for every table.
  fireEvent.click(await screen.findByRole("button", { name: "Add all suggested (1)" }));
  fireEvent.click(screen.getByRole("button", { name: "Save" }));
  await waitFor(() => expect(saveAgentAccess).toHaveBeenCalled());
  expect(saveAgentAccess).toHaveBeenCalledWith("c1", {
    ...choices,
    neverSend: ["notes", "customers.email"],
  });
  expect(await screen.findByText("customers.email")).toBeTruthy();
  expect(screen.queryByRole("dialog")).toBeNull();
  // A bare name reads as every table in the chips.
  expect(screen.getByText(/any table/)).toBeTruthy();
});
