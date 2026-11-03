import { useEffect, useId, useRef, useState, type KeyboardEvent } from "react";
import { ChevronRight } from "lucide-react";
import { commands, type ConnectionConfig, type McpApps, type McpSetup } from "../../api";
import { Alert } from "../../components/ui";
import { cx } from "../../lib/cx";
import { errorText } from "../../lib/errors";
import { AgentPicker, AgentSetupView, useCodingAgent, type NoteSuggestionsApi } from "../agent";
import { AgentActivity } from "./AgentActivity";
import { AppAccessList } from "./AppAccessList";
import { AppsList } from "./AppsList";
import { SuggestedNotes } from "./SuggestedNotes";
import styles from "./ClaudeIntegration.module.css";

export type AiAppsTab = "access" | "activity" | "suggestions";

const TABS: { id: AiAppsTab; label: string }[] = [
  { id: "access", label: "Access" },
  { id: "activity", label: "Activity" },
  { id: "suggestions", label: "Suggestions" },
];

/** The tab last open, so coming back to Settings → AI apps in this session lands on it. */
let lastTab: AiAppsTab = "access";

/**
 * How to add SayQL to the person's coding agent, first on the Access tab: open and set apart
 * while no app has connected (`first`), one line that opens once one has.
 */
function ConnectApp({
  setup,
  open,
  first,
  onToggle,
}: {
  setup: McpSetup | null;
  open: boolean;
  first: boolean;
  onToggle: () => void;
}) {
  const bodyId = useId();
  const { agent, choose, context } = useCodingAgent();
  return (
    <section className={cx(styles.group, first && styles.connectFirst)}>
      <button
        type="button"
        className={styles.disclosure}
        aria-expanded={open}
        aria-controls={bodyId}
        onClick={onToggle}
      >
        <ChevronRight size={14} className={cx(styles.chevron, open && styles.chevronOpen)} />
        <span className={styles.heading}>Connect an app</span>
      </button>
      {!open && !first && (
        <p className={styles.detail}>
          Add SayQL to another AI app: Claude Code, Cursor, VS Code, Codex and more.
        </p>
      )}
      {open && (
        <div id={bodyId} className={styles.groupBody}>
          <p className={styles.intro}>
            Add SayQL to an AI app once. It can also fill in a new connection, or open SQL or a
            saved script here, for you to finish. Passwords and writes never go through MCP.
          </p>
          {setup && !setup.exists && (
            <Alert tone="warning">
              The MCP server isn&apos;t next to SayQL. Building from source? Run `cargo build --bin
              sayql-mcp` in src-tauri.
            </Alert>
          )}
          {setup?.exists && setup.appimage && (
            <Alert tone="warning">
              The AppImage opens from a new folder each time, so this path stops working after SayQL
              restarts. Install the .deb or .rpm to use MCP.
            </Alert>
          )}
          <AgentPicker agent={agent} onChoose={choose} />
          {context ? (
            <AgentSetupView key={agent.id} agent={agent} context={context} />
          ) : (
            <p className={styles.detail}>Couldn&apos;t find the MCP server&apos;s path.</p>
          )}
        </div>
      )}
    </section>
  );
}

/** The Access tab: setting up an app (first), which apps connected, and what they get back per connection. */
function AccessTab({
  connections,
  provider,
  onReview,
}: {
  connections: ConnectionConfig[];
  provider: { label: string; local: boolean };
  onReview: () => void;
}) {
  const [setup, setSetup] = useState<McpSetup | null>(null);
  const [apps, setApps] = useState<McpApps | null>(null);
  const [error, setError] = useState<string | null>(null);
  // Open while no app has connected yet (or the server can't be found); closed once one has.
  const [connectOpen, setConnectOpen] = useState<boolean | null>(null);

  useEffect(() => {
    let live = true;
    Promise.all([commands.mcpSetup().catch(() => null), commands.mcpApps()]).then(
      ([s, a]) => {
        if (!live) return;
        setSetup(s);
        setApps(a);
        setConnectOpen((open) => open ?? (a.apps.length === 0 || !s?.exists || !!s.appimage));
      },
      (e) => {
        if (!live) return;
        setApps({ onlyApproved: false, apps: [] });
        setError(errorText(e));
        setConnectOpen((open) => open ?? true);
      },
    );
    return () => {
      live = false;
    };
  }, []);

  const update = (change: Promise<McpApps>) => {
    setError(null);
    change.then(setApps, (e) => setError(errorText(e)));
  };

  const first = !!apps && apps.apps.length === 0;
  return (
    <>
      {connectOpen !== null && (
        <ConnectApp
          setup={setup}
          open={connectOpen}
          first={first}
          onToggle={() => setConnectOpen(!connectOpen)}
        />
      )}
      <AppsList
        list={apps}
        onBlock={(app, blocked) => update(commands.setMcpAppBlocked(app.name, blocked))}
        onApprove={(app) => update(commands.approveMcpApp(app.name))}
        onOnlyApproved={(on) => update(commands.setMcpOnlyApproved(on))}
      />
      {error && <Alert tone="danger">{error}</Alert>}
      <AppAccessList connections={connections} provider={provider} onReview={onReview} />
    </>
  );
}

