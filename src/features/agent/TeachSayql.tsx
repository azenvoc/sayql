import { useId, useState } from "react";
import { Check, ChevronRight, Copy, X } from "lucide-react";
import type { ConnectionConfig } from "../../api";
import { Button, IconButton } from "../../components/ui";
import { cx } from "../../lib/cx";
import { errorText } from "../../lib/errors";
import { AgentPicker, AgentSetupView } from "./AgentSetup";
import type { CodingAgent } from "./agents";
import { copyAuditPrompt, type AuditTarget } from "./audit";
import { AuditButton } from "./AuditRun";
import { useCodingAgent } from "./useCodingAgent";
import styles from "./TeachSayql.module.css";

export interface TeachSayqlProps {
  /** Opens the setup: Settings → AI apps → Access → Connect an app. */
  onSetUp?: () => void;
  /** The connection the card is about: the command names it, and Copy audit prompt is offered. */
  target?: AuditTarget | null;
  /** Show the title (a dialog shows it in its own header). */
  heading?: boolean;
  /** Hides the card for good, where it can be hidden. */
  onDismiss?: () => void;
  /** Without a target: the connections "Run audit" offers. */
  connections?: ConnectionConfig[];
  /** Shows the suggestions, from a finished audit run. */
  onReview?: () => void;
  /** Opens "Run audit" somewhere else (in place of the dialog the card is in). */
  onAudit?: () => void;
}

/**
 * "Teach SayQL your database": pick your coding agent, connect it, run the audit in your project,
 * approve the notes it suggests. Shown on first use and as the empty state of suggested notes.
 */
export function TeachSayql({
  onSetUp,
  target,
  heading = true,
  onDismiss,
  connections,
  onReview,
  onAudit,
}: TeachSayqlProps) {
  const { agent, choose, context, apps, connectedAs } = useCodingAgent();
  const connected = (apps ?? []).some((a) => a.lastUsed != null);
  const [copied, setCopied] = useState<"command" | "prompt" | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [setupOpen, setSetupOpen] = useState(false);
  const setupId = useId();
  const flash = (what: "command" | "prompt") => {
    setCopied(what);
    window.setTimeout(() => setCopied(null), 1500);
  };
  const copyCommand = (command: string) => {
    setError(null);
    navigator.clipboard.writeText(command).then(
      () => flash("command"),
      (e) => setError(errorText(e)),
    );
  };
  const copyPrompt = () => {
    if (!target) return;
    setError(null);
    copyAuditPrompt(target).then(
      () => flash("prompt"),
      (e) => setError(errorText(e)),
    );
  };
  const done = connected;
  const setup = context ? (
    <AgentSetupView key={agent.id} agent={agent} context={context} compact />
  ) : (
    onSetUp && (
      <>
        {" "}
        <button type="button" className={styles.link} onClick={onSetUp}>
          Set it up
        </button>
      </>
    )
  );

  return (
    <section
      className={cx(styles.card, !heading && styles.bare)}
      aria-label="Teach SayQL your database"
    >
      {(heading || onDismiss) && (
        <div className={styles.head}>
          {heading && <h3 className={styles.title}>Teach SayQL your database</h3>}
          {onDismiss && (
            <IconButton icon={X} label="Don't show again" size="sm" onClick={onDismiss} />
          )}
        </div>
      )}
      <p className={styles.lead}>
        Your code knows things your schema doesn&apos;t: which tables are retired, what a status
        means. Let your coding agent tell SayQL.
      </p>
      <AgentPicker agent={agent} onChoose={choose} />
      <ol className={styles.steps}>
        <li className={done ? styles.done : undefined}>
          <span className={styles.marker} aria-hidden>
            {done ? <Check size={12} /> : 1}
          </span>
          <div className={styles.body}>
            {connectedAs ? (
              <>
                <strong>{agent.short} is connected.</strong> It already uses SayQL.
              </>
            ) : connected ? (
              <>
                <strong>An AI app is connected.</strong> {connectedNames(apps ?? [])} SayQL.{" "}
                {context && (
                  <>
                    <button
                      type="button"
                      className={styles.link}
                      aria-expanded={setupOpen}
                      aria-controls={setupId}
                      onClick={() => setSetupOpen(!setupOpen)}
                    >
                      <ChevronRight
                        size={12}
                        className={cx(styles.chevron, setupOpen && styles.chevronOpen)}
                      />
                      Connect {agent.short} too
                    </button>
                    {setupOpen && <div id={setupId}>{setup}</div>}
                  </>
                )}
              </>
            ) : (
              <>
                <strong>Connect {agent.short} to SayQL.</strong>
                {setup}
              </>
            )}
          </div>
        </li>
        <li>
          <span className={styles.marker} aria-hidden>
            2
          </span>
          <div className={styles.body}>
            <AuditStep
              agent={agent}
              target={target}
              connections={connections}
              onReview={onReview}
              onAudit={onAudit}
              copied={copied}
              onCopyCommand={copyCommand}
              onCopyPrompt={copyPrompt}
            />
          </div>
        </li>
        <li>
          <span className={styles.marker} aria-hidden>
            3
          </span>
          <div className={styles.body}>
            <strong>Approve what&apos;s right.</strong> Every note you accept makes SayQL&apos;s
            answers better, for you and every agent after.
          </div>
        </li>
      </ol>
      <p className={styles.again}>
        Run it again now and then, and after big changes: code moves on and notes go stale. The
        audit also flags notes that look out of date.
      </p>
      {error && <p className={styles.error}>{error}</p>}
    </section>
  );
}

