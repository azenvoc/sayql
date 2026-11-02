import { useCallback, useEffect, useRef, useState } from "react";
import { AlertTriangle, Check, Copy, Lightbulb, ScanSearch, SquareTerminal } from "lucide-react";
import { openUrl } from "@tauri-apps/plugin-opener";
import {
  commands,
  subscribeToAudit,
  type AuditAgent,
  type AuditEvent,
  type AuditModel,
  type KnowledgeSource,
  type ConnectionConfig,
} from "../../api";
import { Alert, Button, Dialog, Markdown, Select, Spinner } from "../../components/ui";
import { cx } from "../../lib/cx";
import { errorText } from "../../lib/errors";
import { AUDITS_CHANGED, copyAuditPrompt, type AuditTarget } from "./audit";
import { KnowledgeSources } from "./KnowledgeSources";
import { SUGGESTIONS_CHANGED } from "./useNoteSuggestions";
import styles from "./AuditRun.module.css";

// Detecting agents runs `--version`: once per session, again when the dialog opens.
let detected: Promise<AuditAgent[]> | null = null;

function detect(fresh = false): Promise<AuditAgent[]> {
  if (!detected || fresh) {
    detected = Promise.resolve()
      .then(() => commands.auditAgents())
      .catch(() => []);
  }
  return detected;
}

/** The coding agents SayQL knows, as found on this computer; null while it looks. */
export function useAuditAgents(fresh = false) {
  const [agents, setAgents] = useState<AuditAgent[] | null>(null);
  const load = useCallback((again: boolean) => {
    let live = true;
    detect(again).then((list) => live && setAgents(list));
    return () => {
      live = false;
    };
  }, []);
  useEffect(() => load(fresh), [load, fresh]);
  const refresh = () => {
    setAgents(null);
    load(true);
  };
  return { agents, refresh };
}

/** The audit button's label: always "Run audit" (the feature isn't named after the agent it runs). */
export function auditButtonLabel(_agents?: AuditAgent[] | null): string {
  return "Run audit";
}

/** Plain text saying what the button runs: "Runs your installed Claude Code, read-only, …". */
export function auditButtonTitle(agents: AuditAgent[] | null): string {
  // The dialog starts with the first ready one: Claude Code, else SayQL's built-in OpenCode.
  const first = (agents ?? []).find((a) => a.ready);
  const what = !first ? "your coding agent" : first.builtIn ? first.label : `your installed ${first.label}`;
  return `Runs ${what}, read-only, over this database's code`;
}

/** "about 66 MB". */
function megabytes(bytes: number): string {
  return `about ${Math.max(1, Math.round(bytes / 1_000_000))} MB`;
}

export interface AuditButtonProps {
  /** The connection to audit; without one the dialog asks. */
  target?: AuditTarget | null;
  /** Connections to choose from when there's no target (team connections are left out). */
  connections?: ConnectionConfig[];
  /** Shows the suggestions, from the finished run. */
  onReview?: () => void;
  /** Opens the run somewhere else instead (e.g. in place of a dialog the button sits in). */
  onOpen?: () => void;
  variant?: "primary" | "secondary" | "ghost";
}

/** Runs the audit with the person's coding agent, read-only, over the connection's knowledge sources. */
export function AuditButton({ target, connections, onReview, onOpen, variant = "secondary" }: AuditButtonProps) {
  const { agents } = useAuditAgents();
  const [open, setOpen] = useState(false);
  const name = target?.name;
  return (
    <>
      <Button
        size="sm"
        variant={variant}
        icon={ScanSearch}
        aria-label={name ? `${auditButtonLabel(agents)} on ${name}` : undefined}
        title={auditButtonTitle(agents)}
        onClick={() => (onOpen ? onOpen() : setOpen(true))}
      >
        {auditButtonLabel(agents)}
      </Button>
      {open && <AuditDialog target={target} connections={connections} onReview={onReview} onClose={() => setOpen(false)} />}
    </>
  );
}

type Line = { kind: "step" | "note" | "warn"; text: string };
type Done = Extract<AuditEvent, { type: "done" }>;

export interface AuditDialogProps {
  target?: AuditTarget | null;
  connections?: ConnectionConfig[];
  onReview?: () => void;
  onClose: () => void;
}

const MODELS: { value: AuditModel; label: string }[] = [
  { value: "sonnet", label: "Sonnet (cheaper)" },
  { value: "opus", label: "Opus" },
  { value: "default", label: "My Claude Code default" },
];

function elapsed(ms: number): string {
  const s = Math.floor(ms / 1000);
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
}

