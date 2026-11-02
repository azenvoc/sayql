import { useCallback, useEffect, useRef, useState } from "react";
import { QUERY_CANCELLED, commands, type Dialect, type QuerySource, type Target } from "../../api";
import { useLatest } from "../../hooks/useLatest";
import { splitForRun } from "../../lib/sqlStatements";
import { targetKey } from "../connections";
import { runStatements, type Outcome } from "./runStatements";

export interface QueryRunner {
  /** One per statement of the last run, as far as it got. */
  outcomes: Outcome[];
  /** How many statements the last run had. */
  total: number;
  /** The database error that stopped the last run, if any. */
  error: string | null;
  /** That error is a team server refusing by its rules, which no AI fix can help with. */
  refused: boolean;
  running: boolean;
  /** A data-changing statement waiting for the user's confirmation. */
  pendingWrite: string | null;
  /**
   * Runs `sql`, split into statements the way `dialect` reads it. `context` says where it came
   * from (the Ask box and its question, or the editor), for a team server's audit log.
   */
  run: (sql: string, dialect?: Dialect, context?: QueryContext) => void;
  /** Stops the running statement on the server; later statements of the run don't start. */
  cancel: () => void;
  confirmWrite: () => void;
  cancelWrite: () => void;
}

export interface QueryContext {
  source: QuerySource;
  question?: string;
}

/**
 * Runs SQL on the tab's database, one statement at a time; each statement that changes
 * data waits for confirmation. `onFailure` hears about the error of a single-statement
 * run, e.g. to offer an AI fix (a script is the person's own, so it's left to them).
 */
export function useQueryRunner(
  target: Target | null,
  onFailure?: (sql: string, error: string) => void,
): QueryRunner {
  const failure = useLatest(onFailure);
  const [outcomes, setOutcomes] = useState<Outcome[]>([]);
  const [total, setTotal] = useState(0);
  const [running, setRunning] = useState(false);
  const [pendingWrite, setPendingWrite] = useState<string | null>(null);
  const answer = useRef<((ok: boolean) => void) | null>(null);
  /** Bumped by each run and database change; a stale run's progress is dropped. */
  const runId = useRef(0);
  /** The id of the statement running on the server, for `cancelQuery`. */
  const statementId = useRef<string | null>(null);

  const stopStatement = () => {
    if (statementId.current) void commands.cancelQuery(statementId.current).catch(() => {});
    statementId.current = null;
  };

  const settle = (ok: boolean) => {
    answer.current?.(ok);
    answer.current = null;
    setPendingWrite(null);
  };

  const key = targetKey(target);
  useEffect(() => {
    runId.current++;
    stopStatement();
    settle(false);
    setOutcomes([]);
    setTotal(0);
    setRunning(false);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key]);

  const run = useCallback(
    async (sql: string, dialect?: Dialect, context?: QueryContext) => {
      const statements = splitForRun(sql, dialect);
      if (!target || statements.length === 0) return;
      const id = ++runId.current;
      stopStatement();
      settle(false);
      setOutcomes([]);
      setTotal(statements.length);
      setRunning(true);
      const out = await runStatements(
        statements,
        async (s, allowWrite) => {
          // A cancelled run starts no further statements.
          if (id !== runId.current) throw QUERY_CANCELLED;
          const sid = crypto.randomUUID();
          statementId.current = sid;
          try {
            return await commands.runQuery(
              target,
              s,
              allowWrite,
              sid,
              context?.source,
              context?.question,
            );
          } finally {
            if (statementId.current === sid) statementId.current = null;
          }
        },
        (s) =>
          new Promise<boolean>((resolve) => {
            if (id !== runId.current) return resolve(false);
            answer.current = resolve;
            setPendingWrite(s);
          }),
        (o) => id === runId.current && setOutcomes(o),
      );
      if (id !== runId.current) return;
      setRunning(false);
      const last = out[out.length - 1];
      if (statements.length === 1 && last?.error && last.error !== QUERY_CANCELLED && !last.refused)
        failure.current?.(last.sql, last.error);
    },
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [target, failure],
  );

  return {
    outcomes,
    total,
    error: outcomes.find((o) => o.error)?.error ?? null,
    refused: !!outcomes.find((o) => o.error)?.refused,
    running,
    pendingWrite,
    run,
    cancel: () => {
      // Keep the outcomes so far; the stopped statement reports QUERY_CANCELLED.
      stopStatement();
      settle(false);
    },
    confirmWrite: () => settle(true),
    cancelWrite: () => settle(false),
  };
}
