import { WRITE_CONFIRM, type QueryResult } from "../../api";
import { errorText, isTeamRefusal } from "../../lib/errors";

/** What happened to one statement of a run. */
export interface Outcome {
  sql: string;
  result: QueryResult | null;
  error: string | null;
  /** The person declined to run this statement that may change data. */
  cancelled: boolean;
  /** `error` is a team server refusing by its rules, not the database failing. */
  refused?: boolean;
}

/**
 * Runs `statements` one after another. A statement that may change data waits for
 * `confirm` first, every time. Stops at the first error or declined statement, so later
 * statements never run on top of a failure. `onProgress` gets the outcomes so far.
 */
export async function runStatements(
  statements: string[],
  exec: (sql: string, allowWrite: boolean) => Promise<QueryResult>,
  confirm: (sql: string) => Promise<boolean>,
  onProgress: (outcomes: Outcome[]) => void,
): Promise<Outcome[]> {
  const out: Outcome[] = [];
  const attempt = async (sql: string, allowWrite: boolean): Promise<Outcome> => {
    try {
      return { sql, result: await exec(sql, allowWrite), error: null, cancelled: false };
    } catch (e) {
      return {
        sql,
        result: null,
        error: errorText(e),
        cancelled: false,
        refused: isTeamRefusal(e),
      };
    }
  };
  for (const sql of statements) {
    let outcome = await attempt(sql, false);
    if (outcome.error === WRITE_CONFIRM) {
      outcome = (await confirm(sql))
        ? await attempt(sql, true)
        : { sql, result: null, error: null, cancelled: true };
    }
    out.push(outcome);
    onProgress([...out]);
    if (outcome.error || outcome.cancelled) break;
  }
  return out;
}