/** Choose the agent and the code, start, watch the progress, then review what it suggested. */
export function AuditDialog({ target, connections = [], onReview, onClose }: AuditDialogProps) {
  const { agents, refresh } = useAuditAgents(true);
  const ready = (agents ?? []).filter((a) => a.ready);
  const local = connections.filter((c) => !c.id.startsWith("team:"));
  const [agentId, setAgentId] = useState<string | null>(null);
  const agent = ready.find((a) => a.id === agentId) ?? ready[0] ?? null;
  const [connectionId, setConnectionId] = useState(target?.connectionId ?? local[0]?.id ?? "");
  const database = target && target.connectionId === connectionId ? target.database : null;
  const connectionName = target?.connectionId === connectionId ? target.name : (local.find((c) => c.id === connectionId)?.name ?? "");
  const [sources, setSources] = useState<KnowledgeSource[]>([]);
  const [model, setModel] = useState<AuditModel>("sonnet");
  const [phase, setPhase] = useState<"setup" | "running" | "done">("setup");
  const [lines, setLines] = useState<Line[]>([]);
  const [done, setDone] = useState<Done | null>(null);
  const [download, setDownload] = useState<{ received: number; total: number } | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  const [started, setStarted] = useState(0);
  const [now, setNow] = useState(0);
  const runId = useRef<string | null>(null);
  const logRef = useRef<HTMLOListElement>(null);
  const suggested = useRef(false);

  useEffect(() => {
    let live = true;
    Promise.resolve()
      .then(() => commands.auditModel())
      .then(
        (m) => live && m && setModel(m),
        () => undefined,
      );
    return () => {
      live = false;
    };
  }, []);

  const chooseModel = (m: AuditModel) => {
    setModel(m);
    void Promise.resolve()
      .then(() => commands.setAuditModel(m))
      .catch((e) => setError(errorText(e)));
  };

  useEffect(() => {
    const unlisten = subscribeToAudit((e) => {
      if (e.runId !== runId.current) return;
      if (e.type === "line") {
        if (e.kind === "note") suggested.current = true;
        setLines((all) => [...all, { kind: e.kind, text: e.text }]);
      } else if (e.type === "download") {
        setDownload(e.received < e.total ? { received: e.received, total: e.total } : null);
      } else {
        setDownload(null);
        if (e.suggested > 0) suggested.current = true;
        setDone(e);
        setPhase("done");
      }
    });
    return () => void unlisten.then((u) => u());
  }, []);

  useEffect(() => {
    if (phase !== "running") return;
    const t = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(t);
  }, [phase]);

  useEffect(() => {
    const log = logRef.current;
    if (log) log.scrollTop = log.scrollHeight;
  }, [lines]);

  const close = () => {
    if (phase === "running" && runId.current) void commands.cancelAudit(runId.current).catch(() => undefined);
    if (suggested.current) window.dispatchEvent(new Event(SUGGESTIONS_CHANGED));
    window.dispatchEvent(new Event(AUDITS_CHANGED));
    onClose();
  };

  const start = () => {
    if (!agent || sources.length === 0 || !connectionId) return;
    const id = crypto.randomUUID();
    runId.current = id;
    setError(null);
    setLines([]);
    setDone(null);
    setDownload(null);
    setStarted(Date.now());
    setNow(Date.now());
    setPhase("running");
    commands.startAudit(id, agent.id, connectionId, database).catch((e) => {
      runId.current = null;
      setPhase("setup");
      setError(errorText(e));
      // A first run downloads: the next one may not need to.
      if (agent.builtIn) refresh();
    });
  };

  const openTerminal = () => {
    if (!agent || sources.length === 0 || !connectionId) return;
    setError(null);
    commands.openAuditInTerminal(agent.id, connectionId, database).catch((e) => setError(errorText(e)));
  };

  const copyPrompt = () => {
    if (!connectionId) return;
    setError(null);
    copyAuditPrompt({ connectionId, database }).then(
      () => {
        setCopied(true);
        window.setTimeout(() => setCopied(false), 1500);
      },
      (e) => setError(errorText(e)),
    );
  };

  const review = () => {
    close();
    onReview?.();
  };

  const canStart = !!agent && sources.length > 0 && !!connectionId;
  const labels = sources.map((s) => s.label).join(", ");
  const actions =
    phase === "running" ? (
      <Button onClick={() => runId.current && void commands.cancelAudit(runId.current).catch(() => undefined)}>Cancel</Button>
    ) : phase === "done" ? (
      <>
        <Button
          variant="ghost"
          onClick={() => {
            setPhase("setup");
            // The first run downloaded it: say so no more.
            if (agent?.builtIn && agent.downloadBytes) refresh();
          }}
        >
          Run again
        </Button>
        {onReview && done && done.suggested > 0 ? (
          <Button variant="primary" onClick={review}>
            Review suggestions
          </Button>
        ) : (
          <Button variant="primary" onClick={close}>
            Done
          </Button>
        )}
      </>
    ) : agent ? (
      <>
        {!agent.builtIn && (
          <Button
            variant="ghost"
            icon={SquareTerminal}
            disabled={!canStart}
            title="Open your terminal in the first code source, running the same read-only audit, to watch or answer it there"
            onClick={openTerminal}
          >
            Open in Terminal
          </Button>
        )}
        <Button variant="primary" disabled={!canStart} onClick={start}>
          {agent.downloadBytes ? "Download and start" : "Start audit"}
        </Button>
      </>
    ) : (
      <Button onClick={close}>Close</Button>
    );

  return (
    <Dialog title="Run an audit" onClose={close} actions={actions}>
      <div className={styles.body}>
        {phase === "setup" && (
          <>
            {agents === null ? (
              <p className={styles.muted}>
                <Spinner /> Looking for coding agents on this computer…
              </p>
            ) : agent ? (
              <>
                {agent.builtIn ? (
                  <p className={styles.lead}>
                    Runs OpenCode {agent.version}, built into SayQL, read-only, over this database&apos;s knowledge sources:
                    code, files and web pages. It reads them and SayQL&apos;s schema and suggests notes for you to approve. It talks to
                    the AI you chose in Settings → AI provider{agent.detail ? ` (${agent.detail})` : ""}, which sees what it reads.
                  </p>
                ) : (
                  <p className={styles.lead}>
                    Runs your installed {agent.label}, read-only, over this database&apos;s knowledge sources: code,
                    files and web pages. It reads them and SayQL&apos;s schema and suggests notes for you to approve. It uses your own{" "}
                    {agent.label} sign-in, however you&apos;ve set it up: SayQL never reads your {agent.label} credentials and sends no
                    code anywhere.
                  </p>
                )}
                {agent.builtIn && agent.downloadBytes ? (
                  <p className={styles.hint}>
                    Downloads OpenCode {agent.version} and ripgrep, {megabytes(agent.downloadBytes)}, once, from GitHub. SayQL checks
                    them against pinned checksums before running them.
                  </p>
                ) : null}
                <div className={styles.fields}>
                  {!target && (
                    <label className={styles.field}>
                      <span className={styles.label}>Connection</span>
                      {local.length > 0 ? (
                        <Select
                          aria-label="Connection to audit"
                          value={connectionId}
                          options={local.map((c) => ({ value: c.id, label: c.name }))}
                          onChange={setConnectionId}
                        />
                      ) : (
                        <span className={styles.muted}>No saved connections yet.</span>
                      )}
                    </label>
                  )}
                  {target && (
                    <div className={styles.field}>
                      <span className={styles.label}>Connection</span>
                      <span>
                        {connectionName}
                        {database ? ` · ${database}` : ""}
                      </span>
                    </div>
                  )}
                  {ready.length > 1 && (
                    <label className={styles.field}>
                      <span className={styles.label}>Audit with</span>
                      <Select aria-label="Audit with" value={agent.id} options={ready.map((a) => ({ value: a.id, label: a.label }))} onChange={setAgentId} />
                    </label>
                  )}
                  {agent.id === "claude" && (
                    <>
                      <div className={styles.field}>
                        <span className={styles.label}>Model</span>
                        <Select aria-label="Model for the audit" value={model} options={MODELS} onChange={chooseModel} />
                      </div>
                      <p className={styles.hint}>
                        {model === "sonnet"
                          ? "Sonnet is cheaper and does this job well. "
                          : model === "opus"
                            ? "Opus is slower and uses more of your plan. "
                            : "Whatever model your Claude Code uses by default. "}
                        The run counts against your own Claude plan or however Claude Code is set up.
                      </p>
                    </>
                  )}
                  <div className={styles.sources}>
                    <span className={styles.label}>Knowledge sources</span>
                    {connectionId ? (
                      <KnowledgeSources connectionId={connectionId} onChange={setSources} />
                    ) : (
                      <span className={styles.muted}>Choose a connection first.</span>
                    )}
                  </div>
                </div>
                <ul className={styles.rules} aria-label="What the agent may do">
                  <li>
                    <Check size={13} /> Read, search and list files in these sources, and use SayQL&apos;s tools.
                  </li>
                  <li>
                    <Check size={13} /> Repositories are fetched first with your own git, to this computer only.
                  </li>
                  <li>
                    <Check size={13} /> No commands, no file changes, no web{agent.builtIn ? ", no .env files" : ""}. SayQL starts it
                    with these limits.
                  </li>
                  <li>
                    <Check size={13} /> Queries go through SayQL: it gets back only what this connection allows AI apps.
                  </li>
                </ul>
              </>
            ) : (
              <NoAgent agents={agents} onCheckAgain={refresh} />
            )}
            {agents !== null && connectionId && (
              <div className={styles.copy}>
                <Button size="sm" variant="ghost" icon={copied ? Check : Copy} onClick={copyPrompt}>
                  {copied ? "Copied" : "Copy audit prompt"}
                </Button>
                <span className={styles.muted}>to paste into another AI app connected to SayQL.</span>
              </div>
            )}
          </>
        )}
        {phase !== "setup" && (
          <>
            <div className={styles.status}>
              {phase === "running" ? (
                <>
                  <Spinner /> Auditing {connectionName} in <code className={styles.path}>{labels}</code>
                  <span className={styles.time}>{elapsed(Math.max(0, now - started))}</span>
                </>
              ) : (
                <span className={styles.muted}>
                  {connectionName} in <code className={styles.path}>{labels}</code>
                </span>
              )}
            </div>
            <ol className={styles.log} ref={logRef} role="log" aria-live="polite" aria-label="Audit progress">
              {lines.length === 0 && phase === "running" && <li className={styles.muted}>Starting {agent?.label ?? "the agent"}…</li>}
              {download && phase === "running" && (
                <li className={styles.muted}>
                  <Spinner /> Downloading OpenCode: {Math.round(download.received / 1_000_000)} of {Math.round(download.total / 1_000_000)} MB
                </li>
              )}
              {lines.map((l, i) => (
                <li key={i} className={cx(styles.line, styles[l.kind])}>
                  {l.kind === "note" ? <Lightbulb size={13} /> : l.kind === "warn" ? <AlertTriangle size={13} /> : <span className={styles.dot} />}
                  <span>{l.text}</span>
                </li>
              ))}
            </ol>
            {done && <Result done={done} onReview={onReview ? review : undefined} />}
          </>
        )}
        {error && <Alert tone="danger">{error}</Alert>}
      </div>
    </Dialog>
  );
}

