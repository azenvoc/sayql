// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import type { McpActivity } from "../../api";
import { AgentActivity } from "./AgentActivity";

const LONG =
  "SELECT c.country, COUNT(*) AS orders FROM orders o JOIN customers c ON c.id = o.customer_id WHERE o.status = 'paid' GROUP BY c.country";
let log: McpActivity[] = [];
const fresh = (): McpActivity[] => [
  {
    database: null,
    workspace: null,
    group: null,
    at: 3,
    app: "Cursor",
    tool: "run_query",
    connectionId: "c1",
    connection: "Shop",
    sql: "SELECT email FROM customers",
    status: "refused",
    detail: "never-send column email",
    maySee: "counts",
    rows: null,
  },
  {
    database: null,
    workspace: null,
    group: null,
    at: 2,
    app: "Claude Code",
    tool: "run_query",
    connectionId: "c1",
    connection: "Shop",
    sql: "SELECT COUNT(*) FROM orders",
    status: "ok",
    rows: 1,
    detail: null,
    maySee: null,
  },
  {
    database: null,
    workspace: null,
    group: null,
    at: 1,
    app: "Claude Code",
    tool: "run_query",
    connectionId: "c2",
    connection: "Warehouse",
    sql: LONG,
    status: "ok",
    rows: 12,
    maySee: "masked",
    detail: null,
  },
];
const mcpActivity = vi.fn(() => Promise.resolve(log));
const clearMcpActivity = vi.fn(() => {
  log = [];
  return Promise.resolve();
});
vi.mock("../../api", async (orig) => ({
  ...(await orig<typeof import("../../api")>()),
  commands: {
    mcpActivity: () => mcpActivity(),
    clearMcpActivity: () => clearMcpActivity(),
  },
}));

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  mcpActivity.mockClear();
  clearMcpActivity.mockClear();
});

const bodyRows = () => within(screen.getAllByRole("rowgroup")[1]).getAllByRole("row");

it("lists what AI apps ran, refusals included, and says rows are never kept", async () => {
  log = fresh();
  render(<AgentActivity />);
  expect(await screen.findByText("SELECT email FROM customers")).toBeTruthy();
  expect(screen.getByText(/never-send column email/)).toBeTruthy();
  expect(screen.getByText(/· 1 row\b/)).toBeTruthy();
  expect(
    screen.getByText(/the SQL and how many rows it returned, not the rows themselves/),
  ).toBeTruthy();
});

it("filters by app, connection and refused only", async () => {
  log = fresh();
  render(<AgentActivity />);
  await screen.findByText("SELECT email FROM customers");
  expect(bodyRows()).toHaveLength(3);

  fireEvent.change(screen.getByLabelText("App"), { target: { value: "Claude Code" } });
  expect(bodyRows()).toHaveLength(2);
  fireEvent.change(screen.getByLabelText("Connection"), { target: { value: "Warehouse" } });
  expect(bodyRows()).toHaveLength(1);

  fireEvent.click(screen.getByRole("radio", { name: "Refused only" }));
  expect(screen.getByText("Nothing in the log matches these filters.")).toBeTruthy();

  fireEvent.change(screen.getByLabelText("App"), { target: { value: "" } });
  fireEvent.change(screen.getByLabelText("Connection"), { target: { value: "" } });
  expect(bodyRows()).toHaveLength(1);
  expect(screen.getByText("SELECT email FROM customers")).toBeTruthy();
});

it("opens a row for the full SQL and its detail, and refreshes", async () => {
  log = fresh();
  render(<AgentActivity />);
  await screen.findByText("SELECT email FROM customers");
  const row = screen.getByText(LONG).closest("tr")!;
  expect(row.getAttribute("aria-expanded")).toBe("false");

  fireEvent.click(row);
  expect(row.getAttribute("aria-expanded")).toBe("true");
  // The full SQL, wrapped, plus what the app could see at the time.
  expect(screen.getAllByText(LONG)).toHaveLength(2);
  expect(screen.getByText("Rows, masked")).toBeTruthy();

  fireEvent.keyDown(row, { key: "Enter" });
  expect(screen.getAllByText(LONG)).toHaveLength(1);

  fireEvent.click(screen.getByRole("button", { name: "Refresh" }));
  await waitFor(() => expect(mcpActivity).toHaveBeenCalledTimes(2));
});

it("clears the log only after confirming", async () => {
  log = fresh();
  const confirm = vi.spyOn(window, "confirm").mockReturnValueOnce(false).mockReturnValueOnce(true);
  render(<AgentActivity />);
  await screen.findByText("SELECT email FROM customers");

  fireEvent.click(screen.getByRole("button", { name: "Clear the log" }));
  expect(confirm).toHaveBeenCalledWith("Delete the activity log? Blocked apps stay blocked.");
  expect(clearMcpActivity).not.toHaveBeenCalled();

  fireEvent.click(screen.getByRole("button", { name: "Clear the log" }));
  await waitFor(() => expect(clearMcpActivity).toHaveBeenCalled());
  expect(await screen.findByText(/Nothing yet\. Queries from Claude, Cursor/)).toBeTruthy();
  expect(screen.queryByRole("button", { name: "Clear the log" })).toBeNull();
});
