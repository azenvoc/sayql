import { useEffect, useState } from "react";
import { Check, Copy, Pencil } from "lucide-react";
import {
  commands,
  type AgentAccess,
  type AgentAccessSettings,
  type AppResults,
  type ConnectionConfig,
} from "../../api";
import { Alert, Badge, Button, Select } from "../../components/ui";
import { cx } from "../../lib/cx";
import { errorText } from "../../lib/errors";
import {
  APP_OPTIONS,
  AgentAccessDialog,
  AuditButton,
  NEVER_SEND_SCOPE,
  withDefault,
  NeverSendLabel,
  auditAge,
  auditCommand,
  auditStale,
  copyAuditPrompt,
  lastAudit,
  otherAudits,
  saveAgentAccess,
  useAudits,
} from "../agent";
import { DATABASE_KINDS } from "../connections";
import styles from "./ClaudeIntegration.module.css";

/** Per saved connection: what AI apps over MCP get back from a query, and the columns never sent. */
export function AppAccessList({
  connections,
  provider,
  onReview,
}: {
  connections: ConnectionConfig[];
  provider: { label: string; local: boolean };
  /** Shows the suggestions, from a finished audit run. */
  onReview?: () => void;
}) {
  // MCP serves the connections saved on this computer, not a team's.
  const local = connections.filter((c) => !c.id.startsWith("team:"));
  const [access, setAccess] = useState<Record<string, AgentAccess>>({});
  const [error, setError] = useState<string | null>(null);
  const [editing, setEditing] = useState<ConnectionConfig | null>(null);
  const audits = useAudits();
  const [copied, setCopied] = useState<string | null>(null);
  const copyPrompt = (c: ConnectionConfig) => {
    setError(null);
    copyAuditPrompt({ connectionId: c.id, database: null }).then(
      () => {
        setCopied(c.id);
        window.setTimeout(() => setCopied((id) => (id === c.id ? null : id)), 1500);
      },
      (e) => setError(errorText(e)),
    );
  };

  useEffect(() => {
    let live = true;
    Promise.all(local.map((c) => commands.agentAccess(c.id).then((a) => [c.id, a] as const))).then(
      (pairs) => live && setAccess(Object.fromEntries(pairs)),
      (e) => live && setError(errorText(e)),
    );
    return () => {
      live = false;
    };
    // The list of ids, not the array, decides what to load.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [local.map((c) => c.id).join("\n")]);

  const save = (id: string, next: AgentAccessSettings) =>
    saveAgentAccess(id, next).then((saved) => setAccess((all) => ({ ...all, [id]: saved })));

  const choose = (id: string, apps: AppResults) => {
    const { team: _team, ...current } = access[id];
    setError(null);
    save(id, { ...current, apps }).catch((e) => setError(errorText(e)));
  };

  return (
    <section className={styles.group} aria-labelledby="ai-apps-connections">
      <h3 id="ai-apps-connections" className={styles.heading}>
        Connections
      </h3>
      <p className={styles.intro}>
        Per connection, choose what an AI app gets back from a query. {NEVER_SEND_SCOPE}{" "}
        <strong>Last audit</strong> is when an AI app last fetched the audit prompt or ran it from
        here, which teaches SayQL about the database from your code; run it again after big changes.{" "}
        <strong>Run audit</strong> runs your installed Claude Code, or SayQL&apos;s built-in
        OpenCode, read-only over the database&apos;s knowledge sources.
      </p>
      <dl className={styles.legend} aria-label="What each choice sends">
        {APP_OPTIONS.map((r) => (
          <div key={r.value} className={styles.legendItem}>
            <dt>{withDefault(r)}</dt>
            <dd>{r.detail}</dd>
          </div>
        ))}
      </dl>
      {local.length === 0 ? (
        <p className={styles.empty}>
          No saved connections yet. AI apps can use the connections saved on this computer.
        </p>
      ) : (
        <div className={styles.tableWrap}>
          <table className={cx(styles.table, styles.accessTable)}>
            <colgroup>
              <col />
              <col className={styles.colChoice} />
              <col className={styles.colNever} />
              <col className={styles.colAudit} />
            </colgroup>
            <thead>
              <tr>
                <th>Connection</th>
                <th>What AI apps get back</th>
                <th>Never send</th>
                <th>Last audit</th>
              </tr>
            </thead>
            <tbody>
              {local.map((c) => {
                const a = access[c.id];
                // The row's Run audit is for the default database, so that's what it reports.
                const audited = lastAudit(audits, c);
                return (
                  <tr key={c.id}>
                    <td>
                      <div className={styles.connName}>
                        <span className={styles.ellipsis} title={c.name}>
                          {c.name}
                        </span>
                        <span className={styles.detail}>
                          {DATABASE_KINDS[c.kind]?.label ?? c.kind}
                        </span>
                      </div>
                    </td>
                    <td>
                      <Select
                        aria-label={`What AI apps may see on ${c.name}`}
                        className={styles.choice}
                        value={a?.apps ?? "counts"}
                        disabled={!a}
                        options={APP_OPTIONS.map((r) => ({
                          value: r.value,
                          label: withDefault(r),
                        }))}
                        onChange={(v) => choose(c.id, v)}
                      />
                    </td>
                    <td>
                      <div className={styles.chips}>
                        {a && a.neverSend.length === 0 && (
                          <span className={styles.detail}>None</span>
                        )}
                        {a?.neverSend.map((col) => (
                          <Badge
                            key={col}
                            title={col.includes(".") ? undefined : `${col}, in every table`}
                          >
                            <span className={styles.chip}>
                              <NeverSendLabel entry={col} />
                            </span>
                          </Badge>
                        ))}
                        <Button
                          size="sm"
                          variant="ghost"
                          icon={Pencil}
                          disabled={!a}
                          aria-label={`Edit never-send columns on ${c.name}`}
                          onClick={() => setEditing(c)}
                        >
                          Edit
                        </Button>
                      </div>
                    </td>
                    <td>
                      <div className={styles.audit}>
                        <span>{auditAge(audited)}</span>
                        {otherAudits(audits, c).map((o) => (
                          <span key={o.database} className={styles.detail}>
                            {o.database}: {auditAge(o.at)}
                          </span>
                        ))}
                        {auditStale(audited) && (
                          <span
                            className={styles.nudge}
                            title={`In your project, type ${auditCommand({ connectionId: c.id, name: c.name, database: null })} in Claude Code, or copy the audit prompt into another app. Code moves on and notes go stale.`}
                          >
                            Run an audit
                          </span>
                        )}
                        <Button
                          size="sm"
                          variant="ghost"
                          icon={copied === c.id ? Check : Copy}
                          aria-label={`Copy the audit prompt for ${c.name}`}
                          title="Copy the audit instructions, for AI apps without MCP prompts: paste them into the chat."
                          onClick={() => copyPrompt(c)}
                        >
                          {copied === c.id ? "Copied" : "Copy prompt"}
                        </Button>
                        <AuditButton
                          variant="ghost"
                          target={{ connectionId: c.id, name: c.name, database: null }}
                          onReview={onReview}
                        />
                      </div>
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
      )}
      {error && <Alert tone="danger">{error}</Alert>}
      {editing && access[editing.id] && (
        <AgentAccessDialog
          neverSendOnly
          connectionName={editing.name}
          target={{ connectionId: editing.id, database: null }}
          provider={provider}
          access={access[editing.id]}
          onSave={async (next) => void (await save(editing.id, next))}
          onClose={() => setEditing(null)}
        />
      )}
    </section>
  );
}
