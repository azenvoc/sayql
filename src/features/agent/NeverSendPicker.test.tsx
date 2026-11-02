// @vitest-environment jsdom
import { useState } from "react";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { AgentAccess, Column, Schema } from "../../api";
import { AgentAccessDialog } from "./AgentAccessDialog";
import { NeverSendPicker } from "./NeverSendPicker";
import { sensitiveKind, suggestedColumns } from "./neverSend";

const col = (name: string, data_type = "text"): Column => ({
  name,
  data_type,
  nullable: true,
  primary_key: false,
  enum_values: null,
});
const schema: Schema = {
  dialect: "SQLite",
  tables: [
    {
      schema: null,
      name: "categories",
      kind: "table",
      foreign_keys: [],
      row_estimate: 8,
      columns: [col("id", "integer"), col("name")],
    },
    {
      schema: null,
      name: "customers",
      kind: "table",
      foreign_keys: [],
      row_estimate: 500,
      columns: [
        col("id", "integer"),
        col("first_name"),
        col("last_name"),
        col("email"),
        col("country"),
      ],
    },
    {
      schema: null,
      name: "patients",
      kind: "table",
      foreign_keys: [],
      row_estimate: 5,
      columns: [col("id", "integer"), col("diagnosis"), col("notes")],
    },
  ],
};

const openDatabase = vi.fn((_t: unknown): Promise<Schema> => Promise.resolve(schema));
vi.mock("../../api", async (orig) => ({
  ...(await orig<typeof import("../../api")>()),
  commands: {
    openDatabase: (t: unknown) => openDatabase(t),
    listDatabases: () => Promise.resolve(null),
  },
}));

beforeEach(() => openDatabase.mockClear());
afterEach(cleanup);

/** The picker with its value held in state, reporting each change. */
function Harness({ initial, onChange }: { initial: string[]; onChange?: (v: string[]) => void }) {
  const [value, setValue] = useState(initial);
  return (
    <NeverSendPicker
      target={{ connectionId: "c1", database: null }}
      label="Never send these columns"
      value={value}
      onChange={(v) => {
        setValue(v);
        onChange?.(v);
      }}
    />
  );
}

const chips = () => within(screen.getByRole("list", { name: /Never send these columns/ }));

it("suggests likely personal columns by name, not things' names", () => {
  expect(sensitiveKind("email")).toBe("email");
  expect(sensitiveKind("PatientEmail")).toBe("email");
  expect(sensitiveKind("mobile_number")).toBe("phone");
  expect(sensitiveKind("date_of_birth")).toBe("birth date");
  expect(sensitiveKind("api_key")).toBe("secret");
  expect(sensitiveKind("password_hash")).toBe("secret");
  expect(sensitiveKind("iban")).toBe("card or bank");
  expect(sensitiveKind("postcode")).toBe("address");
  expect(sensitiveKind("salary")).toBe("pay");
  expect(sensitiveKind("diagnosis")).toBe("health");
  expect(sensitiveKind("ip_address")).toBe("IP address");
  expect(sensitiveKind("product_name")).toBeNull();
  expect(sensitiveKind("country")).toBeNull();
  expect(sensitiveKind("email_verified")).toBe("email");
  const found = suggestedColumns(schema).map((s) => `${s.table}.${s.column}`);
  expect(found).toEqual([
    "customers.first_name",
    "customers.last_name",
    "customers.email",
    "patients.diagnosis",
    "patients.notes",
  ]);
});

it("adds all suggested, then ticks and unticks single columns, saving table.column", async () => {
  const onChange = vi.fn();
  render(<Harness initial={[]} onChange={onChange} />);
  fireEvent.click(await screen.findByRole("button", { name: "Add all suggested (5)" }));
  expect(onChange).toHaveBeenLastCalledWith([
    "customers.first_name",
    "customers.last_name",
    "customers.email",
    "patients.diagnosis",
    "patients.notes",
  ]);
  expect(screen.getByRole("button", { name: "All suggested added" })).toHaveProperty(
    "disabled",
    true,
  );

  // Unticking in Suggested removes it; the chip goes too.
  const suggested = within(screen.getByRole("region", { name: "Suggested" }));
  fireEvent.click(suggested.getByRole("checkbox", { name: "patients.notes" }));
  expect(onChange).toHaveBeenLastCalledWith([
    "customers.first_name",
    "customers.last_name",
    "customers.email",
    "patients.diagnosis",
  ]);
  expect(chips().queryByText("patients.notes")).toBeNull();

  // Removing a chip unticks the column.
  fireEvent.click(screen.getByRole("button", { name: "Remove customers.email" }));
  expect(suggested.getByRole("checkbox", { name: "customers.email" })).toHaveProperty(
    "checked",
    false,
  );
});

