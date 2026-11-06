import { expect, it } from "vitest";
import { auditAge, auditCommand, auditStale, lastAudit, notesDatabase, otherAudits } from "./audit";

const DAY = 86_400_000;
const NOW = Date.UTC(2026, 9, 28, 12);

it("names the connection in the command, by id when its name has spaces", () => {
  expect(auditCommand()).toBe("/mcp__sayql__audit");
  expect(auditCommand({ connectionId: "c1", name: "Shop", database: null })).toBe(
    "/mcp__sayql__audit Shop",
  );
  expect(auditCommand({ connectionId: "c1", name: "Demo store", database: "sales" })).toBe(
    "/mcp__sayql__audit c1 sales",
  );
});

it("says when a connection was last audited, and when to run it again", () => {
  const audits = [
    { connectionId: "c1", database: "a", app: "Cursor", at: NOW - 40 * DAY },
    { connectionId: "c1", database: "b", app: "Cursor", at: NOW - 3 * DAY },
  ];
  const c1 = { id: "c1", database: "a" };
  expect(lastAudit(audits, c1)).toBe(NOW - 40 * DAY);
  expect(lastAudit(audits, c1, "b")).toBe(NOW - 3 * DAY);
  expect(lastAudit(audits, { id: "c2", database: "" })).toBeNull();
  expect(auditAge(null, NOW)).toBe("Never");
  expect(auditAge(NOW - 3 * DAY, NOW)).toBe("3 days ago");
  expect(auditAge(NOW - 1 * DAY, NOW)).toBe("yesterday");
  expect(auditAge(NOW - 60_000, NOW)).toBe("today");
  expect(auditAge(NOW - 90 * DAY, NOW)).toBe("3 months ago");
  expect(auditStale(null, NOW)).toBe(true);
  expect(auditStale(NOW - 29 * DAY, NOW)).toBe(false);
  expect(auditStale(NOW - 31 * DAY, NOW)).toBe(true);
});

it("doesn't let one tenant's audit make every database look audited", () => {
  const audits = [{ connectionId: "server", database: "tenant_a", app: "Cursor", at: NOW }];
  const server = { id: "server", database: "" };
  expect(lastAudit(audits, server, "tenant_a")).toBe(NOW);
  expect(lastAudit(audits, server, "tenant_b")).toBeNull();
  expect(lastAudit(audits, server)).toBeNull();
  expect(otherAudits(audits, server)).toEqual([{ database: "tenant_a", at: NOW }]);
  expect(otherAudits(audits, server, "tenant_a")).toEqual([]);
});

it("treats no database and the connection's default as the same place", () => {
  const conn = { id: "c1", database: "shop" };
  const named = [{ connectionId: "c1", database: "shop", app: "Cursor", at: NOW }];
  const unnamed = [{ connectionId: "c1", database: null, app: "Cursor", at: NOW }];
  for (const audits of [named, unnamed]) {
    expect(lastAudit(audits, conn)).toBe(NOW);
    expect(lastAudit(audits, conn, "shop")).toBe(NOW);
    expect(otherAudits(audits, conn)).toEqual([]);
  }
  expect(notesDatabase(" ", " shop ")).toBe("shop");
  expect(notesDatabase(null, "")).toBeNull();
});

it("lists other audited databases, most recent first", () => {
  const audits = [
    { connectionId: "c1", database: "old", app: "Cursor", at: NOW - DAY },
    { connectionId: "c1", database: "new", app: "Cursor", at: NOW },
    { connectionId: "c2", database: "elsewhere", app: "Cursor", at: NOW },
  ];
  expect(otherAudits(audits, { id: "c1", database: "" }).map((o) => o.database)).toEqual([
    "new",
    "old",
  ]);
});
