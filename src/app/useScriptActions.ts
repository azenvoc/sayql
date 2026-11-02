import type { Script } from "../api";
import type { useScripts } from "../features/scripts";
import type { EditorTab, useTabs } from "../features/tabs";

type Scripts = ReturnType<typeof useScripts>;
type Tabs = ReturnType<typeof useTabs>;

/** Saving tabs as scripts, opening scripts in tabs, and deleting scripts without losing open SQL. */
export function useScriptActions(scripts: Scripts, tabs: Tabs) {
  /** The saved script a tab edits, if it still exists. */
  const scriptOf = (t: EditorTab) =>
    t.scriptId ? (scripts.scripts.find((s) => s.id === t.scriptId) ?? null) : null;

  /** Saves the tab as a script (named `name`, or keeping its script's name) and marks it saved. */
  const saveTab = async (t: EditorTab, name?: string) => {
    const existing = scriptOf(t);
    const saved = await scripts.save({
      id: existing?.id ?? "",
      name: name ?? existing?.name ?? t.title,
      connectionId: t.target.connectionId,
      database: t.target.database,
      question: t.question,
      sql: t.sql,
      updatedAt: 0,
    });
    tabs.update(t.id, { scriptId: saved.id, title: saved.name, savedSql: t.sql });
  };

  /** Focuses the tab already editing the script, or opens it in a new one; false when there's no such script. */
  const openScript = (id: string) => {
    const s = scripts.scripts.find((x) => x.id === id);
    if (!s) return false;
    const open = tabs.tabs.find((t) => t.scriptId === id);
    if (open) tabs.activate(open.id);
    else
      tabs.open(
        { connectionId: s.connectionId, database: s.database },
        { scriptId: s.id, title: s.name, sql: s.sql, question: s.question, savedSql: s.sql },
      );
    return true;
  };

  const deleteScript = async (s: Script) => {
    await scripts.remove(s.id);
    // Open tabs keep their SQL, now unsaved, so nothing is lost by accident.
    tabs.tabs
      .filter((t) => t.scriptId === s.id)
      .forEach((t) => tabs.update(t.id, { scriptId: undefined, savedSql: undefined }));
  };

  return { scriptOf, saveTab, openScript, deleteScript };
}
