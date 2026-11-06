import type { ConnectionConfig, ConnectionDraft, Handoff, Target } from "../../api";
import { DATABASE_KINDS } from "../connections";
import type { NewTab } from "../tabs";

/** What a handoff can do in the window. Nothing here runs SQL or saves a connection. */
export interface HandoffActions {
  connection: (id: string) => ConnectionConfig | null;
  /** Shows the New connection dialog, filled in, with `note` on top. */
  openConnectionDialog: (
    config: ConnectionConfig,
    note: string,
    /** The workspace and group the asking AI app serves. */
    place: { workspaceId: string | null; groupId: string | null },
  ) => void;
  openTab: (target: Target, init: NewTab) => void;
  /** Opens a saved script (or shows its open tab); false when there's no such script. */
  openScript: (id: string) => boolean;
  /** Shows "Sign in to your team", filled in from an invite link. Never signs in on its own. */
  openTeamSignIn: (server: string, code: string) => void;
  /** Finishes "Sign in with your company" with the code the browser came back with. */
  finishTeamSso: (code: string) => void;
  notify: (text: string) => void;
}

/** "From Claude (MCP)". */
export function handoffNote(source: string | null): string {
  return `From ${source || "an AI app"} (MCP)`;
}

/** The New connection form, filled in from an AI app's draft. The id stays empty: it's new. */
export function draftToConnection(d: ConnectionDraft): ConnectionConfig {
  const kind = DATABASE_KINDS[d.kind];
  return {
    id: "",
    name: d.name,
    kind: d.kind,
    host: d.kind === "sqlite" ? "" : d.host || "localhost",
    port: d.kind === "sqlite" ? null : (d.port ?? kind.defaultPort),
    database: d.database,
    user: d.user,
    file: d.file,
    ssl: d.ssl ?? kind.defaultSsl,
    trustCert: d.trustCert ?? false,
    production: false,
  };
}

/** Shows work handed over by an AI app, for the person to review and act on. */
export function applyHandoff(h: Handoff, a: HandoffActions): void {
  if (h.type === "teamJoin") {
    a.openTeamSignIn(h.server, h.code);
    return;
  }
  if (h.type === "teamSso") {
    a.finishTeamSso(h.code);
    return;
  }
  const note = handoffNote(h.source);
  if (h.type === "newConnection") {
    a.openConnectionDialog(draftToConnection(h), note, {
      workspaceId: h.workspace,
      groupId: h.group,
    });
    return;
  }
  if (h.type === "openScript") {
    if (!a.openScript(h.scriptId))
      a.notify(`${note} asked to open a script that isn't saved here.`);
    return;
  }
  const c = a.connection(h.connectionId);
  if (!c) {
    a.notify(`${note} asked to open SQL on a connection that isn't saved here.`);
    return;
  }
  a.openTab(
    { connectionId: c.id, database: h.database ?? (c.database.trim() || null) },
    // No autoRun: the person reviews the SQL and clicks Run.
    { title: h.title ?? "", question: h.question ?? "", sql: h.sql, origin: note },
  );
}
