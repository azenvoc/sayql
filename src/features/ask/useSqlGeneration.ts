import { useCallback, useEffect, useRef, useState } from "react";
import {
  commands,
  subscribeToAi,
  type GenerationTask,
  type PreviousQuery,
  type Schema,
  type Skill,
  type Target,
} from "../../api";
import { useLatest } from "../../hooks/useLatest";
import { errorText } from "../../lib/errors";
import { targetKey } from "../connections";
import { fastPath } from "./fastPath";
import { actionTask, menuQuery, planRequest, type ActionCommand, type Plan } from "./slashCommands";
import { cleanSql } from "./sqlText";

/** Wait this long after the last keystroke before asking the AI. */
const DEBOUNCE_MS = 250;

export type TaskKind = GenerationTask["kind"];

export type GenerationStatus =
  | { kind: "idle" }
  | { kind: "instant"; source: "rules" | "cache" }
  | { kind: "writing"; task: TaskKind }
  | { kind: "done"; task: TaskKind; firstTokenMs: number | null; totalMs: number }
  | { kind: "error"; message: string; fixInSettings: boolean };

export interface SqlGenerationOptions {
  /** Fixed for the life of the hook: one hook per editor tab. */
  target: Target | null;
  /** Cancellation scope on the backend; the tab id. */
  slot: string;
  schema: Schema | null;
  /** The provider and model that draft while typing; part of the answer cache key. */
  typingModel: string;
  aiReady: boolean;
  initialQuestion?: string;
  onQuestionChange?: (question: string) => void;
  /** The query last run in the tab; follow-up drafts are asked to edit it. */
  previous: PreviousQuery | null;
  /** Tables the person picked; the AI builds on them. */
  focusTables: string[];
  /** Saved skills, for "/slug" in the question. */
  skills: Skill[];
  /** The SQL in the editor: what /followup edits when nothing was run yet. */
  sql: string;
  /** Receives the SQL as it streams in, and the final SQL. */
  onSql: (sql: string) => void;
  /** Receives finished SQL that should run now: a run requested while writing, or a fix. */
  onRunRequested: (sql: string, task: TaskKind) => void;
}

const normalize = (q: string) => q.trim().toLowerCase().replace(/\s+/g, " ");

// Shared by every tab: request ids must be unique across tabs because AI events are
// broadcast, and the same question on the same database is instant in any tab.
let lastRequestId = 0;
const answers = new Map<string, string>();

/**
 * Turns the question into SQL:
 * - while typing, instant local answers, a per-question cache, and otherwise a debounced
 *   streamed draft from the fast model (building on the previous query for follow-ups);
 * - on request, `fix` (a query the database rejected) and `improve` with the expert model,
 *   and `explain` (plain English, into `explanation` instead of the editor);
 * - slash commands and skills in the question change the request (see `planRequest`).
 * Each new request supersedes the previous one in the same tab.
 */