function noteCount(n: number): string {
  return `${n} ${n === 1 ? "note" : "notes"} suggested`;
}

function Result({ done, onReview }: { done: Done; onReview?: () => void }) {
  const where = onReview ? null : " They wait under Notes in the navigator and in Settings → AI apps → Suggestions.";
  const link =
    onReview && done.suggested > 0 ? (
      <>
        {" "}
        <button type="button" className={styles.link} onClick={onReview}>
          Review them
        </button>
      </>
    ) : (
      where && done.suggested > 0 && where
    );
  if (done.outcome === "ok") {
    return (
      <>
        <Alert tone="success">
          {done.suggested > 0 ? (
            <>
              <strong>{noteCount(done.suggested)}.</strong>
              {link}
            </>
          ) : (
            "Done: no notes suggested."
          )}
        </Alert>
        {done.summary && <Markdown className={styles.summary} text={done.summary} onLink={(u) => void openUrl(u)} />}
      </>
    );
  }
  const tone = done.outcome === "error" ? "danger" : done.outcome === "timeout" ? "warning" : "info";
  const text = done.outcome === "cancelled" ? "Audit cancelled." : (done.error ?? "The audit stopped.");
  return (
    <Alert tone={tone}>
      {text}
      {done.suggested > 0 && (
        <>
          {" "}
          {noteCount(done.suggested)} before it stopped.{link}
        </>
      )}
    </Alert>
  );
}

