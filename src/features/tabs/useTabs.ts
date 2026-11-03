import { useCallback } from "react";
import type { PreviousQuery, Target } from "../../api";
import { usePersistentState } from "../../hooks/usePersistentState";
import type { AgentRunState } from "../agent";

export interface EditorTab {
  id: string;
  /** The database this tab works in; fixed for the tab's life, like a DBeaver script. */
  target: Target;
  title: string;
  question: string;
  sql: string;
  /** Run `sql` when the tab first shows (table previews). */
  autoRun?: boolean;
  /** Ask the AI `question` when the tab first shows, then run the result (first-run setup). */
  autoAsk?: boolean;
  /** The query last run here, which follow-up requests build on. */
  lastRun?: PreviousQuery | null;
  /** Tables picked in the navigator (qualified names) that the AI should build on. */
  focusTables?: string[];
  /** False while the picked tables are hidden from the AI (kept, not cleared). */
  focusEnabled?: boolean;
  /** The saved script this tab edits, if any. */
  scriptId?: string;
  /** The SQL as last saved (or as generated for a table preview); differs from `sql` when there are unsaved changes. */
  savedSql?: string;
  /** Opened by an AI app, e.g. "From Claude (MCP)". */
  origin?: string;
  /** The tab's last agent run: its steps' queries and the answer, never rows. */
  agentRun?: AgentRunState | null;
}

/** A tab nothing has happened in yet: no SQL, question, run, title or script. */
export function isPristine(tab: EditorTab): boolean {
  return (
    !tab.sql.trim() &&
    !tab.question.trim() &&
    !tab.title &&
    !tab.scriptId &&
    !tab.lastRun &&
    !tab.agentRun &&
    !tab.origin
  );
}

/** True when closing the tab would lose SQL. */
export function hasUnsavedChanges(tab: EditorTab): boolean {
  return tab.sql.trim() !== "" && tab.sql !== (tab.savedSql ?? "");
}

interface TabsState {
  tabs: EditorTab[];
  activeId: string | null;
}

export type NewTab = Partial<
  Pick<
    EditorTab,
    "title" | "question" | "sql" | "autoRun" | "autoAsk" | "scriptId" | "savedSql" | "origin"
  >
>;

/** Open editor tabs and which one is active. Remembered across launches. */
export function useTabs() {
  const [state, setState] = usePersistentState<TabsState>("editor-tabs", {
    tabs: [],
    activeId: null,
  });
  const active = state.tabs.find((t) => t.id === state.activeId) ?? state.tabs[0] ?? null;

  const open = useCallback(
    (target: Target, init: NewTab = {}) => {
      const tab: EditorTab = {
        id: crypto.randomUUID(),
        target,
        title: "",
        question: "",
        sql: "",
        ...init,
      };
      setState((s) => ({ tabs: [...s.tabs, tab], activeId: tab.id }));
      return tab.id;
    },
    [setState],
  );

  const activate = useCallback(
    (id: string) => setState((s) => ({ ...s, activeId: id })),
    [setState],
  );

  const update = useCallback(
    (id: string, patch: Partial<EditorTab>) =>
      setState((s) => ({ ...s, tabs: s.tabs.map((t) => (t.id === id ? { ...t, ...patch } : t)) })),
    [setState],
  );

  /** Closes the matching tabs; the active tab moves to the neighbour on the left. */
  const closeWhere = useCallback(
    (match: (t: EditorTab) => boolean) =>
      setState((s) => {
        const tabs = s.tabs.filter((t) => !match(t));
        if (tabs.some((t) => t.id === s.activeId)) return { ...s, tabs };
        const index = s.tabs.findIndex((t) => t.id === s.activeId);
        const neighbour =
          s.tabs
            .slice(0, Math.max(index, 0))
            .reverse()
            .find((t) => !match(t)) ??
          tabs[0] ??
          null;
        return { tabs, activeId: neighbour?.id ?? null };
      }),
    [setState],
  );

  const close = useCallback((id: string) => closeWhere((t) => t.id === id), [closeWhere]);

  return { tabs: state.tabs, active, open, activate, update, close, closeWhere };
}