it("selects a whole table from its header and filters tables by column", async () => {
  const onChange = vi.fn();
  render(<Harness initial={["customers.email"]} onChange={onChange} />);
  const all = await screen.findByRole("checkbox", { name: "All columns of customers" });
  expect(all.getAttribute("aria-checked")).toBe("mixed");
  fireEvent.click(all);
  expect(onChange).toHaveBeenLastCalledWith([
    "customers.email",
    "customers.id",
    "customers.first_name",
    "customers.last_name",
    "customers.country",
  ]);
  fireEvent.click(all);
  expect(onChange).toHaveBeenLastCalledWith([]);

  // Expanding shows column types; filtering by a column opens its table.
  fireEvent.click(screen.getByRole("button", { name: "Expand categories" }));
  expect(screen.getByRole("checkbox", { name: "categories.name" })).toBeTruthy();
  fireEvent.change(screen.getByLabelText("Filter tables and columns"), {
    target: { value: "diag" },
  });
  const tables = within(screen.getByRole("list", { name: "Tables" }));
  expect(tables.getByRole("checkbox", { name: "patients.diagnosis" })).toBeTruthy();
  expect(tables.queryByRole("checkbox", { name: "All columns of customers" })).toBeNull();
});

it("keeps bare names and entries not in the schema, and adds a name for every table", async () => {
  const onChange = vi.fn();
  render(<Harness initial={["diagnosis", "legacy.ssn"]} onChange={onChange} />);
  await screen.findByRole("region", { name: "Suggested" });
  // The bare name covers the column in every table: ticked and locked there.
  expect(chips().getByText(/any table/)).toBeTruthy();
  const diag = screen.getAllByRole("checkbox", { name: "patients.diagnosis" })[0];
  expect(diag).toHaveProperty("checked", true);
  expect(diag).toHaveProperty("disabled", true);
  // Saved but not in this database: kept, marked.
  expect(chips().getByText("legacy.ssn")).toBeTruthy();
  expect(chips().getByText("not found")).toBeTruthy();

  const input = screen.getByLabelText("Add a column name for every table");
  fireEvent.change(input, { target: { value: "salary" } });
  fireEvent.keyDown(input, { key: "Enter" });
  expect(onChange).toHaveBeenLastCalledWith(["diagnosis", "legacy.ssn", "salary"]);
  // Ticking another column keeps both.
  fireEvent.click(screen.getAllByRole("checkbox", { name: "customers.email" })[0]);
  expect(onChange).toHaveBeenLastCalledWith([
    "diagnosis",
    "legacy.ssn",
    "salary",
    "customers.email",
  ]);
});

it("falls back to typing when the tables can't be loaded", async () => {
  openDatabase.mockImplementationOnce(() => Promise.reject("connection refused"));
  const onChange = vi.fn();
  render(<Harness initial={["notes"]} onChange={onChange} />);
  expect(
    await screen.findByText(/couldn't load this connection's tables \(connection refused\)/),
  ).toBeTruthy();
  const box = screen.getByLabelText("Never send these columns") as HTMLTextAreaElement;
  expect(box.value).toBe("notes");
  fireEvent.change(box, { target: { value: "notes\npatients.diagnosis, email" } });
  expect(onChange).toHaveBeenLastCalledWith(["notes", "patients.diagnosis", "email"]);
});

it("saves the picked columns in the stored format, keeping the team admin's out of the list", async () => {
  const access: AgentAccess = {
    enabled: true,
    results: "counts",
    apps: "counts",
    askBeforeSend: true,
    neverSend: ["diagnosis", "customers.email"],
    team: { maxResults: "rows", neverSend: ["customers.email"] },
  };
  const onSave = vi.fn((_a: AgentAccess) => Promise.resolve());
  render(
    <AgentAccessDialog
      connectionName="Clinic"
      target={{ connectionId: "c1", database: "clinic" }}
      provider={{ label: "Anthropic", local: false }}
      access={access}
      onSave={onSave}
      onClose={() => {}}
    />,
  );
  await screen.findByRole("region", { name: "Suggested" });
  expect(openDatabase).toHaveBeenCalledWith({ connectionId: "c1", database: "clinic" });
  // The admin's column is ticked and locked, not a removable chip.
  const email = screen.getAllByRole("checkbox", { name: "customers.email" })[0];
  expect(email).toHaveProperty("disabled", true);
  expect(screen.queryByRole("button", { name: "Remove customers.email" })).toBeNull();

  fireEvent.click(screen.getAllByRole("checkbox", { name: "customers.last_name" })[0]);
  fireEvent.click(screen.getByRole("button", { name: "Save" }));
  await waitFor(() => expect(onSave).toHaveBeenCalled());
  expect(onSave.mock.calls[0][0].neverSend).toEqual(["diagnosis", "customers.last_name"]);
});
