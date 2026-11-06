import { cx } from "../lib/cx";
import {
  MoveConnectionDialog,
  MoveDatabaseDialog,
  NameDialog,
  NewWorkspaceDialog,
  useWorkspaces,
  workspaceStyles,
  WorkspaceRail,
} from "../features/workspaces";
import { useEffect, useRef, useState } from "react";
import { Copy, Database, FileCode2, Plus, Save, Server, Table2, X } from "lucide-react";
import {
  TEAM_SIGNED_OUT,
  commands,
  subscribeToMenu,
  type Bootstrap,
  type ConnectionConfig,
  type DbNote,
  type DbNoteInput,
  type Script,
  type Table,
  type TeamAccount,
  type TeamAiStatus,
  type Target,
} from "../api";
import {
  Alert,
  Button,
  Dialog,
  EmptyState,
  ErrorBoundary,
  Spinner,
  SplitPane,
  TabBar,
  type MenuEntry,
  type TabItem,
} from "../components/ui";
import {
  AgentAccessDialog,
  NoteDialog,
  NoteSuggestions,
  SUGGESTIONS_CHANGED,
  TeachSayql,
  AuditDialog,
  lastAudit,
  saveAgentAccess,
  suggestionFor,
  useAgentAccess,
  useAudits,
  useNoteSuggestions,
} from "../features/agent";
import { useLatest } from "../hooks/useLatest";
import { usePersistentState } from "../hooks/usePersistentState";
import {
  ConnectionDialog,
  DATABASE_KINDS,
  ProdBadge,
  newConnection,
  targetKey,
  useConnections,
} from "../features/connections";
import { DatabaseNavigator, type NavigatorScript } from "../features/navigator";
import { CloseTabDialog, CloseTabsDialog, SaveScriptDialog, useScripts } from "../features/scripts";
import { selectAllSql } from "../features/schema";
import {
  SettingsPage,
  lowCreditText,
  useSettings,
  type AiAppsTab,
  type SettingsSection,
} from "../features/settings";
import { useSkills } from "../features/skills";
import { hasUnsavedChanges, isPristine, useTabs, type EditorTab } from "../features/tabs";
import { UpdateDialog, useUpdate } from "../features/updates";
import { FeedbackDialog } from "../features/feedback";
import { errorText } from "../lib/errors";
import { useHandoffs } from "../features/handoff";
import {
  TeamSignInDialog,
  teamAiDestination,
  teamAiOn,
  useTeamConnections,
} from "../features/team";
import {
  exampleQuestions,
  GetStarted,
  OnboardingDialog,
  useGetStarted,
  type OnboardingStep,
} from "../features/onboarding";
import {
  DOCS_URL,
  ShortcutSheet,
  WhatsNewDialog,
  isTypingTarget,
  whatsNewOnStart,
} from "../features/help";
import { openUrl } from "@tauri-apps/plugin-opener";
import { version } from "../../package.json";
import { QueryTab } from "./QueryTab";
import { StatusBar } from "./StatusBar";
import { ConnectionOverview } from "./ConnectionOverview";
import { TopBar } from "./TopBar";
import { useConnectionActions } from "./useConnectionActions";
import { useScriptActions } from "./useScriptActions";
import styles from "./Workbench.module.css";

type OpenDialog =
  /** `fromSetup`: opened from the first-run setup, which resumes when it closes. */
  /** `note`: where the details came from, for a handoff from an AI app. */
  | {
      kind: "connection";
      config: ConnectionConfig;
      fromSetup?: boolean;
      note?: string;
      key?: string;
      workspaceId?: string | null;
      groupId?: string | null;
    }
  /** `createSkill`: opened from "Create skill…" in the Ask box. */
  | { kind: "settings"; section?: SettingsSection; aiAppsTab?: AiAppsTab; createSkill?: boolean }
  | { kind: "saveScript"; tabId: string; closeAfter: boolean }
  | { kind: "closeTab"; tabId: string }
  | { kind: "closeTabs"; tabIds: string[] }
  | { kind: "deleteConnection"; connection: ConnectionConfig }
  | { kind: "feedback" }
  | { kind: "deleteScript"; script: Script }
  | { kind: "agentAccess"; connection: ConnectionConfig }
  | { kind: "overview"; connection: ConnectionConfig }
  | { kind: "note"; note: DbNote | null }
  | { kind: "noteSuggestions" }
  /** "Teach SayQL your database", from the navigator. */
  | { kind: "teach" }
  /** "Run audit", from "Teach SayQL your database". */
  | { kind: "audit" }
  /** From a sayql://team-join invite link. */
  | { kind: "teamSignIn"; server: string; code: string; key: string }
  | { kind: "newGroup" }
  | { kind: "renameGroup"; id: string; name: string }
  | { kind: "newWorkspace" }
  | { kind: "moveConnection"; connection: ConnectionConfig }
  | { kind: "moveDatabase"; connection: ConnectionConfig; database: string }
  | null;