export interface ClaudeIntegrationProps {
  connections: ConnectionConfig[];
  /** Where agent mode sends results, for the agent settings opened from here. */
  provider: { label: string; local: boolean };
  /** Notes AI apps suggested, waiting; the same list the sidebar and the top bar count. */
  suggestions: NoteSuggestionsApi;
  /** The tab to open on; otherwise the one open last in this session. */
  tab?: AiAppsTab;
}

/** Settings → AI apps: what AI apps over MCP may see and did, and the notes they suggested, in three tabs. */
export function ClaudeIntegration({
  connections,
  provider,
  suggestions,
  tab: initial,
}: ClaudeIntegrationProps) {
  const [tab, setTabState] = useState<AiAppsTab>(initial ?? lastTab);
  const setTab = (t: AiAppsTab) => {
    lastTab = t;
    setTabState(t);
  };
  useEffect(() => {
    if (initial) {
      lastTab = initial;
      setTabState(initial);
    }
  }, [initial]);

  const base = useId();
  const tabRefs = useRef<Record<AiAppsTab, HTMLButtonElement | null>>({
    access: null,
    activity: null,
    suggestions: null,
  });
  // Arrow keys, Home and End move between tabs, as in any tab list.
  const onKeyDown = (e: KeyboardEvent) => {
    const i = TABS.findIndex((t) => t.id === tab);
    const next =
      e.key === "ArrowRight"
        ? (i + 1) % TABS.length
        : e.key === "ArrowLeft"
          ? (i - 1 + TABS.length) % TABS.length
          : e.key === "Home"
            ? 0
            : e.key === "End"
              ? TABS.length - 1
              : -1;
    if (next < 0) return;
    e.preventDefault();
    setTab(TABS[next].id);
    tabRefs.current[TABS[next].id]?.focus();
  };
  const waiting = suggestions.suggestions.length;

  return (
    <div className={styles.section}>
      <p className={styles.intro}>
        Let Claude, Cursor or any other MCP app list your connections, read schemas and run{" "}
        <strong>read-only</strong> queries. You choose, per connection, what it gets back: by
        default counts and summaries, not your rows.
      </p>
      <div className={styles.tabs} role="tablist" aria-label="AI apps" onKeyDown={onKeyDown}>
        {TABS.map((t) => (
          <button
            key={t.id}
            ref={(el) => {
              tabRefs.current[t.id] = el;
            }}
            id={`${base}-${t.id}-tab`}
            type="button"
            role="tab"
            aria-selected={tab === t.id}
            aria-controls={`${base}-${t.id}-panel`}
            tabIndex={tab === t.id ? 0 : -1}
            className={cx(styles.tab, tab === t.id && styles.tabActive)}
            onClick={() => setTab(t.id)}
          >
            {t.label}
            {t.id === "suggestions" && waiting > 0 && (
              <span className={styles.count} aria-label={`${waiting} waiting`}>
                {waiting}
              </span>
            )}
          </button>
        ))}
      </div>
      <div
        id={`${base}-${tab}-panel`}
        role="tabpanel"
        aria-labelledby={`${base}-${tab}-tab`}
        className={styles.panel}
      >
        {tab === "access" && (
          <AccessTab
            connections={connections}
            provider={provider}
            onReview={() => setTab("suggestions")}
          />
        )}
        {tab === "activity" && <AgentActivity />}
        {tab === "suggestions" && (
          <SuggestedNotes
            connections={connections}
            {...suggestions}
            onSetUp={() => setTab("access")}
          />
        )}
      </div>
    </div>
  );
}
