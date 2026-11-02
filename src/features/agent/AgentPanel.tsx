import { useEffect, useState, type ReactNode } from "react";
import {
  AlertTriangle,
  Bot,
  Check,
  ChevronRight,
  CircleStop,
  Database,
  FileCode2,
  MessageSquareReply,
  Pencil,
  Play,
  RotateCcw,
  Save,
  Send,
  FastForward,
  SkipForward,
  StickyNote,
  X,
  XCircle,
} from "lucide-react";
import type { AgentAnswer, AgentStep } from "../../api";
import { Alert, Badge, Button, EmptyState, Panel, Spinner, TextInput } from "../../components/ui";
import { cx } from "../../lib/cx";
import { isActive, type AgentRunState } from "./agentRun";
import styles from "./AgentPanel.module.css";
import { useNoteSuggestions } from "./useNoteSuggestions";

export interface AgentPanelProps {
  run: AgentRunState;
  /** The step whose rows and SQL are shown. */
  selected: number | null;
  onSelect: (stepId: number) => void;
  onGo: (plan: string[] | null) => void;
  onSend: () => void;
  /** Turns on Auto mode, which also carries on from the current pause. */
  onAuto: () => void;
  onSkip: () => void;
  onAnswer: (text: string) => void;
  onStop: () => void;
  /** Puts SQL in a new tab as a normal query. */
  onOpenQuery: (sql: string) => void;
  /** Saves the evidence queries as a script. */
  onSaveScript: (sqls: string[]) => void;
  /** Runs the same queries again on fresh data, without the AI. */
  onRerun: (sqls: string[]) => void;
  /** Puts the Ask box in agent mode for a follow-up question. */
  onFollowUp: () => void;
  /** Saves a suggested note; rejects with a message the panel shows. */
  onSaveNote: (text: string) => Promise<void>;
  /** Continues the run from a step whose SQL the person changed. */
  onSteer: (stepId: number, sql: string) => void;
}