/** The main window. Owns no logic of its own: it wires the feature hooks to the feature views. */
export function Workbench({ boot }: { boot: Bootstrap }) {
  const workspaces = useWorkspaces(boot.layout);
  const everyConnection = useConnections(boot.connections);
  // Lists show the open workspace's connections. Looking one up by id still finds any of them:
  // tabs in other workspaces stay open and keep running.
  const conn = {
    ...everyConnection,
    connections: everyConnection.connections.filter(
      (c) => workspaces.workspaceOf(c.id) === workspaces.activeId,
    ),
  };
  const tabs = useTabs({
    id: workspaces.activeId,
    workspaceOf: workspaces.workspaceOf,
    reveal: workspaces.reveal,
  });
  const scripts = useScripts();
  const { scriptOf, saveTab, openScript, deleteScript } = useScriptActions(scripts, tabs);
  const connActions = useConnectionActions(conn, tabs);
  const skills = useSkills();
  const { settings, setSettings, modelNames } = useSettings(boot.settings);
  const updates = useUpdate();
  const [dialog, setDialog] = useState<OpenDialog>(null);
  const [notice, setNotice] = useState<string | null>(null);
  // First launch: the setup shows until it's finished or dismissed. People who already
  // have an AI provider working (from before the setup existed) never see it.
  const [setupStep, setSetupStep] = useState<OnboardingStep | null>(() =>
    !boot.settings.onboarded && !boot.settings.aiReady ? "welcome" : null,
  );

  // Help: the shortcut sheet, and What's new, once after an update (not for someone new).
  const [whatsNew] = useState(() =>
    whatsNewOnStart(version, boot.settings.onboarded || boot.settings.aiReady),
  );
  const [help, setHelp] = useState<"shortcuts" | "whatsNew" | null>(() =>
    whatsNew.show ? "whatsNew" : null,
  );
  const closeHelp = () => {
    if (help === "whatsNew") whatsNew.markSeen();
    setHelp(null);
  };

  /** Tabs with an agent run going, which show a spinner. */
  const [busyTabs, setBusyTabs] = useState<Set<string>>(new Set());
  const setBusy = (id: string, busy: boolean) =>
    setBusyTabs((s) => {
      if (s.has(id) === busy) return s;
      const next = new Set(s);
      if (busy) next.add(id);
      else next.delete(id);
      return next;
    });
  /** Bumped when a note changes, so the navigator lists it. */
  const [notesVersion, setNotesVersion] = useState(0);

  // The navigator follows the active tab, like DBeaver's "link with editor".
  const target = tabs.active?.target ?? null;
  const active = target ? conn.connection(target.connectionId) : null;
  const { schema, loading, error } = conn.schemaState(target);
  const databaseName = conn.databaseName(target);
  const [notes, setNotes] = useState<DbNote[]>([]);
  /** The notes are the team's, from the team server: read-only here, and new ones are suggested. */
  const [sharedNotes, setSharedNotes] = useState(false);
  useEffect(() => {
    if (!target) {
      setSharedNotes(false);
      return setNotes([]);
    }
    let current = true;
    const local = () =>
      commands.listNotes(target).then(
        (n) => current && (setNotes(n), setSharedNotes(false)),
        () => current && (setNotes([]), setSharedNotes(false)),
      );
    // A team connection whose server keeps shared notes uses those; anything else the notes saved here.
    if (target.connectionId.startsWith("team:")) {
      commands.listSharedNotes(target.connectionId).then(
        (shared) => (shared ? current && (setNotes(shared), setSharedNotes(true)) : local()),
        () => local(),
      );
    } else void local();
    return () => {
      current = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [targetKey(target), notesVersion]);
  // Reading a schema reviews the notes against it, so list them again after each read.
  useEffect(() => {
    if (schema) setNotesVersion((v) => v + 1);
  }, [schema]);
  // Notes something has made doubtful; their banner can be put off per database, for the session.
  // The team's doubtful notes are for its owners to review on the team server.
  const doubtful = sharedNotes ? [] : notes.filter((n) => n.suspect);
  const doubtKey = `${target ? targetKey(target) : ""}:${doubtful.map((n) => n.id).join(",")}`;
  const [hiddenDoubts, setHiddenDoubts] = useState<string[]>([]);
  // Notes AI apps suggested over MCP; an accepted one (here or in Settings) is a new note.
  const noteSuggestions = useNoteSuggestions();
  const suggestionsHere = noteSuggestions.suggestions.filter((s) =>
    suggestionFor(s, target, target && conn.connection(target.connectionId)?.database),
  );
  useEffect(() => {
    const bump = () => setNotesVersion((v) => v + 1);
    window.addEventListener(SUGGESTIONS_CHANGED, bump);
    return () => window.removeEventListener(SUGGESTIONS_CHANGED, bump);
  }, []);
  // "Teach SayQL your database" shows under the open database until it's hidden or an AI app
  // has audited the connection. MCP serves the connections saved here, not a team's.
  const [teachHidden, setTeachHidden] = usePersistentState("sayql.teachHidden", false);
  const audits = useAudits();
  const teachHere =
    !!active &&
    !active.id.startsWith("team:") &&
    !teachHidden &&
    lastAudit(audits, active, target?.database) === null;
  const agentAccessConnection = dialog?.kind === "agentAccess" ? dialog.connection.id : null;
  const dialogAccess = useAgentAccess(agentAccessConnection);
  // Get started: a checklist under the navigator until it's done or hidden.
  const openDemo = () => {
    const demo = conn.connection("demo");
    if (demo) openConnection(demo);
  };
  const getStarted = useGetStarted(
    {
      aiReady: settings.aiReady,
      askedNow: tabs.tabs.some((t) => !!t.lastRun || !!t.agentRun),
      ownDatabase: everyConnection.connections.some((c) => c.id !== "demo"),
      taught: audits.length > 0 || notes.length > 0,
    },
    {
      setUpAi: () => setSetupStep("ai"),
      tryIt: () => {
        if (!active) openDemo();
        setSetupStep("try");
      },
      connectDatabase: () => openNewConnection(),
      connectApp: () => openSettings({ section: "mcp", aiAppsTab: "access" }),
      teach: () => {
        if (!active || active.id.startsWith("team:")) openDemo();
        setDialog({ kind: "teach" });
      },
    },
  );
  const currentProvider = settings.aiReady
    ? settings.providers.find((p) => p.id === settings.provider)
    : undefined;
  // On SayQL AI credits: warn once per launch when the balance is low, before it runs out mid-work.
  const onCredits = settings.aiReady && settings.provider === "sayql";
  useEffect(() => {
    if (!onCredits) return;
    commands.creditsBalance().then(
      (usd) => {
        const low = usd == null ? null : lowCreditText(usd);
        if (low) setNotice(low);
      },
      () => {},
    );
  }, [onCredits]);
  /** Where the Ask box and Investigate send for a connection: the team's AI on a team connection with team AI on. */
  const destinationFor = (connectionId: string) => {
    const ai = teamAiFor(connectionId);
    if (ai) return teamAiDestination(ai);
    return currentProvider ? { label: currentProvider.label, local: currentProvider.local } : null;
  };

  // SayQL Team: the team's shared connections, fetched only while signed in.
  const setTeam = (team: TeamAccount | null) => setSettings((s) => ({ ...s, team }));
  /** Team AI: the company's AI for team connections; null when off or signed out. */
  const [teamAi, setTeamAi] = useState<TeamAiStatus | null>(null);
  useTeamConnections(
    settings.team ?? null,
    conn.setTeam,
    (e) => {
      if (e === TEAM_SIGNED_OUT) setTeam(null);
      setNotice(`Couldn't load your team's connections: ${e}`);
    },
    setTeamAi,
    (lost) => {
      const names = lost.map((c) => c.name).join(", ");
      setNotice(`You no longer have access to ${names}. Ask your team admin.`);
      const ids = new Set(lost.map((c) => c.id));
      // A tab with SQL that isn't saved stays, so the person can copy it.
      tabs.closeWhere((t) => ids.has(t.target.connectionId) && !hasUnsavedChanges(t));
    },
  );
  /** Team AI for a connection: only team connections use it. */
  const teamAiFor = (connectionId: string) =>
    conn.connection(connectionId)?.team && teamAiOn(teamAi) ? teamAi : null;
  // A query or schema load that found the team sign-in revoked: show the team as signed out.
  useEffect(() => {
    if (error?.includes(TEAM_SIGNED_OUT)) setTeam(null);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [error]);

  // On start: drop restored tabs whose connection no longer exists. Nothing connects on its
  // own, not even the first connection: restored tabs wait for the person to click the
  // connection, Connect, Run or Ask (a production database is never opened silently).
  // Tabs on team connections stay while signed in: those connections load from the server.
  const started = useRef(false);
  useEffect(() => {
    if (started.current) return;
    started.current = true;
    const teamTab = (id: string) => !!boot.settings.team && id.startsWith("team:");
    tabs.closeWhere(
      (t) => !conn.connection(t.target.connectionId) && !teamTab(t.target.connectionId),
    );
    // Only on start.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Load the schema of whatever tab is showing, once its connection is connected (also after
  // the connection was edited).
  const targetConnected = !!target && conn.isConnected(target.connectionId);
  useEffect(() => {
    if (target && targetConnected) conn.ensure(target);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [targetKey(target), targetConnected, conn.connections]);

  /** Focuses a tab already open on `t`, or opens a new one, and connects (the person asked for it). */
  const showTarget = (t: Target) => {
    const name = conn.databaseName(t);
    workspaces.reveal(t.connectionId);
    const existing = tabs.tabs.find(
      (x) =>
        x.target.connectionId === t.connectionId &&
        conn.databaseName(x.target) === name &&
        !x.title,
    );
    if (existing) tabs.activate(existing.id);
    else tabs.open(t);
    conn.connect(existing?.target ?? t);
  };

  const openConnection = (c: ConnectionConfig) =>
    showTarget({ connectionId: c.id, database: c.database.trim() || null });

  const newTab = () => {
    if (target) tabs.open(target);
    else if (conn.connections[0]) openConnection(conn.connections[0]);
  };

  const tabById = (id: string) => tabs.tabs.find((t) => t.id === id) ?? null;

  /** Saves straight away when the tab already is a script; otherwise asks for a name first. */
  const requestSave = (t: EditorTab, closeAfter = false) => {
    if (!t.sql.trim()) return;
    const ask = () => setDialog({ kind: "saveScript", tabId: t.id, closeAfter });
    if (!scriptOf(t)) return ask();
    saveTab(t).then(() => closeAfter && tabs.close(t.id), ask);
  };

  /** Closes several tabs; asks first when some have SQL that isn't saved. */
  const requestCloseMany = (ids: string[]) => {
    if (ids.length === 1) return requestClose(ids[0]);
    const closing = new Set(ids);
    if (tabs.tabs.some((t) => closing.has(t.id) && hasUnsavedChanges(t)))
      setDialog({ kind: "closeTabs", tabIds: ids });
    else tabs.closeWhere((t) => closing.has(t.id));
  };

  /** A tab's right-click menu, like an editor's. */
  const tabMenu = (id: string): MenuEntry[] => {
    const all = tabs.here;
    const i = all.findIndex((t) => t.id === id);
    const t = all[i];
    if (!t) return [];
    const ids = (list: EditorTab[]) => list.map((x) => x.id);
    const others = all.filter((x) => x.id !== id);
    const right = all.slice(i + 1);
    const left = all.slice(0, i);
    const saved = all.filter((x) => !hasUnsavedChanges(x));
    return [
      { label: "Close", icon: X, onSelect: () => requestClose(id) },
      {
        label: "Close others",
        disabled: !others.length,
        onSelect: () => requestCloseMany(ids(others)),
      },
      {
        label: "Close to the right",
        disabled: !right.length,
        onSelect: () => requestCloseMany(ids(right)),
      },
      {
        label: "Close to the left",
        disabled: !left.length,
        onSelect: () => requestCloseMany(ids(left)),
      },
      {
        label: "Close saved tabs",
        disabled: !saved.length,
        onSelect: () => requestCloseMany(ids(saved)),
      },
      { label: "Close all", onSelect: () => requestCloseMany(ids(all)) },
      "separator",
      {
        label: "Duplicate",
        icon: Copy,
        onSelect: () => tabs.open(t.target, { title: t.title, question: t.question, sql: t.sql }),
      },
      { label: "New tab on this database", icon: Plus, onSelect: () => tabs.open(t.target) },
      {
        label: scriptOf(t) ? "Save" : "Save as script…",
        icon: Save,
        shortcut: "⌘S",
        disabled: !t.sql.trim(),
        onSelect: () => requestSave(t),
      },
    ];
  };

  /** Closes the tab, first asking to save when that would lose SQL. */
  const requestClose = (id: string) => {
    const t = tabById(id);
    if (t && hasUnsavedChanges(t)) setDialog({ kind: "closeTab", tabId: id });
    else tabs.close(id);
  };

  /** ⌘W: closes the active tab, asking first when that would lose SQL. */
  const closeActiveTab = () => {
    if (tabs.active && !dialog) requestClose(tabs.active.id);
  };

  // ⌘T / Ctrl+T opens a new tab on the current database; ⌘S / Ctrl+S saves the active tab;
  // ⌘W closes it; ⌘, opens Settings; ? (outside a text field) lists the shortcuts. On macOS
  // the menu bar owns ⌘W and ⌘, and sends them as menu actions (below). The listener is added
  // once and reads the latest state through a ref.
  const onKey = useLatest((e: KeyboardEvent) => {
    if (
      e.key === "?" &&
      !e.metaKey &&
      !e.ctrlKey &&
      !e.altKey &&
      !isTypingTarget(e.target) &&
      !dialog &&
      !setupStep
    ) {
      e.preventDefault();
      setHelp("shortcuts");
      return;
    }
    if (!(e.metaKey || e.ctrlKey) || e.altKey) return;
    const key = e.key.toLowerCase();
    if (key === "t") {
      e.preventDefault();
      newTab();
    } else if (key === "s" && tabs.active) {
      e.preventDefault();
      requestSave(tabs.active);
    } else if (key === "w" && !e.shiftKey) {
      e.preventDefault();
      closeActiveTab();
    } else if (key === ",") {
      e.preventDefault();
      if (dialog?.kind !== "settings") openSettings();
    }
  });
  useEffect(() => {
    const listener = (e: KeyboardEvent) => onKey.current(e);
    window.addEventListener("keydown", listener);
    return () => window.removeEventListener("keydown", listener);
  }, [onKey]);

  // The macOS menu bar: Close Tab, Settings…, and the Help menu.
  const onMenu = useRef<(a: string) => void>(() => {});
  onMenu.current = (action) => {
    if (action === "close-tab") closeActiveTab();
    else if (action === "settings") {
      if (dialog?.kind !== "settings") openSettings();
    } else if (action === "shortcuts") setHelp("shortcuts");
    else if (action === "whats-new") setHelp("whatsNew");
    else if (action === "docs") void openUrl(DOCS_URL);
  };
  useEffect(() => {
    const off = subscribeToMenu((a) => onMenu.current(a));
    return () => void off.then((f) => f());
  }, []);

  const previewTable = (t: Table) => {
    if (!target || !schema) return;
    const sql = selectAllSql(t, schema.dialect);
    tabs.open(target, { title: t.name, sql, savedSql: sql, autoRun: true });
  };

  /** SQL from the navigator in a new tab on the open database; only read-only SQL is run straight away. */
  const openSql = (title: string, sql: string, run: boolean) => {
    if (target) tabs.open(target, { title, sql, savedSql: sql, autoRun: run });
  };

  const tabLabel = (t: EditorTab) => {
    const c = conn.connection(t.target.connectionId);
    const db = c?.kind === "sqlite" ? null : conn.databaseName(t.target);
    return { name: c?.name ?? "Closed connection", db };
  };

  const tabItems: TabItem[] = tabs.here.map((t) => {
    const { name, db } = tabLabel(t);
    const c = conn.connection(t.target.connectionId);
    // Named tabs are table previews, unless they are saved scripts or SQL from an AI app.
    const Icon = t.title && !t.scriptId && !t.origin ? Table2 : FileCode2;
    return {
      id: t.id,
      label: t.title || db || name,
      detail: t.title ? (db ?? name) : db ? name : undefined,
      title:
        [name, db, t.title].filter(Boolean).join(" / ") +
        (conn.isConnected(t.target.connectionId) ? "" : " (not connected)"),
      icon: busyTabs.has(t.id) ? (
        <Spinner size={13} />
      ) : (
        <Icon size={14} color={c ? DATABASE_KINDS[c.kind].color : undefined} />
      ),
      dirty: hasUnsavedChanges(t),
      badge: <ProdBadge connection={c} />,
      // Not connected yet: shown dimmed until the person connects.
      muted: !conn.isConnected(t.target.connectionId),
    };
  });

  const navigatorScripts: NavigatorScript[] = scripts.scripts
    // Scripts follow their connection; one whose connection is gone shows in Personal.
    .filter((s) => workspaces.workspaceOf(s.connectionId) === workspaces.activeId)
    .map((s) => {
      const c = conn.connection(s.connectionId);
      return {
        id: s.id,
        name: s.name,
        location: c?.kind === "sqlite" ? c.name : (s.database ?? c?.database ?? c?.name ?? ""),
        orphaned: !c,
        // With no database open, "This database" lists them all.
        here:
          !target ||
          (!!c &&
            target.connectionId === c.id &&
            (s.database || c.database || null) === (target.database || c.database || null)),
      };
    });

  /** A name to suggest when a tab is saved for the first time. */
  const suggestedName = (t: EditorTab) =>
    t.title || t.question.trim().slice(0, 60) || `${tabLabel(t).db ?? tabLabel(t).name} query`;

  const openSettings = (at?: { section: SettingsSection; aiAppsTab?: AiAppsTab }) => {
    setDialog({ kind: "settings", ...at });
    // Team AI's limits and usage may have changed since: ask again (only when signed in).
    if (settings.team) commands.teamAi().then(setTeamAi, () => {});
  };

  const finishSetup = () => {
    setSetupStep(null);
    commands.finishOnboarding().then(setSettings, () => {});
  };

  /**
   * Ends the setup with a tab that asks `question` and runs the answer: the tab the setup just
   * opened, while it's still empty, so the demo doesn't end up in two identical tabs.
   */
  const trySetupQuestion = (question: string) => {
    finishSetup();
    if (!target) return;
    const t = tabs.active;
    if (t && isPristine(t)) tabs.update(t.id, { question, autoAsk: true });
    else tabs.open(target, { question, autoAsk: true });
  };
  const openNewConnection = () => setDialog({ kind: "connection", config: newConnection() });

  /** Where the connection dialog saves to: the asking AI app's workspace, else the open one. */
  const connectionPlacement = (d: {
    config: ConnectionConfig;
    workspaceId?: string | null;
    groupId?: string | null;
  }) => {
    const known = (id?: string | null) => workspaces.workspaces.find((w) => w.id === id);
    const workspace = known(d.workspaceId) ?? workspaces.workspace;
    if (!workspace) return undefined;
    return {
      workspaceId: workspace.id,
      workspaceName: workspace.name,
      groupId: d.groupId ?? null,
      allowed: d.config.id ? workspaces.allowedOf(d.config.id) : null,
      sharedWith: (host: string, id: string) => {
        if (!host.trim()) return null;
        const other = everyConnection.connections.find(
          (c) =>
            c.id !== id &&
            c.host.toLowerCase() === host.trim().toLowerCase() &&
            workspaces.workspaceOf(c.id) !== workspace.id,
        );
        return other
          ? (workspaces.workspaces.find((w) => w.id === workspaces.workspaceOf(other.id))?.name ??
              null)
          : null;
      },
    };
  };

  // AI apps hand over a connection to finish or SQL to review; nothing runs or saves on its own.
  useHandoffs(
    {
      connection: conn.connection,
      openConnectionDialog: (config, note, place) =>
        setDialog({ kind: "connection", config, note, key: crypto.randomUUID(), ...place }),
      openTab: (t, init) => tabs.open(t, init),
      openScript,
      openTeamSignIn: (server, code) =>
        setDialog({ kind: "teamSignIn", server, code, key: crypto.randomUUID() }),
      finishTeamSso: (code) =>
        commands.teamSsoFinish(code).then(
          (account) => {
            setDialog((d) => (d?.kind === "teamSignIn" ? null : d));
            setTeam(account);
            setNotice(`Signed in to your team as ${account.email}.`);
          },
          (e) => setNotice(`Couldn't sign in to your team: ${errorText(e)}`),
        ),
      notify: setNotice,
    },
    scripts.loaded,
  );

  return (
    <div className={styles.workbench}>
      <TopBar
        active={active}
        databaseName={databaseName}
        schema={schema}
        onNewConnection={openNewConnection}
        pendingSuggestions={noteSuggestions.suggestions.length}
        workspaces={workspaces}
        // With suggested notes waiting, the gear opens them: Settings → AI apps → Suggestions.
        onOpenSettings={() =>
          openSettings(
            noteSuggestions.suggestions.length > 0
              ? { section: "mcp", aiAppsTab: "suggestions" }
              : undefined,
          )
        }
      />

      {notice && (
        <div className={styles.banner}>
          <Alert
            tone="warning"
            actions={
              <Button size="sm" variant="ghost" onClick={() => setNotice(null)}>
                Dismiss
              </Button>
            }
          >
            {notice}
          </Alert>
        </div>
      )}

      {doubtful.length > 0 && target && !hiddenDoubts.includes(doubtKey) && (
        <div className={styles.banner}>
          <Alert
            tone="warning"
            actions={
              <>
                <Button size="sm" onClick={() => setDialog({ kind: "note", note: doubtful[0] })}>
                  Review
                </Button>
                <Button
                  size="sm"
                  variant="ghost"
                  onClick={() => setHiddenDoubts((h) => [...h, doubtKey])}
                >
                  Not now
                </Button>
              </>
            }
          >
            {doubtful.length === 1 ? "1 note" : `${doubtful.length} notes`} about{" "}
            {databaseName ?? "this database"} may be out of date: {doubtful[0].suspect?.reason}
            {doubtful.length > 1 ? ", and more" : ""}. The AI is told to check them before relying
            on them.
          </Alert>
        </div>
      )}

      {error && target && (
        <div className={styles.banner}>
          <Alert
            tone="danger"
            actions={
              <Button size="sm" variant="ghost" onClick={() => conn.dismissError(target)}>
                Dismiss
              </Button>
            }
          >
            {error}
          </Alert>
        </div>
      )}
      {connActions.error && (
        <div className={styles.banner}>
          <Alert
            tone="danger"
            actions={
              <Button size="sm" variant="ghost" onClick={connActions.dismissError}>
                Dismiss
              </Button>
            }
          >
            {connActions.error}
          </Alert>
        </div>
      )}

      <main className={cx(styles.body, workspaceStyles.withRail)}>
        <WorkspaceRail
          workspaces={workspaces}
          onConnectionsDeleted={(ids) => ids.forEach(connActions.forget)}
        />
        <div className={workspaceStyles.besideRail}>
          <SplitPane
            direction="horizontal"
            persistKey="navigator"
            initialSize={280}
            minSize={200}
            maxSize={560}
            first={
              <div className={styles.sideColumn}>
                <DatabaseNavigator
                  groups={workspaces.groupsHere}
                  groupOf={(id) => workspaces.groupOf(id)?.id ?? null}
                  databasesIn={workspaces.databasesIn}
                  isPlacedApart={workspaces.isPlacedApart}
                  onOpenAt={(connectionId, database) => showTarget({ connectionId, database })}
                  onNewGroup={() => setDialog({ kind: "newGroup" })}
                  onRenameGroup={(id, name) => setDialog({ kind: "renameGroup", id, name })}
                  onDeleteGroup={(id) =>
                    void workspaces.deleteGroup(id).catch((e) => setNotice(errorText(e)))
                  }
                  teamSignedIn={!!settings.team}
                  onNewWorkspace={
                    workspaces.several ? undefined : () => setDialog({ kind: "newWorkspace" })
                  }
                  onMove={(connection) => setDialog({ kind: "moveConnection", connection })}
                  onMoveDatabase={(connection, database) =>
                    setDialog({ kind: "moveDatabase", connection, database })
                  }
                  connections={conn.connections}
                  activeId={active?.id ?? null}
                  databases={conn.databases}
                  databaseName={databaseName}
                  schema={schema}
                  loading={loading}
                  connected={targetConnected}
                  onConnect={() => target && conn.connect(target)}
                  onOpen={(id) => {
                    const c = conn.connection(id);
                    if (c && id !== active?.id) openConnection(c);
                    else if (target) conn.connect(target);
                  }}
                  onOpenDatabase={(database) =>
                    active && showTarget({ connectionId: active.id, database })
                  }
                  onAdd={openNewConnection}
                  onEdit={(c) => setDialog({ kind: "connection", config: c })}
                  onDeleteConnection={(c) => setDialog({ kind: "deleteConnection", connection: c })}
                  onRefresh={() => target && conn.ensure(target, true)}
                  onPreviewTable={previewTable}
                  onNewTab={(connectionId, database) => {
                    const c = conn.connection(connectionId);
                    if (c)
                      tabs.open({
                        connectionId,
                        database: database ?? (c.database.trim() || null),
                      });
                  }}
                  onOpenSql={openSql}
                  selectedTables={tabs.active?.focusTables ?? []}
                  // A new pick is meant to be used, so it switches a hidden selection back on.
                  onSelectTables={(focusTables) =>
                    tabs.active && tabs.update(tabs.active.id, { focusTables, focusEnabled: true })
                  }
                  scripts={navigatorScripts}
                  onOpenScript={openScript}
                  onDeleteScript={(id) => {
                    const s = scripts.scripts.find((x) => x.id === id);
                    if (s) setDialog({ kind: "deleteScript", script: s });
                  }}
                  notes={notes}
                  sharedNotes={sharedNotes}
                  onEditNote={(note) =>
                    setDialog({ kind: "note", note: sharedNotes ? null : note })
                  }
                  onDeleteNote={(note) =>
                    void commands.deleteNote(note.id).then(() => setNotesVersion((v) => v + 1))
                  }
                  onCheckNotes={() => {
                    if (!target) return;
                    void commands.checkNotes(target).then(
                      (n) => {
                        setNotes(n);
                        conn.ensure(target, true);
                      },
                      (e) => setNotice(String(e)),
                    );
                  }}
                  suggestedNotes={suggestionsHere.length}
                  onReviewSuggestions={() => setDialog({ kind: "noteSuggestions" })}
                  onTeach={teachHere ? () => setDialog({ kind: "teach" }) : undefined}
                  onHideTeach={() => setTeachHidden(true)}
                  onAgentSettings={(connection) => setDialog({ kind: "agentAccess", connection })}
                  onOverview={(connection) => setDialog({ kind: "overview", connection })}
                  onTeamSignIn={
                    settings.team
                      ? undefined
                      : () =>
                          setDialog({
                            kind: "teamSignIn",
                            server: "",
                            code: "",
                            key: crypto.randomUUID(),
                          })
                  }
                />
                {!getStarted.hidden && !setupStep && (
                  <GetStarted items={getStarted.items} onHide={getStarted.hide} />
                )}
              </div>
            }
            second={
              <div className={styles.editorArea}>
                <TabBar
                  items={tabItems}
                  activeId={tabs.active?.id ?? null}
                  onSelect={tabs.activate}
                  onClose={requestClose}
                  menu={tabMenu}
                  onAdd={conn.connections.length ? newTab : undefined}
                  addLabel="New SQL tab (⌘T)"
                />
                <div className={styles.tabBody}>
                  {tabs.tabs.map((t) => {
                    const { name, db } = tabLabel(t);
                    return (
                      <ErrorBoundary key={t.id} label="This tab" hidden={t.id !== tabs.active?.id}>
                        <QueryTab
                          tab={t}
                          active={t.id === tabs.active?.id}
                          label={db ? `${name} · ${db}` : name}
                          schema={conn.schemaState(t.target).schema}
                          connected={conn.isConnected(t.target.connectionId)}
                          production={!!conn.connection(t.target.connectionId)?.production}
                          onConnect={() => conn.connect(t.target)}
                          settings={settings}
                          modelNames={modelNames}
                          onChange={(patch) => tabs.update(t.id, patch)}
                          onOpenSettings={() => openSettings()}
                          skills={skills.skills}
                          onCreateSkill={() =>
                            setDialog({ kind: "settings", section: "skills", createSkill: true })
                          }
                          onSave={() => requestSave(t)}
                          onOpenQuery={(sql, question) => tabs.open(t.target, { sql, question })}
                          onSaveAsScript={(sql, question) => {
                            const id = tabs.open(t.target, { sql, question });
                            setDialog({ kind: "saveScript", tabId: id, closeAfter: false });
                          }}
                          onAgentBusy={(busy) => setBusy(t.id, busy)}
                          onNotesChanged={() => setNotesVersion((v) => v + 1)}
                          onSettingsChange={setSettings}
                          onNotice={setNotice}
                          teamAi={teamAiFor(t.target.connectionId)}
                        />
                      </ErrorBoundary>
                    );
                  })}
                  {tabs.here.length === 0 && (
                    <EmptyState
                      icon={conn.connections.length ? Database : Server}
                      title={conn.connections.length ? "No tabs open" : "No connections yet"}
                      action={
                        conn.connections.length ? (
                          <Button variant="primary" onClick={newTab}>
                            New SQL tab
                          </Button>
                        ) : (
                          <Button variant="primary" onClick={openNewConnection}>
                            New connection
                          </Button>
                        )
                      }
                    >
                      {conn.connections.length
                        ? "Pick a database in the Database Navigator, or open a new tab."
                        : "Add a connection to get started."}
                    </EmptyState>
                  )}
                </div>
              </div>
            }
          />
        </div>
      </main>

      <StatusBar
        active={active}
        databaseName={databaseName}
        schema={schema}
        loading={loading}
        connected={targetConnected}
        onConnect={() => target && conn.connect(target)}
        modelNames={modelNames}
        aiReady={settings.aiReady}
        updates={updates}
        onFeedback={() => setDialog({ kind: "feedback" })}
        provider={active ? destinationFor(active.id) : null}
        onPrivacy={() => active && setDialog({ kind: "agentAccess", connection: active })}
        aiScope={
          workspaces.several && workspaces.workspace
            ? {
                workspaceName: workspaces.workspace.name,
                onOpen: () => openSettings({ section: "mcp", aiAppsTab: "access" }),
              }
            : undefined
        }
      />

      {dialog?.kind === "overview" && (
        <ConnectionOverview
          connection={dialog.connection}
          connections={conn.connections}
          schema={dialog.connection.id === active?.id && targetConnected ? schema : null}
          notes={dialog.connection.id === active?.id ? notes.length : null}
          database={dialog.connection.id === active?.id ? (target?.database ?? null) : null}
          provider={destinationFor(dialog.connection.id)}
          onClose={() => setDialog(null)}
          onEdit={
            dialog.connection.team
              ? undefined
              : () => setDialog({ kind: "connection", config: dialog.connection })
          }
          onConnect={() => {
            const c = dialog.connection;
            if (c.id !== active?.id) openConnection(c);
            else if (target) conn.connect(target);
          }}
          onAccess={() => setDialog({ kind: "agentAccess", connection: dialog.connection })}
          onOpenSql={(connectionId, database, sql) => {
            setDialog(null);
            tabs.open({ connectionId, database }, { sql });
          }}
        />
      )}
      {dialog?.kind === "connection" && (
        <ConnectionDialog
          // A new handoff replaces the form rather than keeping the old one's edits.
          key={dialog.key}
          initial={dialog.config}
          note={dialog.note}
          placement={connectionPlacement(dialog)}
          onClose={() => {
            setDialog(null);
            if (dialog.fromSetup) setSetupStep("database");
          }}
          onSaved={(c) => {
            setDialog(null);
            conn.upsert(c);
            void workspaces.refresh();
            openConnection(c);
            if (dialog.fromSetup) setSetupStep("try");
          }}
          onDeleted={(id) => {
            setDialog(null);
            connActions.forget(id);
          }}
        />
      )}
      {dialog?.kind === "newGroup" && (
        <NameDialog
          title={`New group in ${workspaces.workspace?.name ?? "this workspace"}`}
          hint="Databases that belong together, such as one client project."
          action="Create"
          onSubmit={workspaces.createGroup}
          onClose={() => setDialog(null)}
        />
      )}
      {dialog?.kind === "renameGroup" && (
        <NameDialog
          title={`Rename ${dialog.name}`}
          initial={dialog.name}
          action="Rename"
          onSubmit={(name) => workspaces.renameGroup(dialog.id, name)}
          onClose={() => setDialog(null)}
        />
      )}
      {dialog?.kind === "newWorkspace" && (
        <NewWorkspaceDialog workspaces={workspaces} onClose={() => setDialog(null)} />
      )}
      {dialog?.kind === "moveConnection" && (
        <MoveConnectionDialog
          workspaces={workspaces}
          connection={dialog.connection}
          onClose={() => setDialog(null)}
        />
      )}
      {dialog?.kind === "moveDatabase" && (
        <MoveDatabaseDialog
          workspaces={workspaces}
          connection={dialog.connection}
          database={dialog.database}
          onClose={() => setDialog(null)}
        />
      )}
      {dialog?.kind === "saveScript" && tabById(dialog.tabId) && (
        <SaveScriptDialog
          initialName={suggestedName(tabById(dialog.tabId)!)}
          onCancel={() => setDialog(null)}
          onSave={async (name) => {
            await saveTab(tabById(dialog.tabId)!, name);
            if (dialog.closeAfter) tabs.close(dialog.tabId);
            setDialog(null);
          }}
        />
      )}
      {dialog?.kind === "closeTab" && tabById(dialog.tabId) && (
        <CloseTabDialog
          tabName={tabItems.find((i) => i.id === dialog.tabId)?.label ?? "this tab"}
          onCancel={() => setDialog(null)}
          onDiscard={() => {
            tabs.close(dialog.tabId);
            setDialog(null);
          }}
          onSave={() => {
            setDialog(null);
            requestSave(tabById(dialog.tabId)!, true);
          }}
        />
      )}
      {dialog?.kind === "closeTabs" &&
        (() => {
          const closing = new Set(dialog.tabIds);
          const unsaved = tabs.tabs.filter((t) => closing.has(t.id) && hasUnsavedChanges(t));
          return (
            <CloseTabsDialog
              count={tabs.tabs.filter((t) => closing.has(t.id)).length}
              unsaved={unsaved.map((t) => tabItems.find((i) => i.id === t.id)?.label ?? "Untitled")}
              onCancel={() => setDialog(null)}
              onDiscard={() => {
                tabs.closeWhere((t) => closing.has(t.id));
                setDialog(null);
              }}
              onKeepUnsaved={() => {
                tabs.closeWhere((t) => closing.has(t.id) && !hasUnsavedChanges(t));
                setDialog(null);
              }}
            />
          );
        })()}
      {dialog?.kind === "agentAccess" && dialogAccess && (
        <AgentAccessDialog
          connectionName={dialog.connection.name}
          target={{
            connectionId: dialog.connection.id,
            database: target?.connectionId === dialog.connection.id ? target.database : null,
          }}
          provider={(() => {
            const ai = teamAiFor(dialog.connection.id);
            if (ai) return teamAiDestination(ai);
            const p = settings.providers.find((x) => x.id === settings.provider);
            return { label: p?.label ?? settings.provider, local: p?.local ?? false };
          })()}
          access={dialogAccess}
          onSave={async (a) => void (await saveAgentAccess(dialog.connection.id, a))}
          onClose={() => setDialog(null)}
        />
      )}
      {dialog?.kind === "note" && target && (
        <NoteDialog
          key={dialog.note?.id ?? "new"}
          note={dialog.note}
          databaseName={databaseName ?? active?.name ?? "this database"}
          fix={
            dialog.note
              ? noteSuggestions.suggestions.find((s) => s.replaces === dialog.note?.id)
              : undefined
          }
          suggest={sharedNotes && !dialog.note}
          onSave={async (text, check) => {
            if (sharedNotes && !dialog.note) {
              const sent = await commands.suggestNote(target.connectionId, text, check);
              setNotesVersion((v) => v + 1);
              return sent;
            }
            const base: DbNoteInput = dialog.note ?? {
              id: "",
              connectionId: target.connectionId,
              database: target.database,
              text,
              updatedAt: 0,
            };
            await commands.saveNote({
              ...base,
              text,
              check: check.trim() ? { sql: check.trim() } : null,
            });
            setNotesVersion((v) => v + 1);
          }}
          onConfirm={
            dialog.note?.suspect
              ? async () => {
                  await commands.confirmNote(dialog.note!.id);
                  setNotesVersion((v) => v + 1);
                }
              : undefined
          }
          onCheck={
            dialog.note?.check
              ? async () => {
                  const n = await commands.checkNote(dialog.note!.id);
                  setNotesVersion((v) => v + 1);
                  return n;
                }
              : undefined
          }
          onAcceptFix={async (id) => void (await noteSuggestions.accept(id))}
          onDismissFix={noteSuggestions.dismiss}
          onDelete={
            dialog.note
              ? () =>
                  void commands.deleteNote(dialog.note!.id).then(() => {
                    setNotesVersion((v) => v + 1);
                    setDialog(null);
                  })
              : undefined
          }
          onClose={() => setDialog(null)}
        />
      )}
      {dialog?.kind === "noteSuggestions" && (
        <Dialog
          title={`Notes suggested for ${databaseName ?? active?.name ?? "this database"}`}
          onClose={() => setDialog(null)}
          actions={<Button onClick={() => setDialog(null)}>Done</Button>}
        >
          <p className={styles.dialogLead}>
            AI apps suggest these from what they found. Accepted notes are saved like your own and
            go to the Ask box, agent mode and every AI app using SayQL.
          </p>
          <NoteSuggestions
            suggestions={suggestionsHere}
            onAccept={noteSuggestions.accept}
            onDismiss={noteSuggestions.dismiss}
          />
        </Dialog>
      )}
      {dialog?.kind === "teach" && active && (
        <Dialog
          title="Teach SayQL your database"
          onClose={() => setDialog(null)}
          actions={
            <>
              <Button
                variant="ghost"
                onClick={() => {
                  setTeachHidden(true);
                  setDialog(null);
                }}
              >
                Don&apos;t show again
              </Button>
              <Button onClick={() => setDialog(null)}>Done</Button>
            </>
          }
        >
          <TeachSayql
            heading={false}
            target={{
              connectionId: active.id,
              name: active.name,
              database: target?.database ?? null,
            }}
            onSetUp={() => openSettings({ section: "mcp", aiAppsTab: "access" })}
            onAudit={() => setDialog({ kind: "audit" })}
          />
        </Dialog>
      )}
      {dialog?.kind === "audit" && active && (
        <AuditDialog
          target={{
            connectionId: active.id,
            name: active.name,
            database: target?.database ?? null,
          }}
          onReview={() => setDialog({ kind: "noteSuggestions" })}
          onClose={() => setDialog(null)}
        />
      )}
      {dialog?.kind === "deleteScript" && (
        <Dialog
          title={`Delete “${dialog.script.name}”?`}
          size="sm"
          onClose={() => setDialog(null)}
          actions={
            <>
              <Button onClick={() => setDialog(null)}>Cancel</Button>
              <Button
                variant="danger"
                onClick={() => {
                  void deleteScript(dialog.script);
                  setDialog(null);
                }}
              >
                Delete
              </Button>
            </>
          }
        >
          <p style={{ margin: 0 }}>
            The saved script is removed. If it’s open in a tab, the tab keeps its SQL as unsaved.
          </p>
        </Dialog>
      )}
      {dialog?.kind === "deleteConnection" && (
        <Dialog
          title={`Delete “${dialog.connection.name}”?`}
          size="sm"
          onClose={() => setDialog(null)}
          actions={
            <>
              <Button onClick={() => setDialog(null)}>Cancel</Button>
              <Button
                variant="danger"
                onClick={() => {
                  setDialog(null);
                  void connActions.remove(dialog.connection.id);
                }}
              >
                Delete
              </Button>
            </>
          }
        >
          <p style={{ margin: 0 }}>
            The saved connection and its password are removed from SayQL, and its tabs close. The
            database itself isn’t touched.
          </p>
        </Dialog>
      )}
      {dialog?.kind === "feedback" && <FeedbackDialog onClose={() => setDialog(null)} />}
      {dialog?.kind === "settings" && (
        <SettingsPage
          settings={settings}
          connections={conn.connections}
          workspaces={workspaces}
          skills={skills}
          section={dialog.section}
          aiAppsTab={dialog.aiAppsTab}
          noteSuggestions={noteSuggestions}
          createSkill={dialog.createSkill}
          onShowSetup={() => {
            setDialog(null);
            setSetupStep("welcome");
          }}
          // Settings closes first: its Esc would otherwise close both.
          onShowShortcuts={() => {
            setDialog(null);
            setHelp("shortcuts");
          }}
          onShowWhatsNew={() => {
            setDialog(null);
            setHelp("whatsNew");
          }}
          onClose={() => setDialog(null)}
          onSaved={setSettings}
          onTeamChange={setTeam}
        />
      )}
      {dialog?.kind === "teamSignIn" && (
        <TeamSignInDialog
          key={dialog.key}
          server={dialog.server}
          code={dialog.code}
          fromLink={!!dialog.server}
          onClose={() => setDialog(null)}
          onSignedIn={(account) => {
            setDialog(null);
            setTeam(account);
          }}
        />
      )}
      {help === "shortcuts" && <ShortcutSheet onClose={closeHelp} />}
      {help === "whatsNew" && <WhatsNewDialog version={version} onClose={closeHelp} />}
      {/* Waits behind the first-run setup and any other dialog, then shows. */}
      {updates.open && !dialog && !help && !setupStep && <UpdateDialog updates={updates} />}
      {setupStep && !dialog && (
        <OnboardingDialog
          settings={settings}
          step={setupStep}
          onStep={setSetupStep}
          onSettingsSaved={setSettings}
          demoAvailable={!!conn.connection("demo")}
          onUseDemo={() => {
            const demo = conn.connection("demo");
            if (demo) openConnection(demo);
            setSetupStep("try");
          }}
          onConnectOwn={() => {
            setSetupStep(null);
            setDialog({ kind: "connection", config: newConnection(), fromSetup: true });
          }}
          examples={exampleQuestions(schema, active?.id === "demo")}
          onTry={trySetupQuestion}
          onOpenSettings={() => openSettings()}
          onOpenAiApps={() => {
            finishSetup();
            openSettings({ section: "mcp", aiAppsTab: "access" });
          }}
          onTeach={
            active && !active.id.startsWith("team:")
              ? () => {
                  finishSetup();
                  setDialog({ kind: "teach" });
                }
              : undefined
          }
          onFinish={finishSetup}
        />
      )}
    </div>
  );
}
