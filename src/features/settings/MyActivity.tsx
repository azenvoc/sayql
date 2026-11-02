import { Fragment, useCallback, useEffect, useState } from "react";
import { ChevronRight, RefreshCw } from "lucide-react";
import { commands, type ConnectionConfig, type MyActivity } from "../../api";
import { Alert, Button, SegmentedControl, Select } from "../../components/ui";
import { cx } from "../../lib/cx";
import { errorText } from "../../lib/errors";
import { when } from "./AppsList";
import styles from "./ClaudeIntegration.module.css";

/** How much of the log to read; the backend keeps more. */
const LOG_LIMIT = 300;
const ALL = "";
const SOURCE: Record<MyActivity["source"], string> = {
  editor: "Editor",
  ask: "Ask box",
  investigate: "Investigate",
};
const STATUS: Record<MyActivity["status"], string> = {
  ok: "OK",
  error: "Error",
  cancelled: "Cancelled",
};

type SourceFilter = "all" | MyActivity["source"];

/** The time today, the date otherwise. */
function short(ms: number): string {
  const d = new Date(ms);
  return d.toDateString() === new Date().toDateString()
    ? d.toLocaleTimeString(undefined, { timeStyle: "short" })
    : d.toLocaleString(undefined, {
        month: "short",
        day: "numeric",
        hour: "numeric",
        minute: "2-digit",
      });
}

function result(e: MyActivity): string {
  const rows =
    e.status === "ok" && e.rows != null ? ` · ${e.rows} ${e.rows === 1 ? "row" : "rows"}` : "";
  const ms =
    e.status === "ok" && e.ms != null
      ? ` · ${e.ms < 1000 ? `${e.ms} ms` : `${(e.ms / 1000).toFixed(1)} s`}`
      : "";
  return `${STATUS[e.status]}${rows}${ms}`;
}

export interface MyActivityLogProps {
  connections: ConnectionConfig[];
  /** Only this connection's queries, without the connection filter (the connection overview). */
  connectionId?: string;
  /** Opens the SQL in a new tab on its connection. */
  onOpenSql?: (connectionId: string, database: string | null, sql: string) => void;
}

/**
 * Your own queries in SayQL, from the editor, the Ask box and Investigate: kept on this computer,
 * with the SQL and counts, never rows. For Investigate it also says what went to the AI.
 */
