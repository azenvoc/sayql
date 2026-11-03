// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { commands } from "../../api/commands";
import { ConnectionDialog, newConnection } from "./ConnectionDialog";

vi.mock("../../api/commands", () => ({
  commands: { deleteConnection: vi.fn(), saveConnection: vi.fn() },
}));
const deleteConnection = vi.mocked(commands.deleteConnection);
const saveConnection = vi.mocked(commands.saveConnection);

const existing = { ...newConnection(), id: "c1", name: "Shop", database: "shop" };

function renderDialog() {
  const onDeleted = vi.fn();
  render(
    <ConnectionDialog
      initial={existing}
      onClose={() => {}}
      onSaved={() => {}}
      onDeleted={onDeleted}
    />,
  );
  return onDeleted;
}

describe("ConnectionDialog delete", () => {
  afterEach(() => {
    cleanup();
    deleteConnection.mockReset();
  });

  it("deletes after a second click", async () => {
    deleteConnection.mockResolvedValue(null);
    const onDeleted = renderDialog();
    fireEvent.click(screen.getByRole("button", { name: /^Delete$/ }));
    fireEvent.click(await screen.findByRole("button", { name: /Click again to delete/ }));
    await vi.waitFor(() => expect(onDeleted).toHaveBeenCalledWith("c1"));
  });

  it("shows the error and keeps the dialog open when the delete fails", async () => {
    deleteConnection.mockRejectedValue("keychain is locked");
    const onDeleted = renderDialog();
    fireEvent.click(screen.getByRole("button", { name: /^Delete$/ }));
    fireEvent.click(await screen.findByRole("button", { name: /Click again to delete/ }));
    expect((await screen.findByRole("alert")).textContent).toContain(
      "Could not delete the connection: keychain is locked",
    );
    expect(onDeleted).not.toHaveBeenCalled();
    // Back to the first step: another click asks again rather than deleting straight away.
    expect(screen.getByRole("button", { name: /^Delete$/ })).toBeTruthy();
  });
});

describe("ConnectionDialog connection string", () => {
  afterEach(() => {
    cleanup();
    saveConnection.mockReset();
  });

  it("fills the form from a pasted string, clears it, and saves the password separately", async () => {
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
    const box = screen.getByLabelText(/Connection string/) as HTMLInputElement;
    fireEvent.paste(box, {
      clipboardData: {
        getData: () =>
          "Server=tcp:coastal.database.windows.net,1433;Initial Catalog=CoastalHealth;User ID=reader;Password=s3cret;Encrypt=True",
      },
    });
    expect(box.value).toBe("");
    expect((screen.getByLabelText("Host") as HTMLInputElement).value).toBe(
      "coastal.database.windows.net",
    );
    expect((screen.getByLabelText("Database") as HTMLInputElement).value).toBe("CoastalHealth");
    expect((screen.getByLabelText("User") as HTMLInputElement).value).toBe("reader");
    expect((screen.getByLabelText("Password") as HTMLInputElement).value).toBe("s3cret");
    expect(screen.getByText(/Filled in, including the password/)).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    await vi.waitFor(() => expect(onSaved).toHaveBeenCalled());
    const [config, password] = saveConnection.mock.calls[0];
    expect(config).toMatchObject({
      kind: "mssql",
      host: "coastal.database.windows.net",
      port: 1433,
      ssl: true,
    });
    expect(JSON.stringify(config)).not.toContain("s3cret");
    expect(password).toBe("s3cret");
  });

  it("says so when the text isn't a connection string", () => {
    render(
      <ConnectionDialog
        initial={newConnection()}
        onClose={() => {}}
        onSaved={() => {}}
        onDeleted={() => {}}
      />,
    );
    fireEvent.paste(screen.getByLabelText(/Connection string/), {
      clipboardData: { getData: () => "hello there" },
    });
    expect(screen.getByText(/doesn't look like a connection string/)).toBeTruthy();
  });
});

describe("ConnectionDialog database cards", () => {
  afterEach(cleanup);

  const renderNew = (note?: string) =>
    render(
      <ConnectionDialog
        initial={newConnection()}
        note={note}
        onClose={() => {}}
        onSaved={() => {}}
        onDeleted={() => {}}
      />,
    );

  it("starts a new connection on a card for every database, each with its logo", () => {
    renderNew();
    for (const name of ["PostgreSQL", "MySQL", "MariaDB", "SQL Server", "Azure SQL", "SQLite"])
      expect(screen.getByRole("button", { name: new RegExp(`^${name}`) })).toBeTruthy();
    expect(screen.queryByLabelText("Host")).toBeNull();
  });

  it("filters the cards, and Enter picks the one left", () => {
    renderNew();
    const box = screen.getByLabelText("Filter databases");
    fireEvent.change(box, { target: { value: "azure" } });
    expect(screen.queryByRole("button", { name: /^PostgreSQL/ })).toBeNull();
    fireEvent.keyDown(box, { key: "Enter" });
    expect(screen.getByText("Azure SQL")).toBeTruthy();
    expect((screen.getByLabelText("Port") as HTMLInputElement).value).toBe("1433");
    expect(screen.getByLabelText<HTMLInputElement>("Encrypt connection").checked).toBe(true);
  });

  it("says so when no database matches", () => {
    renderNew();
    fireEvent.change(screen.getByLabelText("Filter databases"), { target: { value: "oracle" } });
    expect(screen.getByText("No database matches")).toBeTruthy();
  });

  it("goes back to the cards from the form", () => {
    renderNew();
    fireEvent.click(screen.getByRole("button", { name: /^SQLite/ }));
    expect(screen.getByLabelText("Database file")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "All databases" }));
    expect(screen.getByLabelText("Filter databases")).toBeTruthy();
  });

  it("skips the cards for details an AI app filled in", () => {
    renderNew("From Claude (MCP)");
    expect(screen.queryByLabelText("Filter databases")).toBeNull();
    expect(screen.getByLabelText("Host")).toBeTruthy();
  });
});
