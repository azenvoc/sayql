import { useEffect, useState } from "react";
import { Bot, Database, Pencil, Plug, ShieldCheck, StickyNote } from "lucide-react";
import { commands, type ConnectionConfig, type McpActivity, type Schema } from "../api";
import { Button, Dialog } from "../components/ui";
import {
  AuditButton,
  KnowledgeSources,
  auditAge,
  lastAudit,
  otherAudits,
  privacySummary,
  useAgentAccess,
  useAudits,
} from "../features/agent";
import { MyActivityLog } from "../features/settings";
import { DATABASE_KINDS, ProdBadge } from "../features/connections";
import styles from "./ConnectionOverview.module.css";

export interface ConnectionOverviewProps {
  connection: ConnectionConfig;
  connections: ConnectionConfig[];
  /** The database open in the active tab, when this is its connection. */
  database?: string | null;
  /** Its schema, when it's the open connection and connected. */
  schema: Schema | null;
  /** Its notes, when it's the open connection. */
  notes: number | null;
  provider: { label: string; local: boolean } | null;
  onClose: () => void;
  onEdit?: () => void;
  onConnect: () => void;
  onAccess: () => void;
  onOpenSql: (connectionId: string, database: string | null, sql: string) => void;
}

/** "PostgreSQL · db.internal:5432 / analytics", or the file for SQLite. */
function where(c: ConnectionConfig): string {
  const kind = DATABASE_KINDS[c.kind].label;
  if (c.team) return `${kind} · on your team's server`;
  if (c.kind === "sqlite") return `${kind} · ${c.file}`;
  return `${kind} · ${c.host}${c.port ? `:${c.port}` : ""}${c.database ? ` / ${c.database}` : ""}`;
}

/**
 * One connection at a glance: what it is, what can reach an AI from it, what AI apps did with it
 * and when it was last audited, and your own recent queries on it.
 */
export function ConnectionOverview(p: ConnectionOverviewProps) {
  const { connection: c } = p;
  const access = useAgentAccess(c.id);
  const audits = useAudits();
  const [apps, setApps] = useState<McpActivity[] | null>(null);
  // The local AI apps' log; team connections aren't served to them.
  const team = !!c.team;
  useEffect(() => {
    if (team) return;
    commands.mcpActivity(1000).then(
      (log) => setApps(log.filter((e) => e.connectionId === c.id)),
      () => setApps([]),
    );
  }, [c.id, team]);

  const summary = access ? privacySummary(access, p.provider) : null;
  const audited = lastAudit(audits, c, p.database);
  const others = otherAudits(audits, c, p.database);
  const appNames = [...new Set((apps ?? []).map((e) => e.app))];
  const refused = (apps ?? []).filter((e) => e.status === "refused").length;
  const columns = p.schema?.tables.reduce((n, t) => n + t.columns.length, 0) ?? 0;

  return (
    <Dialog
      title={c.name}
      onClose={p.onClose}
      size="lg"
      actions={<Button onClick={p.onClose}>Close</Button>}
    >
      <div className={styles.body}>
        <p className={styles.where}>
          {where(c)} <ProdBadge connection={c} />
        </p>
        <div className={styles.cards}>
          <section className={styles.card} aria-labelledby="ov-db">
            <h3 id="ov-db" className={styles.cardTitle}>
              <Database size={14} /> Database
            </h3>
            {p.schema ? (
              <p className={styles.big}>
                {p.schema.tables.length} tables · {columns} columns
              </p>
            ) : (
              <p className={styles.muted}>Not connected, so SayQL hasn&apos;t read its tables.</p>
            )}
            {p.notes != null && (
              <p className={styles.muted}>
                <StickyNote size={12} /> {p.notes} database {p.notes === 1 ? "note" : "notes"}
              </p>
            )}
            <div className={styles.actions}>
              {!p.schema && (
                <Button size="sm" icon={Plug} onClick={p.onConnect}>
                  Connect
                </Button>
              )}
              {p.onEdit && (
                <Button size="sm" variant="ghost" icon={Pencil} onClick={p.onEdit}>
                  Edit
                </Button>
              )}
            </div>
          </section>

          <section className={styles.card} aria-labelledby="ov-ai">
            <h3 id="ov-ai" className={styles.cardTitle}>
              <ShieldCheck size={14} /> What can reach an AI
            </h3>
            {summary && (
              <>
                <p className={styles.big}>{summary.label}</p>
                <ul className={styles.lines}>
                  {summary.lines.map((l) => (
                    <li key={l}>{l}</li>
                  ))}
                </ul>
              </>
            )}
            <div className={styles.actions}>
              <Button size="sm" icon={Bot} onClick={p.onAccess}>
                Change
              </Button>
            </div>
          </section>

          <section className={styles.card} aria-labelledby="ov-apps">
            <h3 id="ov-apps" className={styles.cardTitle}>
              <Plug size={14} /> AI apps
            </h3>
            {c.team ? (
              <p className={styles.muted}>
                AI apps on this computer can&apos;t use team connections. AI agents reach them only
                through your team&apos;s agent gateway, with their own keys and the rules your admin
                sets; what they run is in the team&apos;s audit log.
              </p>
            ) : apps === null ? (
              <p className={styles.muted}>Loading…</p>
            ) : apps.length === 0 ? (
              <p className={styles.muted}>No AI app has used this connection yet.</p>
            ) : (
              <>
                <p className={styles.big}>
                  {apps.length} {apps.length === 1 ? "request" : "requests"}
                  {refused > 0 && ` · ${refused} refused`}
                </p>
                <p className={styles.muted}>From {appNames.join(", ")}</p>
              </>
            )}
            {!c.team && (
              <>
                <p className={styles.muted}>
                  Last audit{p.database ? ` of ${p.database}` : ""}:{" "}
                  {auditAge(audited).toLowerCase()}
                </p>
                {others.map((o) => (
                  <p key={o.database} className={styles.muted}>
                    {o.database}: {auditAge(o.at).toLowerCase()}
                  </p>
                ))}
                <div className={styles.actions}>
                  <AuditButton target={{ connectionId: c.id, name: c.name, database: null }} />
                </div>
              </>
            )}
          </section>
        </div>

        {!c.team && (
          <>
            <h3 className={styles.sectionTitle}>Knowledge sources</h3>
            <p className={styles.muted}>
              The code and documents the audit reads for this connection. They stay on this computer
              and go only to the AI you choose, when you run an audit.
            </p>
            <KnowledgeSources connectionId={c.id} />
          </>
        )}

        <h3 className={styles.sectionTitle}>Your recent queries here</h3>
        <MyActivityLog connections={p.connections} connectionId={c.id} onOpenSql={p.onOpenSql} />
      </div>
    </Dialog>
  );
}
