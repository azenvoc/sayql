import { Fragment, useCallback, useEffect, useState } from "react";
import { ChevronRight, RefreshCw } from "lucide-react";
import { commands, type McpActivity } from "../../api";
import { Alert, Button, SegmentedControl, Select } from "../../components/ui";
import { cx } from "../../lib/cx";
import { errorText } from "../../lib/errors";
import { APP_OPTIONS } from "../agent";
import { when } from "./AppsList";
import styles from "./ClaudeIntegration.module.css";

const STATUS: Record<McpActivity["status"], string> = {
  ok: "OK",
  refused: "Refused",
  error: "Error",
};
/** How much of the log to read; the backend keeps more and caps a read at 1000. */
const LOG_LIMIT = 200;
const ALL = "";

type ResultFilter = "all" | "refused";

/** Short enough for a table cell: the time today, the date otherwise. */
function short(ms: number): string {
  const d = new Date(ms);
  const today = d.toDateString() === new Date().toDateString();
  return today
    ? d.toLocaleTimeString(undefined, { timeStyle: "short" })
    : d.toLocaleString(undefined, {
        month: "short",
        day: "numeric",
        hour: "numeric",
        minute: "2-digit",
      });
}

/** What the entry did, in one line. */
function summary(e: McpActivity): string {
  if (e.tool === "export_csv") return `CSV export: ${e.sql}`;
  if (e.tool === "suggest_note") {
    // The MCP server logs what kind of suggestion it was ("suggested replacing a note"), never the note.
    if (e.status === "ok" && e.detail?.startsWith("suggested "))
      return `Suggested ${e.detail.slice("suggested ".length)}`;
    return `Suggested a note${e.status === "ok" && e.detail ? `: ${e.detail}` : ""}`;
  }
  if (e.tool === "audit")
    return e.status === "ok" && !e.connection
      ? "Fetched the audit prompt (connection not named)"
      : "Fetched the audit prompt";
  return e.sql || e.tool;
}

function result(e: McpActivity): string {
  return `${STATUS[e.status]}${e.status === "ok" && e.rows != null ? ` · ${e.rows} ${e.rows === 1 ? "row" : "rows"}` : ""}`;
}

/** The detail beside the result; a suggested note's text is already the summary. */
const detailOf = (e: McpActivity) =>
  e.detail && !((e.tool === "suggest_note" || e.tool === "audit") && e.status === "ok")
    ? e.detail
    : null;