/** "Claude Code already uses", "Claude Code and Cursor already use", "Claude Code, Cursor and 2 more already use". */
function connectedNames(apps: { name: string; lastUsed?: number | null }[]): string {
  const names = apps.filter((a) => a.lastUsed != null).map((a) => a.name);
  if (names.length <= 1) return `${names[0] ?? "An app"} already uses`;
  if (names.length === 2) return `${names[0]} and ${names[1]} already use`;
  return `${names[0]}, ${names[1]} and ${names.length - 2} more already use`;
}

interface AuditStepProps {
  agent: CodingAgent;
  target?: AuditTarget | null;
  connections?: ConnectionConfig[];
  onReview?: () => void;
  onAudit?: () => void;
  copied: "command" | "prompt" | null;
  onCopyCommand: (command: string) => void;
  onCopyPrompt: () => void;
}

/** Step 2, for the agent: SayQL runs it, its slash command, its prompt menu, or paste the prompt. */
function AuditStep({
  agent,
  target,
  connections,
  onReview,
  onAudit,
  copied,
  onCopyCommand,
  onCopyPrompt,
}: AuditStepProps) {
  const how = agent.audit;
  const paste = how.kind === "paste";
  const command = how.kind === "run" || how.kind === "prompt" ? how.command(target) : null;
  const commandLine = command && (
    <>
      <code className={styles.command}>{command}</code>
      <IconButton
        icon={copied === "command" ? Check : Copy}
        label={copied === "command" ? "Copied" : "Copy the command"}
        size="sm"
        onClick={() => onCopyCommand(command)}
      />
    </>
  );
  const promptButton = target ? (
    <div className={styles.copy}>
      <Button
        size="sm"
        variant={paste ? "secondary" : "ghost"}
        icon={copied === "prompt" ? Check : Copy}
        onClick={onCopyPrompt}
      >
        {copied === "prompt" ? "Copied" : "Copy audit prompt"}
      </Button>
      {!paste && <span className={styles.hint}>or paste the prompt into the chat yourself.</span>}
    </div>
  ) : (
    <span className={styles.hint}>
      {" "}
      {paste ? "Copy the audit prompt" : "To paste the prompt instead, copy it"} for a connection in
      Settings → AI apps → Access.
    </span>
  );

  return (
    <>
      <strong>Run an audit in your project.</strong>{" "}
      {how.kind === "run" ? (
        <>
          Let SayQL run your installed {agent.short} in the project folder, read-only, on your own
          sign-in, with no setup:
          <div className={styles.copy}>
            <AuditButton
              target={target}
              connections={connections}
              onReview={onReview}
              onOpen={onAudit}
              variant="primary"
            />
          </div>
          Or in {how.where}, type {commandLine}
          <br />
        </>
      ) : how.kind === "prompt" ? (
        <>
          In {how.where}, type {commandLine}
          <br />
        </>
      ) : how.kind === "menu" ? (
        <>{how.text} </>
      ) : (
        <>Copy the audit prompt and paste it into {how.where}. </>
      )}
      It reads your schema, its usage figures and your code, and suggests notes. It needs no rows;
      any query it runs gets only what you allow on that connection.
      {agent.caveat && <span className={styles.hint}> {agent.caveat}</span>}
      {promptButton}
    </>
  );
}