export function MyActivityLog({ connections, connectionId, onOpenSql }: MyActivityLogProps) {
  const [log, setLog] = useState<MyActivity[]>([]);
  const [loaded, setLoaded] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [connection, setConnection] = useState(ALL);
  const [source, setSource] = useState<SourceFilter>("all");
  const [open, setOpen] = useState<Set<number>>(new Set());

  const load = useCallback(() => {
    setError(null);
    commands.myActivity(connectionId ?? null, LOG_LIMIT).then(
      (l) => {
        setLog(l);
        setOpen(new Set());
        setLoaded(true);
      },
      (e) => {
        setLoaded(true);
        setError(errorText(e));
      },
    );
  }, [connectionId]);
  useEffect(load, [load]);

  const name = (id: string) => connections.find((c) => c.id === id)?.name ?? "Deleted connection";
  const clear = () => {
    if (!window.confirm("Delete your activity log? Your saved queries and tabs aren't affected."))
      return;
    commands.clearMyActivity().then(load, (e) => setError(errorText(e)));
  };
  const toggle = (i: number) =>
    setOpen((s) => {
      const next = new Set(s);
      if (!next.delete(i)) next.add(i);
      return next;
    });

  const ids = [...new Set(log.map((e) => e.connectionId))];
  const rows = log
    .map((e, i) => ({ e, i }))
    .filter(
      ({ e }) =>
        (connection === ALL || e.connectionId === connection) &&
        (source === "all" || e.source === source),
    );
  const columns = connectionId ? 4 : 5;

  return (
    <div className={styles.activity}>
      <p className={styles.intro}>
        The queries you ran in SayQL{connectionId ? " on this connection" : ""}: from the editor,
        the Ask box and Investigate. It stays on this computer and keeps the SQL and how many rows
        came back, never the rows. For Investigate, it also says what went to the AI. Click a row
        for the full SQL.
      </p>
      <div className={styles.filters}>
        {!connectionId && (
          <Select
            aria-label="Connection"
            className={styles.filter}
            value={connection}
            options={[
              { value: ALL, label: "All connections" },
              ...ids.map((id) => ({ value: id, label: name(id) })),
            ]}
            onChange={setConnection}
          />
        )}
        <div className={styles.resultFilter}>
          <SegmentedControl<SourceFilter>
            ariaLabel="From"
            value={source}
            options={[
              { value: "all", label: "All" },
              { value: "editor", label: "Editor" },
              { value: "ask", label: "Ask" },
              { value: "investigate", label: "Investigate" },
            ]}
            onChange={setSource}
          />
        </div>
        <span className={styles.spacer} />
        <Button size="sm" variant="ghost" icon={RefreshCw} onClick={load}>
          Refresh
        </Button>
      </div>
      {loaded && log.length === 0 ? (
        <p className={styles.empty}>Nothing yet. Queries you run show up here.</p>
      ) : loaded && rows.length === 0 ? (
        <p className={styles.empty}>Nothing in the log matches these filters.</p>
      ) : (
        rows.length > 0 && (
          <div className={styles.tableWrap}>
            <table className={cx(styles.table, styles.logTable)}>
              <colgroup>
                <col className={styles.colWhen} />
                <col className={styles.colApp} />
                {!connectionId && <col className={styles.colConnection} />}
                <col className={styles.colResult} />
                <col />
              </colgroup>
              <thead>
                <tr>
                  <th>When</th>
                  <th>From</th>
                  {!connectionId && <th>Connection</th>}
                  <th>Result</th>
                  <th>SQL</th>
                </tr>
              </thead>
              <tbody>
                {rows.map(({ e, i }) => {
                  const expanded = open.has(i);
                  return (
                    <Fragment key={`${e.at}-${i}`}>
                      <tr
                        className={cx(
                          styles.logRow,
                          e.status === "error" && styles.notOk,
                          expanded && styles.expandedRow,
                        )}
                        tabIndex={0}
                        aria-expanded={expanded}
                        onClick={() => toggle(i)}
                        onKeyDown={(k) => {
                          if (k.key === "Enter" || k.key === " ") {
                            k.preventDefault();
                            toggle(i);
                          }
                        }}
                      >
                        <td title={when(e.at)}>
                          <span className={styles.whenCell}>
                            <ChevronRight
                              size={12}
                              className={cx(styles.chevron, expanded && styles.chevronOpen)}
                            />
                            {short(e.at)}
                          </span>
                        </td>
                        <td className={styles.ellipsis}>{SOURCE[e.source]}</td>
                        {!connectionId && (
                          <td className={styles.ellipsis}>{name(e.connectionId)}</td>
                        )}
                        <td className={styles.ellipsis}>{result(e)}</td>
                        <td className={styles.sql}>{e.sql}</td>
                      </tr>
                      {expanded && (
                        <tr className={cx(styles.detailRow, e.status === "error" && styles.notOk)}>
                          <td colSpan={columns}>
                            <pre className={styles.fullSql}>{e.sql}</pre>
                            <dl className={styles.facts}>
                              <div>
                                <dt>When</dt>
                                <dd>{when(e.at)}</dd>
                              </div>
                              {e.question && (
                                <div>
                                  <dt>Question</dt>
                                  <dd>{e.question}</dd>
                                </div>
                              )}
                              <div>
                                <dt>Result</dt>
                                <dd>{result(e)}</dd>
                              </div>
                              {e.detail && (
                                <div>
                                  <dt>{e.status === "ok" ? "Sent to the AI" : "Detail"}</dt>
                                  <dd>{e.detail}</dd>
                                </div>
                              )}
                            </dl>
                            {onOpenSql && (
                              <Button
                                size="sm"
                                onClick={() => onOpenSql(e.connectionId, e.database ?? null, e.sql)}
                              >
                                Open in a new tab
                              </Button>
                            )}
                          </td>
                        </tr>
                      )}
                    </Fragment>
                  );
                })}
              </tbody>
            </table>
          </div>
        )
      )}
      {log.length > 0 && !connectionId && (
        <div>
          <Button size="sm" variant="dangerGhost" onClick={clear}>
            Clear the log
          </Button>
        </div>
      )}
      {error && <Alert tone="danger">{error}</Alert>}
    </div>
  );
}