export function useSqlGeneration(options: SqlGenerationOptions) {
  const [question, setQuestionState] = useState(options.initialQuestion ?? "");
  const [status, setStatus] = useState<GenerationStatus>({ kind: "idle" });
  /** The latest /explain answer, as it streams in. */
  const [explanation, setExplanation] = useState<string | null>(null);

  const opts = useLatest(options);
  const statusRef = useLatest(status);
  /** The request this tab is waiting for; 0 when none. */
  const reqId = useRef(0);
  const buffer = useRef("");
  const firstTokenMs = useRef<number | null>(null);
  /** `text` is the whole question, with any "/command"; `question` is what's sent. */
  const inFlight = useRef<{
    text: string;
    question: string;
    task: GenerationTask;
    skill: string | null;
  } | null>(null);
  const runWhenReady = useRef(false);
  const debounce = useRef<number | undefined>(undefined);

  const plan = (text: string): Plan =>
    planRequest(text, opts.current.skills, {
      previous: opts.current.previous,
      sql: opts.current.sql,
    });

  // The previous query is part of the key: "remove the middle name" means something else after each query.
  // So is a skill's instruction, which can be edited under the same name.
  const cacheKey = (text: string) => {
    const p = plan(text);
    const previous = p.kind === "draft" ? (p.task.previous?.sql ?? "") : "";
    const skill = opts.current.skills.find((s) => s.slug === p.skill)?.instruction ?? "";
    return `${targetKey(opts.current.target)}|${opts.current.typingModel}|${previous}|${skill}|${opts.current.focusTables.join(",")}|${normalize(text)}`;
  };

  const clearDebounce = () => {
    window.clearTimeout(debounce.current);
    debounce.current = undefined;
  };

  const fail = useCallback((message: string) => {
    runWhenReady.current = false;
    setStatus({ kind: "error", message, fixInSettings: message.includes("Settings") });
  }, []);

  /** Drops any request in flight; its events will be ignored. */
  const cancel = useCallback(() => {
    clearDebounce();
    reqId.current = 0;
    void commands.cancelGeneration(opts.current.slot);
  }, [opts]);

  const start = useCallback(
    (text: string, q: string, task: GenerationTask, skill: string | null) => {
      const { target, aiReady, slot } = opts.current;
      clearDebounce();
      if (!target) return;
      if (!aiReady) {
        fail("Set up an AI provider in Settings to turn English into SQL.");
        return;
      }
      const id = ++lastRequestId;
      reqId.current = id;
      buffer.current = "";
      firstTokenMs.current = null;
      inFlight.current = { text, question: q, task, skill };
      setExplanation(null);
      setStatus({ kind: "writing", task: task.kind });
      commands
        .generateSql(target, q, id, slot, task, opts.current.focusTables, skill)
        .catch((e) => id === reqId.current && fail(errorText(e)));
    },
    [fail, opts],
  );

  const draft = useCallback(
    (text: string) => {
      const p = plan(text);
      if (p.kind === "draft") start(text, p.question, p.task, p.skill);
    },
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [opts, start],
  );

  useEffect(() => {
    const unsubscribe = subscribeToAi({
      delta: (e) => {
        if (e.reqId !== reqId.current) return;
        if (e.firstTokenMs != null) firstTokenMs.current = e.firstTokenMs;
        buffer.current += e.text;
        if (inFlight.current?.task.kind === "explain") setExplanation(buffer.current);
        else opts.current.onSql(cleanSql(buffer.current));
      },
      reset: (id) => {
        if (id === reqId.current) buffer.current = "";
      },
      done: (e) => {
        if (e.reqId !== reqId.current || !inFlight.current) return;
        if (e.error) return fail(e.error);
        const { text, task } = inFlight.current;
        if (task.kind === "explain") {
          setExplanation(buffer.current.trim());
          setStatus({
            kind: "done",
            task: task.kind,
            firstTokenMs: firstTokenMs.current,
            totalMs: e.totalMs,
          });
          return;
        }
        const sql = cleanSql(buffer.current).trimEnd();
        opts.current.onSql(sql);
        if (task.kind === "draft") answers.set(cacheKey(text), sql);
        // Still typing: a newer draft is about to start, so stay in "writing".
        if (debounce.current !== undefined) return;
        setStatus({
          kind: "done",
          task: task.kind,
          firstTokenMs: firstTokenMs.current,
          totalMs: e.totalMs,
        });
        if (runWhenReady.current) {
          runWhenReady.current = false;
          opts.current.onRunRequested(sql, task.kind);
        }
      },
    });
    return () => void unsubscribe.then((off) => off());
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const setQuestion = useCallback(
    (text: string) => {
      setQuestionState(text);
      opts.current.onQuestionChange?.(text);
      runWhenReady.current = false;
      clearDebounce();
      const p = plan(text);
      // Nothing to draft: an empty question, a command still being picked from the menu, or
      // explain, fix and improve, which wait for ↵. Stop any draft, but not a fix or explanation.
      if (
        p.kind === "action" ||
        p.kind === "agent" ||
        !p.question.trim() ||
        menuQuery(text) !== null
      ) {
        const s = statusRef.current;
        if (s.kind === "writing" && s.task !== "draft") return;
        cancel();
        setStatus({ kind: "idle" });
        return;
      }
      const instant = p.instant ? fastPath(p.question, opts.current.schema) : null;
      const cached = answers.get(cacheKey(text));
      if (instant ?? cached) {
        cancel();
        opts.current.onSql((instant ?? cached)!);
        setStatus({ kind: "instant", source: instant ? "rules" : "cache" });
        return;
      }
      setStatus({ kind: "writing", task: "draft" });
      debounce.current = window.setTimeout(() => draft(text), DEBOUNCE_MS);
    },
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [cancel, draft],
  );

  /**
   * Call when the person asks to run. Returns true if the SQL is still being written —
   * it will then be passed to `onRunRequested` once finished.
   */
  const requestRun = useCallback((): boolean => {
    if (statusRef.current.kind !== "writing") return false;
    runWhenReady.current = true;
    if (debounce.current !== undefined) draft(question); // skip the remaining debounce wait
    return true;
  }, [question, draft, statusRef]);

  /**
   * Works on `sql` with the question (without its command) as the request, keeping a skill:
   * `fix` corrects it given the database's error (or none) and runs the fix; `improve` reviews
   * it with the expert model; `explain` describes it in `explanation`.
   */
  const act = useCallback(
    (command: ActionCommand, sql: string, error: string | null = null) => {
      if (!sql.trim()) return fail("There's no query yet. Ask a question or write SQL first.");
      const p = plan(question);
      runWhenReady.current = command === "fix";
      start(question, p.question, actionTask(command, sql, error), p.skill);
    },
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [fail, question, start],
  );

  const fix = useCallback((sql: string, error: string) => act("fix", sql, error), [act]);
  const improve = useCallback((sql: string) => act("improve", sql), [act]);

  /** Repeats the last request (after an error), or drafts the current question. */
  const retry = useCallback(() => {
    const last = inFlight.current;
    if (last && last.task.kind !== "draft") start(last.text, last.question, last.task, last.skill);
    else if (question.trim()) draft(question);
  }, [draft, question, start]);

  // An API key was just added: retry the request that was waiting for it.
  useEffect(() => {
    if (options.aiReady && statusRef.current.kind === "error" && statusRef.current.fixInSettings)
      retry();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [options.aiReady]);

  return {
    question,
    setQuestion,
    status,
    requestRun,
    act,
    fix,
    improve,
    retry,
    explanation,
    dismissExplanation: () => setExplanation(null),
  };
}
