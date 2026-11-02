import { describe, expect, it, vi } from "vitest";
import type { ConnectionConfig, Handoff } from "../../api";
import { applyHandoff, draftToConnection, handoffNote, type HandoffActions } from "./applyHandoff";

const demo: ConnectionConfig = {
  id: "demo",
  name: "Demo store (SQLite)",
  kind: "sqlite",
  host: "",
  port: null,
  database: "",
  user: "",
  file: "/demo.sqlite",
  ssl: false,
  trustCert: false,
};

function actions(connections: ConnectionConfig[] = [demo]) {
  return {
    connection: (id: string) => connections.find((c) => c.id === id) ?? null,
    openConnectionDialog: vi.fn(),
    openTab: vi.fn(),
    openScript: vi.fn((id: string) => id === "s1"),
    openTeamSignIn: vi.fn(),
    finishTeamSso: vi.fn(),
    notify: vi.fn(),
  } satisfies HandoffActions;
}

const draft = {
  name: "",
  host: "",
  port: null,
  database: "",
  user: "",
  file: "",
  ssl: null,
  trustCert: null,
};

describe("applyHandoff", () => {
  it("prefills the New connection dialog, without a password or id", () => {
    const a = actions();
    const h: Handoff = {
      source: "Claude",
      type: "newConnection",
      ...draft,
      kind: "postgres",
      host: "db.example.com",
      database: "shop",
      user: "reader",
    };
    applyHandoff(h, a);
    expect(a.openConnectionDialog).toHaveBeenCalledWith(
      {
        id: "",
        name: "",
        kind: "postgres",
        host: "db.example.com",
        port: 5432,
        database: "shop",
        user: "reader",
        file: "",
        ssl: false,
        trustCert: false,
      },
      "From Claude (MCP)",
    );
    expect(a.openConnectionDialog.mock.calls[0][0]).not.toHaveProperty("password");
    expect(a.openTab).not.toHaveBeenCalled();
  });

  it("uses each kind's defaults for what the AI app left out", () => {
    expect(draftToConnection({ ...draft, kind: "mssql", host: "sql.local" })).toMatchObject({
      port: 1433,
      ssl: true,
      trustCert: false,
    });
    expect(draftToConnection({ ...draft, kind: "mysql", port: 3307, ssl: true })).toMatchObject({
      host: "localhost",
      port: 3307,
      ssl: true,
    });
    expect(
      draftToConnection({ ...draft, kind: "sqlite", file: "/tmp/a.db", host: "x", port: 1 }),
    ).toMatchObject({ host: "", port: null, file: "/tmp/a.db" });
  });

  it("opens SQL in a new tab without running it", () => {
    const a = actions();
    const h: Handoff = {
      source: null,
      type: "openQuery",
      connectionId: "demo",
      database: null,
      sql: "SELECT 1",
      title: "Check",
      question: "Is it up?",
    };
    applyHandoff(h, a);
    expect(a.openTab).toHaveBeenCalledWith(
      { connectionId: "demo", database: null },
      { title: "Check", question: "Is it up?", sql: "SELECT 1", origin: "From an AI app (MCP)" },
    );
    const init = a.openTab.mock.calls[0][1];
    expect(init.autoRun).toBeUndefined();
    expect(init.autoAsk).toBeUndefined();
  });

  it("uses the connection's default database unless one is given", () => {
    const pg = { ...demo, id: "pg", kind: "postgres" as const, database: "shop" };
    const a = actions([pg]);
    const base = {
      source: "Cursor",
      type: "openQuery" as const,
      connectionId: "pg",
      sql: "SELECT 1",
      title: null,
      question: null,
    };
    applyHandoff({ ...base, database: null }, a);
    applyHandoff({ ...base, database: "analytics" }, a);
    expect(a.openTab.mock.calls.map((c) => c[0].database)).toEqual(["shop", "analytics"]);
  });

  it("says so when the connection isn't saved here", () => {
    const a = actions();
    applyHandoff(
      {
        source: "Claude",
        type: "openQuery",
        connectionId: "gone",
        database: null,
        sql: "SELECT 1",
        title: null,
        question: null,
      },
      a,
    );
    expect(a.openTab).not.toHaveBeenCalled();
    expect(a.notify).toHaveBeenCalledWith(expect.stringContaining("From Claude (MCP)"));
  });

  it("opens a saved script, or says it isn't saved", () => {
    const a = actions();
    applyHandoff({ source: "Claude", type: "openScript", scriptId: "s1" }, a);
    expect(a.openScript).toHaveBeenCalledWith("s1");
    expect(a.notify).not.toHaveBeenCalled();
    applyHandoff({ source: "Claude", type: "openScript", scriptId: "gone" }, a);
    expect(a.notify).toHaveBeenCalledWith(expect.stringContaining("script"));
    expect(a.openTab).not.toHaveBeenCalled();
  });

  it("opens the team sign-in dialog from an invite link, and nothing else", () => {
    const a = actions();
    applyHandoff(
      { source: null, type: "teamJoin", server: "https://sayql.acme.internal", code: "ABCD" },
      a,
    );
    expect(a.openTeamSignIn).toHaveBeenCalledWith("https://sayql.acme.internal", "ABCD");
    expect(a.openTab).not.toHaveBeenCalled();
    expect(a.openConnectionDialog).not.toHaveBeenCalled();
  });

  it("finishes a company sign-in from its link", () => {
    const a = actions();
    applyHandoff(
      { source: null, type: "teamSso", server: "https://sayql.acme.internal", code: "sqt_abc" },
      a,
    );
    expect(a.finishTeamSso).toHaveBeenCalledWith("sqt_abc");
    expect(a.openTeamSignIn).not.toHaveBeenCalled();
  });

  it("names the AI app", () => {
    expect(handoffNote("Claude Code")).toBe("From Claude Code (MCP)");
    expect(handoffNote(null)).toBe("From an AI app (MCP)");
  });
});