/** Agent mode's view of a run: plan, steps, pauses, and the cited answer. */
export function AgentPanel(p: AgentPanelProps) {
  const { run } = p;
  const active = isActive(run);
  const queries = run.steps.filter((s) => s.tool === "run_query" && s.status === "done" && s.sql);
  const cited = new Set((run.answer?.sentences ?? []).flatMap((s) => s.steps));
  const evidence = queries.filter((s) => cited.size === 0 || cited.has(s.id)).map((s) => s.sql!);

  return (
    <Panel
      title="Investigation"
      icon={Bot}
      subtitle={<Meter run={run} />}
      actions={
        active && (
          <Button size="sm" variant="ghost" icon={CircleStop} onClick={p.onStop}>
            Stop
          </Button>
        )
      }
    >
      <div className={styles.body}>
        {run.phase === "idle" ? (
          <EmptyState icon={Bot} title="No investigation yet">
            Switch the Ask box to <b>Investigate</b> and ask a question. SayQL looks around in a few
            read-only steps and answers with the queries it ran.
          </EmptyState>
        ) : (
          <>
            <p className={styles.question}>{run.question}</p>
            {run.plan && <PlanCard run={run} onGo={p.onGo} onAuto={p.onAuto} />}
            <ol className={styles.steps}>
              {run.steps.map((s) => (
                <StepRow
                  key={s.id}
                  step={s}
                  selected={p.selected === s.id}
                  onSelect={() => p.onSelect(s.id)}
                  onSteer={
                    !active && s.tool === "run_query" && s.sql
                      ? (sql) => p.onSteer(s.id, sql)
                      : undefined
                  }
                />
              ))}
            </ol>
            {run.phase === "starting" ||
            (run.phase === "running" && !run.steps.some((s) => s.status === "running")) ? (
              <span className={styles.working}>
                <Spinner size={13} /> Thinking…
              </span>
            ) : null}
            {run.pending?.kind === "send" && (
              <div className={styles.card} role="group" aria-label="Send to the AI?">
                <strong>Send this to the AI?</strong>
                <pre className={styles.payload}>{run.pending.payload}</pre>
                <div className={styles.buttons}>
                  <Button size="sm" variant="primary" icon={Send} onClick={p.onSend}>
                    Send
                  </Button>
                  <Button size="sm" icon={SkipForward} onClick={p.onSkip}>
                    Skip (send the row count only)
                  </Button>
                  <Button
                    size="sm"
                    variant="ghost"
                    icon={FastForward}
                    onClick={() => {
                      p.onSend();
                      p.onAuto();
                    }}
                    title="Send this and the rest without asking (Auto mode, for every run until you turn it off in the Ask box)"
                  >
                    Send all from now on
                  </Button>
                </div>
              </div>
            )}
            {run.pending?.kind === "question" && (
              <QuestionCard
                question={run.pending.question}
                choices={run.pending.choices}
                onAnswer={p.onAnswer}
              />
            )}
            {run.phase === "error" && run.error && (
              <Alert tone="danger">
                <strong>The run stopped</strong>
                <span className={styles.errorText}>{run.error}</span>
              </Alert>
            )}
            {run.phase === "stopped" && (
              <Alert tone="warning">Stopped. The steps so far are above.</Alert>
            )}
            {run.answer && (
              <AnswerCard
                answer={run.answer}
                onStep={p.onSelect}
                onOpenQuery={p.onOpenQuery}
                onSaveNote={p.onSaveNote}
                actions={
                  <>
                    {evidence.length > 0 && (
                      <>
                        <Button
                          size="sm"
                          icon={FileCode2}
                          onClick={() => p.onOpenQuery(evidence[evidence.length - 1])}
                        >
                          Open as query
                        </Button>
                        <Button
                          size="sm"
                          variant="ghost"
                          icon={Save}
                          onClick={() => p.onSaveScript(evidence)}
                        >
                          Save as script
                        </Button>
                        <Button
                          size="sm"
                          variant="ghost"
                          icon={RotateCcw}
                          onClick={() => p.onRerun(evidence)}
                        >
                          Rerun
                        </Button>
                      </>
                    )}
                    <Button
                      size="sm"
                      variant="ghost"
                      icon={MessageSquareReply}
                      onClick={p.onFollowUp}
                    >
                      Ask a follow-up
                    </Button>
                  </>
                }
              />
            )}
          </>
        )}
      </div>
    </Panel>
  );
}

function Meter({ run }: { run: AgentRunState }) {
  const g = run.progress;
  if (!g) return null;
  const tokens = g.usage.input + g.usage.output;
  return (
    <span
      className={styles.meter}
      title={`${g.usage.cached.toLocaleString()} prompt tokens read from cache`}
    >
      {isActive(run) && <Spinner size={12} />} step {g.turns} of {g.maxTurns} · {g.queries} of{" "}
      {g.maxQueries} queries · {tokens.toLocaleString()} tokens · {Math.round(g.elapsedMs / 1000)} s
    </span>
  );
}

function PlanCard({
  run,
  onGo,
  onAuto,
}: {
  run: AgentRunState;
  onGo: (plan: string[] | null) => void;
  onAuto: () => void;
}) {
  const waiting = run.pending?.kind === "plan";
  const [text, setText] = useState((run.plan ?? []).join("\n"));
  useEffect(() => setText((run.plan ?? []).join("\n")), [run.plan]);
  const lines = text
    .split("\n")
    .map((l) => l.trim())
    .filter(Boolean);
  const go = () => onGo(lines.join("\n") === (run.plan ?? []).join("\n") ? null : lines);
  if (!waiting) {
    return (
      <details className={styles.plan}>
        <summary>Plan</summary>
        <ol>
          {run.plan?.map((s, i) => (
            <li key={i}>{s}</li>
          ))}
        </ol>
      </details>
    );
  }
  return (
    <div className={styles.card} role="group" aria-label="Plan">
      <strong>Plan</strong>
      <span className={styles.muted}>Change any line, then press Go. One step per line.</span>
      <textarea
        className={styles.planText}
        value={text}
        rows={Math.max(3, lines.length + 1)}
        onChange={(e) => setText(e.target.value)}
        aria-label="Plan"
      />
      <div className={styles.buttons}>
        <Button size="sm" variant="primary" icon={Play} onClick={go}>
          Go
        </Button>
        <Button
          size="sm"
          variant="ghost"
          icon={FastForward}
          onClick={() => {
            go();
            onAuto();
          }}
          title="Go now, and don't stop for plans or sends again (Auto mode, until you turn it off in the Ask box)"
        >
          Go, and don't ask again
        </Button>
      </div>
    </div>
  );
}

