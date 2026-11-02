import { useEffect, useState, type ReactNode } from "react";
import { Ban, Check, CheckCircle2, Copy, Table2 } from "lucide-react";
import { QUERY_CANCELLED } from "../../api";
import { Alert, Badge, Button, EmptyState, Panel, Spinner } from "../../components/ui";
import { cx } from "../../lib/cx";
import type { Outcome } from "../query";
import { DataGrid } from "./DataGrid";
import { plural, statementLabel, toCsv } from "./format";
import styles from "./ResultsPanel.module.css";

export interface ResultsPanelProps {
  /** One per statement of the last run, as far as it got. */
  outcomes: Outcome[];
  /** How many statements the last run had. */
  total: number;
  running: boolean;
  /** Shown under a database error, e.g. advice to rephrase. */
  errorHint?: string;
  /** Shown next to a database error, e.g. a Fix with AI button. */
  errorAction?: ReactNode;
  /** A short note on what ran, e.g. that a selection was widened to whole statements. */
  note?: string;
}

/** The results of the last run; a tab per statement when it had several. */
export function ResultsPanel({
  outcomes,
  total,
  running,
  errorHint,
  errorAction,
  note,
}: ResultsPanelProps) {
  const [copied, setCopied] = useState(false);
  // Shows the statement that stopped the run, else the first, until the person picks a tab.
  const [picked, setPicked] = useState<number | null>(null);
  const fresh = outcomes.length === 0;
  useEffect(() => {
    if (fresh) setPicked(null);
  }, [fresh]);
  const stopped = outcomes.findIndex((o) => o.error || o.cancelled);
  const index = Math.min(picked ?? (stopped >= 0 ? stopped : 0), Math.max(outcomes.length - 1, 0));
  const shown: Outcome | undefined = outcomes[index];
  const result = shown?.result ?? null;
  const hasGrid = !!result && result.columns.length > 0;
  const many = total > 1;

  const copyCsv = async () => {
    if (!result) return;
    await navigator.clipboard.writeText(toCsv(result));
    setCopied(true);
    window.setTimeout(() => setCopied(false), 1500);
  };

  return (
    <Panel
      title="Results"
      icon={Table2}
      subtitle={
        <>
          {running && (
            <span className={styles.meta}>
              <Spinner size={12} />{" "}
              {many ? `Running ${Math.min(outcomes.length + 1, total)} of ${total}…` : "Running…"}
            </span>
          )}
          {!running && result && (
            <span className={styles.meta}>
              {hasGrid
                ? plural(result.rows.length, "row")
                : plural(result.rowsAffected ?? 0, "row") + " affected"}
              {" · "}
              {result.elapsedMs} ms
              {result.truncated && <Badge tone="warning">first 5,000 rows</Badge>}
              {!!result.withheld?.length && (
                <Badge tone="warning" title={`Withheld: ${result.withheld.join(", ")}`}>
                  {plural(result.withheld.length, "column")} withheld by your team admin
                </Badge>
              )}
            </span>
          )}
          {!running && many && stopped >= 0 && (
            <Badge tone="warning">
              Stopped at {stopped + 1} of {total}
            </Badge>
          )}
          {note && (
            <Badge
              tone="accent"
              title="Part of a statement was selected, so the whole statement ran. It's highlighted in the editor."
            >
              {note}
            </Badge>
          )}
        </>
      }
      actions={
        hasGrid && (
          <Button size="sm" variant="ghost" icon={copied ? Check : Copy} onClick={copyCsv}>
            {copied ? "Copied" : "Copy as CSV"}
          </Button>
        )
      }
    >
      {many && outcomes.length > 0 && (
        <div className={styles.tabs} role="tablist" aria-label="Statement results">
          {outcomes.map((o, i) => (
            <button
              key={i}
              type="button"
              role="tab"
              aria-selected={i === index}
              title={o.sql}
              className={cx(
                styles.tab,
                i === index && styles.active,
                (o.error || o.cancelled) && styles.failed,
              )}
              onClick={() => setPicked(i)}
            >
              <span className={styles.number}>{i + 1}</span> {statementLabel(o.sql)}
            </button>
          ))}
        </div>
      )}
      {shown?.error === QUERY_CANCELLED ? (
        <EmptyState icon={Ban} title="Query cancelled">
          SayQL stopped it on the server
          {index + 1 < total ? ", and the statements after it didn't run" : ""}.
        </EmptyState>
      ) : shown?.error ? (
        <div className={styles.error}>
          <Alert tone="danger" actions={shown.refused ? undefined : errorAction}>
            <strong>
              {shown.refused
                ? "Your team's SayQL server refused this"
                : many
                  ? `Statement ${index + 1} returned an error`
                  : "The database returned an error"}
            </strong>
            <pre className={styles.errorSql}>{shown.error}</pre>
          </Alert>
          {many && <pre className={styles.statement}>{shown.sql}</pre>}
          {errorHint && <span className={styles.meta}>{errorHint}</span>}
        </div>
      ) : shown?.cancelled ? (
        <EmptyState icon={Ban} title="Not run">
          You cancelled this statement
          {index + 1 < total ? ", so the ones after it didn't run either" : ""}.
        </EmptyState>
      ) : hasGrid ? (
        <DataGrid key={index} result={result} />
      ) : result ? (
        <EmptyState icon={CheckCircle2} title="Statement executed">
          {plural(result.rowsAffected ?? 0, "row")} affected.
        </EmptyState>
      ) : running ? null : (
        <EmptyState icon={Table2} title="No results yet">
          Ask a question or write SQL, then press Run.
        </EmptyState>
      )}
    </Panel>
  );
}