/** What AI apps ran through SayQL over MCP: filters, a row to open for the full SQL, and clearing the log. */
export function AgentActivity() {
  const [log, setLog] = useState<McpActivity[]>([]);
  const [loaded, setLoaded] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [app, setApp] = useState(ALL);
  const [connection, setConnection] = useState(ALL);
  const [shown, setShown] = useState<ResultFilter>("all");
  const [open, setOpen] = useState<Set<number>>(new Set());

  const load = useCallback(() => {
    setError(null);
    commands.mcpActivity(LOG_LIMIT).then(
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
  }, []);
  useEffect(load, [load]);

  const clear = () => {
    if (!window.confirm("Delete the activity log? Blocked apps stay blocked.")) return;
    commands.clearMcpActivity().then(load, (e) => setError(errorText(e)));
  };
  const toggle = (i: number) =>
    setOpen((s) => {
      const next = new Set(s);
      if (!next.delete(i)) next.add(i);
      return next;
    });

  const apps = [...new Set(log.map((e) => e.app))].sort();
  const connections = [...new Set(log.map((e) => e.connection).filter(Boolean))].sort();
  const rows = log
    .map((e, i) => ({ e, i }))
    .filter(
      ({ e }) =>
        (app === ALL || e.app === app) &&
        (connection === ALL || e.connection === connection) &&
        (shown === "all" || e.status === "refused"),
    );

  return (
    <div className={styles.activity}>
      <p className={styles.intro}>
        What AI apps ran through SayQL on this computer: the SQL and how many rows it returned, not
        the rows themselves. AI apps only: Investigate and your own queries aren&apos;t in this log.
        Click a row for the full SQL.
      </p>
      <div className={styles.filters}>
        <Select
          aria-label="App"
          className={styles.filter}
          value={app}
          options={[
            { value: ALL, label: "All apps" },
            ...apps.map((a) => ({ value: a, label: a })),
          ]}
          onChange={setApp}
        />
        <Select
          aria-label="Connection"
          className={styles.filter}
          value={connection}
          options={[
            { value: ALL, label: "All connections" },
            ...connections.map((c) => ({ value: c, label: c })),
          ]}
          onChange={setConnection}
        />
        <div className={styles.resultFilter}>
          <SegmentedControl<ResultFilter>
            ariaLabel="Result"
            value={shown}
            options={[
              { value: "all", label: "All" },
              { value: "refused", label: "Refused only" },
            ]}
            onChange={setShown}
          />
        </div>
        <span className={styles.spacer} />
        <Button size="sm" variant="ghost" icon={RefreshCw} onClick={load}>
          Refresh
        </Button>
      </div>
      {loaded && log.length === 0 ? (
        <p className={styles.empty}>
          Nothing yet. Queries from Claude, Cursor and other MCP apps show up here.
        </p>
      ) : loaded && rows.length === 0 ? (
        <p className={styles.empty}>Nothing in the log matches these filters.</p>
      ) : (
        rows.length > 0 && (
          <div className={styles.tableWrap}>
            <table className={cx(styles.table, styles.logTable)}>
              <colgroup>
                <col className={styles.colWhen} />
                <col className={styles.colApp} />
                <col className={styles.colConnection} />
                <col className={styles.colResult} />
                <col />
              </colgroup>
              <thead>
                <tr>
                  <th>When</th>
                  <th>App</th>
                  <th>Connection</th>
                  <th>Result</th>
                  <th>SQL</th>
                </tr>
              </thead>
              <tbody>
                {rows.map(({ e, i }) => {
                  const expanded = open.has(i);
                  const detail = detailOf(e);
                  return (
                    <Fragment key={`${e.at}-${i}`}>
                      <tr
                        className={cx(
                          styles.logRow,
                          e.status !== "ok" && styles.notOk,
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
                        <td className={styles.ellipsis}>{e.app}</td>
                        <td className={styles.ellipsis}>{e.connection || "—"}</td>
                        <td className={styles.ellipsis}>
                          {result(e)}
                          {detail && <span className={styles.detail}> · {detail}</span>}
                        </td>
                        <td className={styles.sql}>{summary(e)}</td>
                      </tr>
                      {expanded && (
                        <tr className={cx(styles.detailRow, e.status !== "ok" && styles.notOk)}>
                          <td colSpan={5}>
                            <pre className={styles.fullSql}>
                              {e.tool === "suggest_note" || e.tool === "audit"
                                ? summary(e)
                                : e.sql || e.tool}
                            </pre>
                            <dl className={styles.facts}>
                              <div>
                                <dt>When</dt>
                                <dd>{when(e.at)}</dd>
                              </div>
                              <div>
                                <dt>Tool</dt>
                                <dd className={styles.mono}>{e.tool}</dd>
                              </div>
                              <div>
                                <dt>Result</dt>
                                <dd>{result(e)}</dd>
                              </div>
                              {detail && (
                                <div>
                                  <dt>Detail</dt>
                                  <dd>{detail}</dd>
                                </div>
                              )}
                              {e.maySee && (
                                <div>
                                  <dt>App could see</dt>
                                  <dd>
                                    {APP_OPTIONS.find((r) => r.value === e.maySee)?.label ??
                                      e.maySee}
                                  </dd>
                                </div>
                              )}
                            </dl>
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
      {log.length > 0 && (
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