function StepRow({
  step,
  selected,
  onSelect,
  onSteer,
}: {
  step: AgentStep;
  selected: boolean;
  onSelect: () => void;
  /** Present when the run has ended: change this step's SQL and continue from it. */
  onSteer?: (sql: string) => void;
}) {
  const [open, setOpen] = useState(false);
  const [editing, setEditing] = useState<string | null>(null);
  const icon =
    step.status === "running" ? (
      <Spinner size={12} />
    ) : step.status === "error" ? (
      <XCircle size={13} className={styles.bad} />
    ) : (
      <Check size={13} className={styles.good} />
    );
  return (
    <li className={cx(styles.step, selected && styles.selected)}>
      <div className={styles.stepHead}>
        <button
          type="button"
          className={styles.expand}
          aria-expanded={open}
          aria-label={open ? "Hide details" : "Show details"}
          onClick={() => setOpen(!open)}
        >
          <ChevronRight size={13} className={cx(open && styles.rotated)} />
        </button>
        <span className={styles.number}>{step.id}</span>
        {icon}
        <button
          type="button"
          className={styles.stepText}
          onClick={onSelect}
          title={step.sql ? "Show this step's SQL and rows" : undefined}
        >
          {step.text}
        </button>
        {step.rowCount !== null && (
          <span className={styles.muted}>
            {step.rowCount.toLocaleString()} {step.rowCount === 1 ? "row" : "rows"}
            {step.elapsedMs !== null && ` · ${step.elapsedMs} ms`}
          </span>
        )}
        {step.sentSummary && (
          <Badge tone="ai" title="What went to the AI from this step">
            {step.sentSummary}
          </Badge>
        )}
        {step.checks.length > 0 && (
          <Badge tone="warning" title={step.checks.join("\n")}>
            check
          </Badge>
        )}
      </div>
      {open && (
        <div className={styles.details}>
          {editing !== null ? (
            <>
              <textarea
                className={styles.planText}
                rows={4}
                value={editing}
                onChange={(e) => setEditing(e.target.value)}
                aria-label={`Step ${step.id} SQL`}
              />
              <div className={styles.buttons}>
                <Button
                  size="sm"
                  variant="primary"
                  icon={Play}
                  disabled={!editing.trim()}
                  onClick={() => (onSteer?.(editing), setEditing(null))}
                >
                  Continue from here
                </Button>
                <Button size="sm" variant="ghost" onClick={() => setEditing(null)}>
                  Cancel
                </Button>
              </div>
            </>
          ) : (
            step.sql && <pre className={styles.sql}>{step.sql}</pre>
          )}
          {onSteer && editing === null && (
            <div>
              <Button
                size="sm"
                variant="ghost"
                icon={Pencil}
                onClick={() => setEditing(step.sql ?? "")}
              >
                Edit and continue
              </Button>
            </div>
          )}
          {step.checks.map((c, i) => (
            <span key={i} className={styles.check}>
              <AlertTriangle size={12} /> {c}
            </span>
          ))}
          {step.error && <span className={styles.errorText}>{step.error}</span>}
          {step.sent !== null && (
            <details className={styles.sent}>
              <summary>Sent to AI</summary>
              <pre className={styles.payload}>{step.sent}</pre>
            </details>
          )}
        </div>
      )}
    </li>
  );
}

