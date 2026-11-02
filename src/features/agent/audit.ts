import { useCallback, useEffect, useState } from "react";
import { commands, type AuditRun } from "../../api";
import { promptArgs } from "./agents";

/** Days after which Settings suggests auditing a connection again. */
export const AUDIT_STALE_DAYS = 30;
const DAY = 24 * 60 * 60 * 1000;

/** The connection an audit is for. */
export interface AuditTarget {
  connectionId: string;
  name: string;
  /** `null`: a SQLite file, or the connection's default database. */
  database: string | null;
}

/**
 * What to type in Claude Code to run the audit. Claude Code passes words after the command as
 * the prompt's arguments in order (connection, then database), so a name with spaces goes by id.
 */
export function auditCommand(target?: AuditTarget | null): string {
  return `/mcp__sayql__audit${promptArgs(target)}`;
}

/** When the connection was last audited (any of its databases), or null. */
export function lastAudit(audits: AuditRun[], connectionId: string): number | null {
  const times = audits.filter((a) => a.connectionId === connectionId).map((a) => a.at);
  return times.length ? Math.max(...times) : null;
}

/** "Never", "today", "yesterday", "3 days ago", "2 months ago". */
export function auditAge(at: number | null, now = Date.now()): string {
  if (!at) return "Never";
  const days = Math.floor(Math.max(0, now - at) / DAY);
  const rtf = new Intl.RelativeTimeFormat("en", { numeric: "auto" });
  if (days < 1) return "today";
  if (days < 45) return rtf.format(-days, "day");
  if (days < 365) return rtf.format(-Math.round(days / 30), "month");
  return rtf.format(-Math.round(days / 365), "year");
}

/** Never audited, or not in the last 30 days. */
export function auditStale(at: number | null, now = Date.now()): boolean {
  return !at || now - at > AUDIT_STALE_DAYS * DAY;
}

/** Copies the audit instructions for `target` (for AI apps without MCP prompts). */
export async function copyAuditPrompt(
  target: Pick<AuditTarget, "connectionId" | "database">,
): Promise<void> {
  const text = await commands.auditPrompt(target.connectionId, target.database);
  await navigator.clipboard.writeText(text);
}

/** Fired on window when an audit run in the app ends, so "Last audit" reads again. */
export const AUDITS_CHANGED = "sayql:audits-changed";

/** When AI apps last fetched the audit prompt, read again when SayQL comes to the front. */
export function useAudits() {
  const [audits, setAudits] = useState<AuditRun[]>([]);
  const load = useCallback(() => {
    Promise.resolve()
      .then(() => commands.mcpAudits())
      .then(setAudits, () => setAudits([]));
  }, []);
  useEffect(() => {
    load();
    window.addEventListener("focus", load);
    window.addEventListener(AUDITS_CHANGED, load);
    return () => {
      window.removeEventListener("focus", load);
      window.removeEventListener(AUDITS_CHANGED, load);
    };
  }, [load]);
  return audits;
}
