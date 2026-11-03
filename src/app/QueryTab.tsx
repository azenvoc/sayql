import { useEffect, useRef, useState } from "react";
import { Plug, Wand2 } from "lucide-react";
import {
  QUERY_CANCELLED,
  commands,
  type QueryResult,
  type Schema,
  type Settings,
  type Skill,
  type TeamAiStatus,
} from "../api";
import { teamAiDestination } from "../features/team";
import { Badge, Button, SplitPane } from "../components/ui";
import {
  AgentAccessDialog,
  AgentPanel,
  agentStarters,
  describeAccess,
  useAgentAuto,
  forStorage,
  isActive,
  saveAgentAccess,
  useAgentAccess,
  useAgentRun,
} from "../features/agent";
import {
  parseCommand,
  planRequest,
  useSqlGeneration,
  withCommand,
  type TaskKind,
} from "../features/ask";
import { cx } from "../lib/cx";
import { errorText } from "../lib/errors";
import { resolveSelection, type TextRange } from "../features/editor";
import { statementRanges } from "../lib/sqlStatements";
import { ConfirmWriteDialog, useQueryRunner } from "../features/query";
import { ResultsPanel } from "../features/results";
import { ModelPicker, type ModelNames } from "../features/settings";
import type { EditorTab } from "../features/tabs";
import { SqlWorkspace } from "./SqlWorkspace";
import styles from "./QueryTab.module.css";

export interface QueryTabProps {
  tab: EditorTab;
  /** Hidden tabs stay mounted so their SQL, results and running requests survive switching. */
  active: boolean;
  label: string;
  schema: Schema | null;
  /**
   * Whether this tab's connection is connected. SayQL never connects on its own at start:
   * a restored tab connects when the person clicks Connect, runs or asks.
   */
  connected: boolean;
  /** Tagged as production: the not-connected bar says so. */
  production?: boolean;
  onConnect: () => void;
  settings: Settings;
  modelNames: ModelNames;
  onChange: (patch: Partial<EditorTab>) => void;
  onOpenSettings: () => void;
  skills: Skill[];
  /** "Create skill…" in the "/" menu. */
  onCreateSkill: () => void;
  onSave: () => void;
  /** Opens SQL in a new tab on the same database. */
  onOpenQuery: (sql: string, question: string) => void;
  /** Opens SQL in a new tab and asks for a script name. */
  onSaveAsScript: (sql: string, question: string) => void;
  /** An agent run started or ended in this tab (the tab shows a spinner). */
  onAgentBusy: (busy: boolean) => void;
  /** A database note was saved (the navigator lists them). */
  onNotesChanged: () => void;
  /** The Ask box switched model or provider. */
  onSettingsChange: (s: Settings) => void;
  /** Shows a message in the banner, e.g. when switching models fails. */
  onNotice: (message: string) => void;
  /** Team AI, when this tab's connection is a team connection and the team provides AI. */
  teamAi?: TeamAiStatus | null;
}