function QuestionCard({
  question,
  choices,
  onAnswer,
}: {
  question: string;
  choices: string[];
  onAnswer: (text: string) => void;
}) {
  const [text, setText] = useState("");
  return (
    <div className={cx(styles.card, styles.ask)} role="group" aria-label="SayQL asks">
      <strong>{question}</strong>
      <div className={styles.buttons}>
        {choices.map((c) => (
          <Button key={c} size="sm" onClick={() => onAnswer(c)}>
            {c}
          </Button>
        ))}
      </div>
      <form
        className={styles.buttons}
        onSubmit={(e) => {
          e.preventDefault();
          if (text.trim()) onAnswer(text.trim());
        }}
      >
        <TextInput
          value={text}
          onChange={(e) => setText(e.target.value)}
          placeholder="Or type an answer"
          aria-label="Your answer"
        />
        <Button size="sm" type="submit" disabled={!text.trim()}>
          Answer
        </Button>
      </form>
    </div>
  );
}

/** The sentence with numbers SayQL couldn't find in any result marked. */
function Marked({ text, unverified }: { text: string; unverified: string[] }) {
  if (unverified.length === 0) return <>{text}</>;
  const pattern = new RegExp(
    `(${unverified.map((u) => u.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")).join("|")})`,
    "g",
  );
  return (
    <>
      {text.split(pattern).map((part, i) =>
        unverified.includes(part) ? (
          <mark
            key={i}
            className={styles.unverified}
            title="Not verified: no step's result contains this number"
          >
            {part}
          </mark>
        ) : (
          part
        ),
      )}
    </>
  );
}

