// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { commands, type ConnectionConfig } from "../../api";
import { AllowedDatabases } from "./AllowedDatabases";

const connection: ConnectionConfig = {
  id: "",
  name: "shared",
  kind: "mssql",
  host: "sql.example.com",
  port: 1433,
  database: "",
  user: "reader",
  file: "",
  ssl: true,
  trustCert: false,
  production: false,
};

function show(allowed: string[] | null, sharedWith: string | null = null) {
  const onChange = vi.fn();
  render(
    <AllowedDatabases
      connection={connection}
      password="typed-in-the-form"
      workspaceName="Acme"
      sharedWith={sharedWith}
      allowed={allowed}
      onChange={onChange}
    />,
  );
  return onChange;
}

describe("AllowedDatabases", () => {
  afterEach(cleanup);

  it("allows everything until the person picks, as a connection always has", () => {
    show(null);
    expect(screen.queryByText("List databases on this server")).toBeNull();
  });

  it("warns when another workspace uses this server and everything is allowed", () => {
    show(null, "Harbor Health");
    expect(screen.getByText(/Harbor Health also uses this server/)).toBeTruthy();
  });

  it("starts an empty list when the person chooses to pick", () => {
    const onChange = show(null);
    fireEvent.click(screen.getByText("Only the ones I pick"));
    expect(onChange).toHaveBeenCalledWith([]);
  });

  it("lists with what's in the form and says only a separate login guarantees it", async () => {
    const list = vi
      .spyOn(commands, "serverDatabases")
      .mockResolvedValue({ databases: ["acme", "harbor"], current: null });
    show(["acme"]);
    fireEvent.click(screen.getByText("List databases on this server"));
    expect(await screen.findByText(/can still reach 1 other database/)).toBeTruthy();
    expect(screen.getByText(/only a separate login/)).toBeTruthy();
    expect(list).toHaveBeenCalledWith(connection, "typed-in-the-form");
  });

  it("keeps a saved choice on screen before the server is listed", () => {
    show(["acme"]);
    expect((screen.getByLabelText("acme") as HTMLInputElement).checked).toBe(true);
  });

  it("adds and removes databases", () => {
    const onChange = show(["acme", "crm"]);
    fireEvent.click(screen.getByLabelText("crm"));
    expect(onChange).toHaveBeenCalledWith(["acme"]);
  });

  it("shows a server's refusal", async () => {
    vi.spyOn(commands, "serverDatabases").mockRejectedValue("Login failed.");
    show([]);
    fireEvent.click(screen.getByText("List databases on this server"));
    expect(await screen.findByText("Login failed.")).toBeTruthy();
  });
});