/** One editor tab: Ask bar and SQL editor above, results below, bound to the tab's database. */
export function QueryTab(props: QueryTabProps) {
  const {
    tab,
    active,
    label,
    schema,
    connected,
    settings,
    modelNames,
    onChange,
    onOpenSettings,
    skills,
    onCreateSkill,
    onSave,
  } = props;
  const askRef = useRef<HTMLTextAreaElement>(null);
  // On a team connection with team AI, the team's AI answers: no personal key needed.
  const teamAi = props.teamAi ?? null;
  const aiReady = settings.aiReady || !!teamAi;
  const setSql = (sql: string) => onChange({ sql });
  const lastRun = tab.lastRun ?? null;
  const focusEnabled = tab.focusEnabled ?? true;
  /** A failed AI query is fixed automatically once; if the fix fails too, the person decides. */
  const autoFixUsed = useRef(false);
  /** Where the selection is in the editor; Run runs just that. */
  const [selection, setSelection] = useState<TextRange | null>(null);
  /** A selection that cut a statement ran as whole statements: these, selected to show it. */
  const [widened, setWidened] = useState<{ range: TextRange; count: number } | null>(null);
  /** The last run was a selection: the AI fixes whole queries, so it stays out of it. */
  const ranSelection = useRef(false);

  /**
   * SayQL's AI may have written the editor's SQL. SQL an AI app handed over (MCP) isn't
   * fixed automatically, so it never spends the person's credits unasked; Fix with AI still can.
   */
  const aiWroteSql = useRef(!tab.origin);
  const query = useQueryRunner(tab.target, (sql, error) => {
    if (
      ranSelection.current ||
      autoFixUsed.current ||
      !aiWroteSql.current ||
      !aiReady ||
      !generation.question.trim()
    )
      return;
    autoFixUsed.current = true;
    generation.fix(sql, error);
  });

  /** Runs `sql` and remembers it as the query follow-ups build on. */
  const runAndRemember = (sql: string, question: string, task: TaskKind | "manual" = "manual") => {
    if (task !== "fix") autoFixUsed.current = false;
    ranSelection.current = false;
    setWidened(null);
    // Follow-ups see the question without its "/command".
    const asked = parseCommand(question, skills).rest;
    onChange({ lastRun: sql.trim() ? { question: asked, sql } : null });
    // On a team connection, the team server's audit log records the question the SQL answers.
    const fromAsk = (task !== "manual" || aiWroteSql.current) && !!asked.trim();
    query.run(
      sql,
      schema?.dialect,
      fromAsk ? { source: "ask", question: asked } : { source: "editor" },
    );
  };

  const generation = useSqlGeneration({
    target: tab.target,
    slot: tab.id,
    schema,
    typingModel: `${settings.provider}:${settings.typingModel}`,
    aiReady,
    initialQuestion: tab.question,
    previous: lastRun,
    // Hidden picks stay selected but don't reach the AI.
    focusTables: focusEnabled ? (tab.focusTables ?? []) : [],
    skills,
    sql: tab.sql,
    onQuestionChange: (question) => {
      onChange({ question });
      if (question.trim() && !connected) props.onConnect();
    },
    onSql: (sql) => {
      aiWroteSql.current = true;
      setSql(sql);
    },
    onRunRequested: (sql, task) => runAndRemember(sql, generation.question, task),
  });

  const run = () => {
    if (!generation.requestRun()) runAndRemember(tab.sql, generation.question);
  };

  /**
   * Runs `then` once the schema is loaded: straight away when connected, else after connecting
   * (Run and ↵ on a restored tab are what connect it).
   */
  const pending = useRef<(() => void) | null>(null);
  const whenReady = (then: () => void) => {
    if (connected && schema) return then();
    pending.current = then;
    if (!connected) props.onConnect();
  };
  useEffect(() => {
    if (!schema || !pending.current) return;
    const then = pending.current;
    pending.current = null;
    then();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [schema]);

  // Agent mode: one run per tab, kept (queries and answer, no rows) when it ends.
  const [auto, setAuto] = useAgentAuto();
  const agent = useAgentRun({
    target: tab.target,
    slot: tab.id,
    initial: tab.agentRun ?? null,
    auto,
  });
  const access = useAgentAccess(tab.target.connectionId);
  const [accessDialog, setAccessDialog] = useState<{ afterTurnOn: (() => void) | null } | null>(
    null,
  );
  const [view, setView] = useState<"agent" | "results">(tab.agentRun ? "agent" : "results");
  /** The agent step whose rows the Results view shows; null shows the last query run here. */
  const [picked, setPicked] = useState<number | null>(null);
  /** The next /agent question continues the last run ("Ask a follow-up"). */
  const followUpNext = useRef(false);
  const provider = settings.providers.find((p) => p.id === settings.provider);

  useEffect(() => {
    if (agent.state.runId && !isActive(agent.state))
      onChange({ agentRun: forStorage(agent.state) });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [agent.state.phase]);
  useEffect(() => props.onAgentBusy(agent.active), [agent.active]); // eslint-disable-line react-hooks/exhaustive-deps

  const investigate = (question: string, skill: string | null) => {
    if (!question.trim()) return;
    const followUp = followUpNext.current;
    const go = () => {
      followUpNext.current = false;
      setView("agent");
      setPicked(null);
      agent.start(question, {
        focusTables: focusEnabled ? (tab.focusTables ?? []) : [],
        skill,
        followUp,
      });
    };
    if (!aiReady) return onOpenSettings();
    if (!access?.enabled) return setAccessDialog({ afterTurnOn: go });
    go();
  };

  const runningStep = agent.state.steps.find((s) => s.status === "running");
  const agentLabel = agent.state.pending
    ? "Waiting for you"
    : runningStep
      ? `Step ${runningStep.id}: ${runningStep.text}`
      : agent.active
        ? "Thinking…"
        : null;
  const step = picked === null ? null : (agent.state.steps.find((s) => s.id === picked) ?? null);
  // A run kept with the tab keeps its queries, not its rows: opening such a step reads them again
  // (read-only, as the agent ran it).
  const [reread, setReread] = useState<
    Record<string, { result: QueryResult | null; error: string | null }>
  >({});
  const rereadKey = step ? `${agent.state.runId}:${step.id}` : "";
  const rowsLost =
    !!step?.sql && step.status === "done" && step.rows.length === 0 && (step.rowCount ?? 0) > 0;
  useEffect(() => {
    if (!rowsLost || !step?.sql || reread[rereadKey]) return;
    setReread((r) => ({ ...r, [rereadKey]: { result: null, error: null } }));
    commands.runQuery(tab.target, step.sql, false, undefined, "agent").then(
      (result) => setReread((r) => ({ ...r, [rereadKey]: { result, error: null } })),
      (e) => setReread((r) => ({ ...r, [rereadKey]: { result: null, error: errorText(e) } })),
    );
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [rereadKey, rowsLost]);
  const again = rowsLost ? reread[rereadKey] : undefined;
  const stepOutcome = step?.sql
    ? [
        {
          sql: step.sql,
          result: again
            ? again.result
            : step.status === "done"
              ? {
                  columns: step.columns,
                  rows: step.rows,
                  rowsAffected: null,
                  truncated: step.truncated,
                  elapsedMs: step.elapsedMs ?? 0,
                }
              : null,
          error: again ? again.error : step.error,
          cancelled: false,
        },
      ]
    : null;

  /**
   * Runs just the selected SQL, if any. A selection that cuts a statement (a DELETE without
   * its WHERE) runs the whole statements it touches instead, selected in the editor to show
   * it. Follow-ups keep building on the whole query, since the AI rewrites the whole editor.
   */
  const runSelectionOrAll = () => {
    const r = selection && resolveSelection(tab.sql, selection, schema?.dialect);
    if (!r) return run();
    ranSelection.current = true;
    setWidened(
      r.expanded ? { range: r.range, count: statementRanges(r.sql, schema?.dialect).length } : null,
    );
    query.run(r.sql, schema?.dialect, { source: "editor" });
  };

  /** ↵ in the Ask box: /explain, /fix and /improve work on the editor's SQL; anything else runs. */
  const submit = () => whenReady(submitNow);
  const submitNow = () => {
    const plan = planRequest(generation.question, skills, { previous: lastRun, sql: tab.sql });
    if (plan.kind === "agent") return investigate(plan.question, plan.skill);
    if (plan.kind === "draft") return run();
    generation.act(plan.command, tab.sql, plan.command === "fix" ? query.error : null);
  };

  // From the first-run setup: ask the tab's question as soon as it shows, then run the answer.
  const runWhenAnswered = useRef(false);
  useEffect(() => {
    if (active && tab.autoAsk && schema) {
      onChange({ autoAsk: false });
      runWhenAnswered.current = true;
      generation.setQuestion(tab.question);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [active, tab.autoAsk, schema]);
  useEffect(() => {
    const kind = generation.status.kind;
    if (!runWhenAnswered.current || kind === "idle" || kind === "writing") return;
    runWhenAnswered.current = false;
    // A write would still stop at the confirmation dialog; an error shows in the Ask box.
    if (kind === "done" || kind === "instant") runAndRemember(tab.sql, generation.question);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [generation.status]);

  // Table previews run as soon as the tab is first shown.
  useEffect(() => {
    if (active && tab.autoRun && schema) {
      onChange({ autoRun: false });
      runAndRemember(tab.sql, "");
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [active, tab.autoRun, schema]);

  useEffect(() => {
    if (active && schema && !tab.sql) askRef.current?.focus();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [active, schema]);

  /** The Ask box's Quick / Agent switch: adds or removes /agent, keeping the question. */
  const asked = parseCommand(generation.question, skills).command;
  const agentMode = asked?.kind === "builtin" && asked.builtin.slug === "investigate";
  const switchMode = (toAgent: boolean) => {
    const { command, rest } = parseCommand(generation.question, skills);
    if (!toAgent) return generation.setQuestion(rest);
    const skill = command?.kind === "skill" ? command.skill.slug : null;
    generation.setQuestion(withCommand("investigate", skill ? withCommand(skill, rest) : rest));
    if (!access?.enabled) setAccessDialog({ afterTurnOn: null });
  };

  const busy = generation.status.kind === "writing";
  const canUseAi = aiReady && !!schema;
  const autoFixFailed = autoFixUsed.current && !busy && !!query.error;

  return (
    <div hidden={!active} className={styles.tab}>
      {!connected && (
        <div className={styles.connectBar} role="status">
          <Plug size={14} />
          <span>
            Not connected to <strong>{label}</strong>
            {props.production && (
              <>
                {" "}
                <Badge tone="danger">PROD</Badge>
              </>
            )}
            {". "}SayQL connects when you click Connect, run or ask.
          </span>
          <Button size="sm" variant="primary" icon={Plug} onClick={props.onConnect}>
            Connect
          </Button>
        </div>
      )}
      <SplitPane
        direction="vertical"
        persistKey="editor"
        initialSize={340}
        minSize={200}
        maxSize={900}
        first={
          <SqlWorkspace
            connectionName={label}
            schema={schema}
            connected={connected}
            active={active}
            modelNames={modelNames}
            onOpenSettings={onOpenSettings}
            skills={{ list: skills, onCreate: onCreateSkill }}
            generation={{
              askRef,
              question: generation.question,
              onQuestionChange: generation.setQuestion,
              status: generation.status,
              onSubmit: submit,
              onRetry: generation.retry,
              onImprove: () => generation.improve(tab.sql),
              canImprove: canUseAi && !busy && !!tab.sql.trim(),
              followUp: lastRun
                ? { question: lastRun.question, onStartFresh: () => onChange({ lastRun: null }) }
                : undefined,
              explanation: generation.explanation,
              onDismissExplanation: generation.dismissExplanation,
              agentHint: (
                <>
                  {access?.enabled
                    ? describeAccess(access)
                    : "Investigate is off for this connection"}
                  <Button
                    size="sm"
                    variant="ghost"
                    onClick={() => setAccessDialog({ afterTurnOn: null })}
                  >
                    {access?.enabled ? "Change" : "Turn on"}
                  </Button>
                </>
              ),
              onInvestigate: canUseAi
                ? () => {
                    const { command, rest } = parseCommand(generation.question, skills);
                    const skill = command?.kind === "skill" ? command.skill.slug : null;
                    generation.setQuestion(
                      withCommand("investigate", skill ? withCommand(skill, rest) : rest),
                    );
                    investigate(rest, skill);
                  }
                : undefined,
              onModeChange: canUseAi ? switchMode : undefined,
              modelPicker: (
                <ModelPicker
                  settings={settings}
                  purpose={agentMode ? "agent" : "typing"}
                  onChange={props.onSettingsChange}
                  onError={props.onNotice}
                  onOpenSettings={onOpenSettings}
                  team={teamAi}
                />
              ),
              agentStarters: agentStarters(schema),
              agentAuto: { on: auto, onChange: setAuto },
            }}
            query={{
              sql: tab.sql,
              onSqlChange: setSql,
              onRun: () => whenReady(runSelectionOrAll),
              running: query.running,
              onCancel: query.cancel,
              selection,
              onSelectionChange: setSelection,
              select: widened?.range ?? null,
              onSave,
            }}
            focus={{
              tables: tab.focusTables ?? [],
              enabled: focusEnabled,
              onEnabledChange: (enabled) => onChange({ focusEnabled: enabled }),
              onTablesChange: (focusTables) => onChange({ focusTables }),
            }}
            origin={tab.origin}
            agent={{ running: agent.active, onStop: agent.stop, label: agentLabel }}
          />
        }
        second={
          agent.state.phase === "idle" ? (
            <ResultsPanel
              outcomes={query.outcomes}
              total={query.total}
              running={query.running}
              note={widened ? `Ran the full statement${widened.count > 1 ? "s" : ""}` : undefined}
              errorHint={
                autoFixFailed
                  ? `${modelNames.expert} couldn't fix it automatically. Try rephrasing your question, or edit the SQL above.`
                  : undefined
              }
              errorAction={
                // The AI rewrites one whole query, so a selection or a script is left to the person.
                canUseAi &&
                !ranSelection.current &&
                query.total === 1 &&
                query.error !== QUERY_CANCELLED &&
                !query.refused && (
                  <Button
                    size="sm"
                    variant="ghost"
                    icon={Wand2}
                    disabled={busy}
                    onClick={() => query.error && generation.fix(tab.sql, query.error)}
                  >
                    Fix with AI
                  </Button>
                )
              }
            />
          ) : (
            <div className={styles.results}>
              <div className={styles.views} role="tablist" aria-label="Investigation or results">
                {(["agent", "results"] as const).map((v) => (
                  <button
                    key={v}
                    type="button"
                    role="tab"
                    aria-selected={view === v}
                    className={cx(styles.view, view === v && styles.viewActive)}
                    onClick={() => setView(v)}
                  >
                    {v === "agent"
                      ? "Investigation"
                      : step
                        ? `Results · step ${step.id}`
                        : "Results"}
                  </button>
                ))}
              </div>
              {view === "agent" ? (
                <AgentPanel
                  run={agent.state}
                  selected={picked}
                  onSelect={(id) => {
                    setPicked(id);
                    setView("results");
                  }}
                  onGo={agent.go}
                  onAuto={() => setAuto(true)}
                  onSend={agent.send}
                  onSkip={agent.skip}
                  onAnswer={agent.answer}
                  onStop={agent.stop}
                  onOpenQuery={(sql) => props.onOpenQuery(sql, agent.state.question)}
                  onSaveScript={(sqls) =>
                    props.onSaveAsScript(joinStatements(sqls), agent.state.question)
                  }
                  onRerun={(sqls) => {
                    setPicked(null);
                    setView("results");
                    runAndRemember(joinStatements(sqls), agent.state.question);
                  }}
                  onFollowUp={() => {
                    followUpNext.current = true;
                    generation.setQuestion(withCommand("investigate", ""));
                    askRef.current?.focus();
                  }}
                  onSteer={(id, sql) => {
                    followUpNext.current = true;
                    investigate(
                      `I changed the query of step ${id}. Run this instead and continue from there:\n${sql}`,
                      null,
                    );
                  }}
                  onSaveNote={async (text) => {
                    await commands.saveNote({
                      id: "",
                      connectionId: tab.target.connectionId,
                      database: tab.target.database,
                      text,
                      updatedAt: 0,
                    });
                    props.onNotesChanged();
                  }}
                />
              ) : (
                <ResultsPanel
                  outcomes={stepOutcome ?? query.outcomes}
                  total={stepOutcome ? 1 : query.total}
                  running={
                    stepOutcome
                      ? step?.status === "running" || (!!again && !again.result && !again.error)
                      : query.running
                  }
                  note={
                    step
                      ? again
                        ? `Investigation step ${step.id}, run again: rows aren't kept after SayQL restarts`
                        : `Investigation step ${step.id}`
                      : undefined
                  }
                />
              )}
            </div>
          )
        }
      />
      {active && accessDialog && access && (
        <AgentAccessDialog
          connectionName={label}
          target={tab.target}
          provider={
            teamAi
              ? teamAiDestination(teamAi)
              : { label: provider?.label ?? settings.provider, local: provider?.local ?? false }
          }
          access={access}
          onSave={async (a) => {
            const saved = await saveAgentAccess(tab.target.connectionId, a);
            if (saved.enabled) accessDialog.afterTurnOn?.();
          }}
          onClose={() => setAccessDialog(null)}
        />
      )}
      {active && query.pendingWrite && (
        <ConfirmWriteDialog
          sql={query.pendingWrite}
          connectionName={label}
          onCancel={query.cancelWrite}
          onConfirm={query.confirmWrite}
        />
      )}
    </div>
  );
}

/** Statements as one script, each ending with `;`. */
const joinStatements = (sqls: string[]) =>
  sqls.map((s) => s.trim().replace(/;+$/, "") + ";").join("\n\n");