function AnswerCard({
  answer,
  onStep,
  onOpenQuery,
  onSaveNote,
  actions,
}: {
  answer: AgentAnswer;
  onStep: (id: number) => void;
  onOpenQuery: (sql: string) => void;
  onSaveNote: (text: string) => Promise<void>;
  actions: ReactNode;
}) {
  const tone =
    answer.confidence === "high" ? "success" : answer.confidence === "low" ? "warning" : "neutral";
  return (
    <section className={cx(styles.card, styles.answer)} aria-label="Answer">
      <div className={styles.answerHead}>
        <strong>Answer</strong>
        <Badge tone={tone}>{answer.confidence} confidence</Badge>
        {answer.cannotAnswer && <Badge tone="warning">the data can't answer this</Badge>}
        {answer.unverified.length > 0 && (
          <Badge tone="warning">{answer.unverified.length} not verified</Badge>
        )}
      </div>
      <p className={styles.answerText}>
        {answer.sentences.map((s, i) => (
          <span key={i}>
            <Marked text={s.text} unverified={s.unverified} />
            {s.steps.map((id) => (
              <button
                key={id}
                type="button"
                className={styles.cite}
                onClick={() => onStep(id)}
                title={`Show step ${id}`}
              >
                {id}
              </button>
            ))}{" "}
          </span>
        ))}
      </p>
      {answer.assumptions.length > 0 && (
        <ul className={styles.assumptions}>
          {answer.assumptions.map((a, i) => (
            <li key={i}>Assumed: {a}</li>
          ))}
        </ul>
      )}
      {answer.proposedSql && (
        <div className={styles.proposed}>
          <span>
            <Badge tone="warning">not run</Badge>{" "}
            {answer.proposedChangesData
              ? "This changes data. Review it before you run it."
              : "A query for you to review and run yourself."}
          </span>
          <pre className={styles.sql}>{answer.proposedSql}</pre>
          <Button size="sm" icon={FileCode2} onClick={() => onOpenQuery(answer.proposedSql!)}>
            Open in editor
          </Button>
        </div>
      )}
      {answer.notes.length > 0 &&
        (answer.notesShared ? (
          <SharedNotes notes={answer.notes} />
        ) : answer.notesQueued ? (
          <QueuedNotes notes={answer.notes} />
        ) : (
          <SuggestedNotes notes={answer.notes} onSave={onSaveNote} />
        ))}
      <div className={styles.buttons}>{actions}</div>
      <span className={styles.muted}>
        <Database size={11} /> {answer.model} ·{" "}
        {(answer.usage.input + answer.usage.output).toLocaleString()} tokens
        {answer.usage.cached > 0 && ` (+${answer.usage.cached.toLocaleString()} from cache)`}
      </span>
    </section>
  );
}

/** The notes the run suggested on a team connection, sent to the team server for its knowledge owners. */
function SharedNotes({ notes }: { notes: string[] }) {
  return (
    <div className={styles.notes}>
      <span className={styles.muted}>
        <StickyNote size={12} /> Suggested notes for this database. Sent to your team's knowledge
        owners for approval.
      </span>
      {notes.map((n) => (
        <div key={n} className={styles.note}>
          <span>{n}</span>
          <Badge tone="neutral">sent</Badge>
        </div>
      ))}
    </div>
  );
}

/**
 * The notes the run suggested, waiting with the other suggestions: new facts, and fixes to
 * saved notes it found wrong. Accepting one here is the same as accepting it in the navigator.
 */
function QueuedNotes({ notes }: { notes: string[] }) {
  const { suggestions, accept, dismiss } = useNoteSuggestions();
  const [done, setDone] = useState<Record<string, string>>({});
  const mine = suggestions.filter((s) => s.app === "Investigate" && notes.includes(s.text));
  const finished = notes.filter((n) => done[n]);
  if (mine.length === 0 && finished.length === 0) return null;
  const settle = (text: string, f: Promise<unknown>, label: string) =>
    f.then(
      () => setDone((d) => ({ ...d, [text]: label })),
      (e) => setDone((d) => ({ ...d, [text]: String(e) })),
    );
  return (
    <div className={styles.notes}>
      <span className={styles.muted}>
        <StickyNote size={12} /> Suggested notes for this database. Accepted notes help every later
        question here.
      </span>
      {finished.map((n) => (
        <div key={n} className={styles.note}>
          <span>{n}</span>
          <Badge tone={done[n] === "accepted" ? "success" : "neutral"}>{done[n]}</Badge>
        </div>
      ))}
      {mine
        .filter((s) => !done[s.text])
        .map((s) => (
          <div key={s.id} className={styles.note}>
            <span>
              {s.replaces && <Badge tone="ai">{s.remove ? "remove a note" : "fixes a note"}</Badge>}{" "}
              {s.text}
            </span>
            <Button
              size="sm"
              variant="ghost"
              icon={Save}
              onClick={() => settle(s.text, accept(s.id), "accepted")}
            >
              Accept
            </Button>
            <Button
              size="sm"
              variant="ghost"
              icon={X}
              onClick={() => settle(s.text, dismiss(s.id), "dismissed")}
            >
              Dismiss
            </Button>
          </div>
        ))}
    </div>
  );
}

function SuggestedNotes({
  notes,
  onSave,
}: {
  notes: string[];
  onSave: (text: string) => Promise<void>;
}) {
  const [state, setState] = useState<Record<string, "saved" | "dismissed" | string>>({});
  const shown = notes.filter((n) => state[n] !== "dismissed");
  if (shown.length === 0) return null;
  return (
    <div className={styles.notes}>
      <span className={styles.muted}>
        <StickyNote size={12} /> Suggested notes for this database. Saved notes help every later
        question here.
      </span>
      {shown.map((n) => (
        <div key={n} className={styles.note}>
          <span>{n}</span>
          {state[n] === "saved" ? (
            <Badge tone="success">saved</Badge>
          ) : (
            <>
              <Button
                size="sm"
                variant="ghost"
                icon={Save}
                onClick={() =>
                  onSave(n).then(
                    () => setState((s) => ({ ...s, [n]: "saved" })),
                    (e) => setState((s) => ({ ...s, [n]: String(e) })),
                  )
                }
              >
                Save
              </Button>
              <Button
                size="sm"
                variant="ghost"
                icon={X}
                onClick={() => setState((s) => ({ ...s, [n]: "dismissed" }))}
              >
                Dismiss
              </Button>
              {state[n] && state[n] !== "saved" && (
                <span className={styles.errorText}>{state[n]}</span>
              )}
            </>
          )}
        </div>
      ))}
    </div>
  );
}
