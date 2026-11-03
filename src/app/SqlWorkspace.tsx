import type { ReactNode, RefObject } from "react";
import { CircleStop, Eraser, FileCode2, Play, Save, Square, Wand2 } from "lucide-react";
import type { Schema, Skill } from "../api";
import { Badge, Button, IconButton, Panel } from "../components/ui";
import { AskBar, type GenerationStatus } from "../features/ask";
import { voicePrompt } from "../features/voice";
import { SqlEditor, type TextRange } from "../features/editor";
import type { ModelNames } from "../features/settings";
import styles from "./Workbench.module.css";

/** The Ask box: the question and the AI's work on it. */
export interface WorkspaceGeneration {
  askRef: RefObject<HTMLTextAreaElement | null>;
  question: string;
  onQuestionChange: (q: string) => void;
  status: GenerationStatus;
  /** ↵ in the Ask box: run, or the picked /explain, /fix or /improve. */
  onSubmit: () => void;
  onRetry: () => void;
  /** Asks the expert model to review the SQL. */
  onImprove: () => void;
  canImprove: boolean;
  followUp?: { question: string; onStartFresh: () => void };
  explanation: string | null;
  onDismissExplanation: () => void;
  /** Agent mode's setting line for this connection, shown under the Ask box in agent mode. */
  agentHint?: ReactNode;
  onInvestigate?: () => void;
  onModeChange?: (agent: boolean) => void;
  modelPicker?: ReactNode;
  agentStarters?: string[];
  agentAuto?: { on: boolean; onChange: (on: boolean) => void };
}

/** An agent run in this tab: Run becomes Stop while it goes. */
export interface WorkspaceAgent {
  running: boolean;
  onStop: () => void;
  /** What the run is doing, for the toolbar. */
  label: string | null;
}

/** The SQL editor and running it. */
export interface WorkspaceQuery {
  sql: string;
  onSqlChange: (sql: string) => void;
  /** Runs the selected SQL if there is any, else all of it (Run button, ⌘↵ in the editor). */
  onRun: () => void;
  running: boolean;
  /** Stops the running query on the server. */
  onCancel: () => void;
  /** Where the selection is in the editor; Run runs just that (snapped to whole statements). */
  selection: TextRange | null;
  onSelectionChange: (selection: TextRange | null) => void;
  /** Shows what ran when a selection was widened. */
  select: TextRange | null;
  /** Saves the tab as a script (⌘S). */
  onSave: () => void;
}

/** Tables picked in the navigator as AI context. */
export interface WorkspaceFocus {
  tables: string[];
  enabled: boolean;
  onEnabledChange: (enabled: boolean) => void;
  onTablesChange: (names: string[]) => void;
}

export interface SqlWorkspaceProps {
  connectionName: string;
  schema: Schema | null;
  /** False on a restored tab not connected yet: Run and the Ask box stay usable, and connect first. */
  connected?: boolean;
  /** Whether this is the visible tab. */
  active: boolean;
  modelNames: ModelNames;
  onOpenSettings: () => void;
  skills: { list: Skill[]; onCreate: () => void };
  generation: WorkspaceGeneration;
  query: WorkspaceQuery;
  focus: WorkspaceFocus;
  /** Opened by an AI app, e.g. "From Claude (MCP)". */
  origin?: string;
  agent?: WorkspaceAgent;
}

/** The SQL editor view: the plain-English Ask bar above an editable SQL editor. */
export function SqlWorkspace({
  connectionName,
  schema,
  connected = true,
  active,
  modelNames,
  onOpenSettings,
  skills,
  generation: g,
  query: q,
  focus,
  origin,
  agent,
}: SqlWorkspaceProps) {
  // Loading the schema (or it failed): wait, unless the tab isn't connected yet, where Run and
  // asking are what connect it, or a question is already being typed.
  const waiting = connected && !schema;
  const writing = g.status.kind === "writing";
  const hasSelection = !!q.selection && !!q.sql.slice(q.selection.start, q.selection.end).trim();
  return (
    <Panel
      title="SQL Editor"
      icon={FileCode2}
      subtitle={connectionName}
      actions={
        <>
          {origin && (
            <Badge tone="ai" title="Opened by an AI app. Nothing has run.">
              {origin}
            </Badge>
          )}
          <IconButton
            icon={Save}
            label="Save script (⌘S)"
            size="sm"
            onClick={q.onSave}
            disabled={!q.sql.trim()}
          />
          <IconButton
            icon={Eraser}
            label="Clear"
            size="sm"
            onClick={() => q.onSqlChange("")}
            disabled={!q.sql}
          />
          <Button
            size="sm"
            variant="ghost"
            icon={Wand2}
            onClick={g.onImprove}
            disabled={!g.canImprove}
            title={`Review and improve this query with ${modelNames.expert}`}
          >
            Improve
          </Button>
          {q.running && (
            <Button
              size="sm"
              variant="ghost"
              icon={Square}
              onClick={q.onCancel}
              title="Stop the running query on the server"
            >
              Cancel
            </Button>
          )}
          {agent?.running ? (
            <>
              {agent.label && <span className={styles.agentLabel}>{agent.label}</span>}
              <Button
                size="sm"
                variant="danger"
                icon={CircleStop}
                onClick={agent.onStop}
                title="Stop the investigation and its running query"
              >
                Stop
              </Button>
            </>
          ) : (
            <Button
              size="sm"
              variant="primary"
              icon={Play}
              shortcut="⌘↵"
              onClick={q.onRun}
              loading={q.running}
              disabled={waiting || (!q.sql.trim() && !writing)}
              title={hasSelection ? "Run only the selected statements" : undefined}
            >
              {hasSelection ? "Run selection" : writing ? "Run when ready" : "Run"}
            </Button>
          )}
        </>
      }
    >
      <AskBar
        ref={g.askRef}
        question={g.question}
        onQuestionChange={g.onQuestionChange}
        onSubmit={g.onSubmit}
        status={g.status}
        modelNames={modelNames}
        disabled={waiting && !g.question.trim()}
        onOpenSettings={onOpenSettings}
        onRetry={g.onRetry}
        followUp={g.followUp}
        skills={skills.list}
        onCreateSkill={skills.onCreate}
        explanation={g.explanation}
        onDismissExplanation={g.onDismissExplanation}
        focusTables={focus.tables}
        focusEnabled={focus.enabled}
        onFocusEnabledChange={focus.onEnabledChange}
        onFocusTablesChange={focus.onTablesChange}
        active={active}
        voicePrompt={voicePrompt(schema, focus.tables)}
        agentHint={g.agentHint}
        onInvestigate={g.onInvestigate}
        onModeChange={g.onModeChange}
        modelPicker={g.modelPicker}
        agentStarters={g.agentStarters}
        agentAuto={g.agentAuto}
      />
      <div className={styles.editor}>
        <SqlEditor
          value={q.sql}
          onChange={q.onSqlChange}
          onRun={q.onRun}
          onSelectionChange={q.onSelectionChange}
          select={q.select}
          schema={schema}
          active={active}
        />
      </div>
    </Panel>
  );
}