/** No agent SayQL can run: what was found, and where to get Claude Code. */
function NoAgent({ agents, onCheckAgain }: { agents: AuditAgent[]; onCheckAgain: () => void }) {
  const supported = agents.filter((a) => a.supported && !a.builtIn);
  const found = agents.filter((a) => (a.path || a.builtIn) && a.problem);
  return (
    <div className={styles.none}>
      <p className={styles.lead}>
        SayQL runs the audit with a coding agent on this computer, read-only. It can keep{" "}
        {supported.map((a) => a.label).join(", ")} read-only, or run its own built-in OpenCode with your AI provider; neither
        can run yet.
      </p>
      {found.length > 0 && (
        <ul className={styles.found}>
          {found.map((a) => (
            <li key={a.id}>
              <strong>{a.label}</strong> {a.path && !a.builtIn && <span className={styles.muted}>({a.path})</span>}: {a.problem}
            </li>
          ))}
        </ul>
      )}
      <div className={styles.copy}>
        {supported
          .filter((a) => !a.path)
          .map((a) => (
            <Button key={a.id} size="sm" variant="primary" onClick={() => void openUrl(a.installUrl)}>
              Install {a.label}
            </Button>
          ))}
        <Button size="sm" variant="ghost" onClick={onCheckAgain}>
          Check again
        </Button>
      </div>
      <p className={styles.muted}>
        Using Codex, Cursor or another app? Connect it to SayQL (Settings → AI apps → Access) and paste the audit prompt
        into it.
      </p>
    </div>
  );
}
