import { useCallback, useMemo, useState } from "react";
import { commands, type WorkspaceLayout } from "../../api";
import { usePersistentState } from "../../hooks/usePersistentState";

/** The workspace that always exists, and the only one an AI app set up without one sees. */
export const PERSONAL = "personal";
/** Team connections' ids start with this. Must match `ID_PREFIX` in src-tauri/src/team/mod.rs. */
const TEAM_PREFIX = "team:";

/** Up to two letters for a workspace's tile. */
export function initialsOf(name: string): string {
  const words = name.trim().split(/\s+/).filter(Boolean);
  return words
    .slice(0, 2)
    .map((w) => [...w][0] ?? "")
    .join("")
    .toUpperCase();
}

const WORKSPACE_COLORS = [
  "var(--color-workspace-0)",
  "var(--color-workspace-1)",
  "var(--color-workspace-2)",
  "var(--color-workspace-3)",
  "var(--color-workspace-4)",
  "var(--color-workspace-5)",
  "var(--color-workspace-6)",
];

/** A workspace's tile colour, a token from styles/tokens.css. */
export const workspaceColor = (color: number) => WORKSPACE_COLORS[color] ?? WORKSPACE_COLORS[0];

/**
 * The saved workspaces and groups, and which workspace this window shows. Everything else in the
 * workbench asks this hook what's in the open workspace. Each change is saved by the backend,
 * which returns what's now saved.
 */
export function useWorkspaces(initial: WorkspaceLayout) {
  const [layout, setLayout] = useState(initial);
  const [saved, setActive] = usePersistentState("sayql.workspace", PERSONAL);
  const workspace =
    layout.workspaces.find((w) => w.id === saved) ??
    layout.workspaces.find((w) => w.id === PERSONAL) ??
    layout.workspaces[0];
  const activeId = workspace?.id ?? PERSONAL;

  const workspaceOf = useCallback(
    (connectionId: string) =>
      connectionId.startsWith(TEAM_PREFIX)
        ? (layout.teamWorkspace ?? PERSONAL)
        : (layout.placement[connectionId]?.workspaceId ?? PERSONAL),
    [layout],
  );

  const refresh = useCallback(() => commands.getLayout().then(setLayout), []);
  /** Applies a change; a refusal goes to the caller, to show where the person is. */
  const apply = useCallback(
    (change: Promise<WorkspaceLayout>) => change.then((next) => void setLayout(next)),
    [],
  );

  return useMemo(
    () => ({
      layout,
      workspaces: layout.workspaces,
      workspace,
      activeId,
      /** More than Personal: only then does anything about workspaces show. */
      several: layout.workspaces.length > 1,
      groupsHere: layout.groups.filter((g) => g.workspaceId === activeId),
      workspaceOf,
      switchTo: setActive,
      /** Shows the workspace `connectionId` is in, for work that opens on it. */
      reveal: (connectionId: string) => setActive(workspaceOf(connectionId)),
      groupOf: (connectionId: string) =>
        layout.groups.find((g) => g.id === layout.placement[connectionId]?.groupId) ?? null,
      /** The group of what's open: a database placed apart, else its connection's. */
      groupAt: (connectionId: string, database: string | null) => {
        const apart = layout.databaseGroups.find(
          (d) => d.connectionId === connectionId && d.database === database,
        );
        const id = apart?.groupId ?? layout.placement[connectionId]?.groupId;
        return layout.groups.find((g) => g.id === id) ?? null;
      },
      /** The single databases placed in a group. */
      databasesIn: (groupId: string) => layout.databaseGroups.filter((d) => d.groupId === groupId),
      /** Whether one database of a connection sits in a group of its own. */
      isPlacedApart: (connectionId: string, database: string) =>
        layout.databaseGroups.some(
          (d) => d.connectionId === connectionId && d.database === database,
        ),
      allowedOf: (connectionId: string) => layout.placement[connectionId]?.allowedDatabases ?? null,
      refresh,
      createWorkspace: async (name: string) => {
        const next = await commands.createWorkspace(name);
        setLayout(next);
        const made = next.workspaces.find((w) => !layout.workspaces.some((old) => old.id === w.id));
        if (made) setActive(made.id);
      },
      renameWorkspace: (id: string, name: string) => apply(commands.renameWorkspace(id, name)),
      deleteWorkspace: (id: string, moveTo: string | null) =>
        apply(commands.deleteWorkspace(id, moveTo)),
      createGroup: (name: string) => apply(commands.createGroup(activeId, name)),
      renameGroup: (id: string, name: string) => apply(commands.renameGroup(id, name)),
      deleteGroup: (id: string) => apply(commands.deleteGroup(id)),
      moveConnection: (id: string, workspaceId: string, groupId: string | null) =>
        apply(commands.moveConnection(id, workspaceId, groupId)),
      setDatabaseGroup: (id: string, database: string, groupId: string | null) =>
        apply(commands.setDatabaseGroup(id, database, groupId)),
      setTeamWorkspace: (id: string) => apply(commands.setTeamWorkspace(id)),
    }),
    [layout, workspace, activeId, workspaceOf, setActive, refresh, apply],
  );
}

export type WorkspacesApi = ReturnType<typeof useWorkspaces>;
