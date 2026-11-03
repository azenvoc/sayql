import { expect, it } from "vitest";
import { auditAge, auditCommand, auditStale, lastAudit } from "./audit";

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
  expect(lastAudit(audits, "c1")).toBe(NOW - 3 * DAY);
  expect(lastAudit(audits, "c2")).toBeNull();
  expect(auditAge(null, NOW)).toBe("Never");
  expect(auditAge(NOW - 3 * DAY, NOW)).toBe("3 days ago");
  expect(auditAge(NOW - 1 * DAY, NOW)).toBe("yesterday");
  expect(auditAge(NOW - 60_000, NOW)).toBe("today");
  expect(auditAge(NOW - 90 * DAY, NOW)).toBe("3 months ago");
  expect(auditStale(null, NOW)).toBe(true);
  expect(auditStale(NOW - 29 * DAY, NOW)).toBe(false);
  expect(auditStale(NOW - 31 * DAY, NOW)).toBe(true);
});
