import { useState } from "react";
import { ChevronRight, Pencil, Plus, Trash2 } from "lucide-react";
import type { Workspace } from "../../api";
import { ContextMenu, useContextMenu } from "../../components/ui";
import { cx } from "../../lib/cx";
import { DeleteWorkspaceDialog, NameDialog } from "./WorkspaceDialogs";
import { initialsOf, PERSONAL, workspaceColor, type WorkspacesApi } from "./useWorkspaces";
import styles from "./Workspaces.module.css";

/** The companies you work for, at the window's left edge. Shows once there's more than Personal. */
export function WorkspaceRail({
  workspaces,
  onConnectionsDeleted,
}: {
  workspaces: WorkspacesApi;
  onConnectionsDeleted: (ids: string[]) => void;
}) {
  const menu = useContextMenu();
  const [dialog, setDialog] = useState<
    { kind: "new" } | { kind: "rename" | "delete"; workspace: Workspace } | null
  >(null);
  if (!workspaces.several) return null;

  const tile = (w: Workspace) => {
    const on = w.id === workspaces.activeId;
    return (
      <div key={w.id} className={styles.railItem}>
        <span className={cx(styles.railMark, on && styles.railMarkOn)} />
        <button
          type="button"
          className={cx(styles.tile, on && styles.tileOn)}
          style={{ background: workspaceColor(w.color) }}
          title={w.name}
          aria-label={w.name}
          aria-current={on}
          onClick={() => workspaces.switchTo(w.id)}
          onContextMenu={(e) =>
            menu.open(e, [
              {
                label: "Rename…",
                icon: Pencil,
                onSelect: () => setDialog({ kind: "rename", workspace: w }),
              },
              {
                label: "Delete…",
                icon: Trash2,
                danger: true,
                disabled: w.id === PERSONAL,
                onSelect: () => setDialog({ kind: "delete", workspace: w }),
              },
            ])
          }
        >
          {initialsOf(w.name)}
        </button>
      </div>
    );
  };

  return (
    <nav className={styles.rail} aria-label="Workspaces">
      {workspaces.workspaces.filter((w) => w.id === PERSONAL).map(tile)}
      <span className={styles.railDivider} />
      {workspaces.workspaces.filter((w) => w.id !== PERSONAL).map(tile)}
      <button
        type="button"
        className={styles.addTile}
        title="New workspace"
        aria-label="New workspace"
        onClick={() => setDialog({ kind: "new" })}
      >
        <Plus size={16} />
      </button>
      {menu.menu && <ContextMenu at={menu.menu.at} items={menu.menu.items} onClose={menu.close} />}
      {dialog?.kind === "new" && (
        <NewWorkspaceDialog workspaces={workspaces} onClose={() => setDialog(null)} />
      )}
      {dialog?.kind === "rename" && (
        <NameDialog
          title={`Rename ${dialog.workspace.name}`}
          initial={dialog.workspace.name}
          action="Rename"
          onSubmit={(name) => workspaces.renameWorkspace(dialog.workspace.id, name)}
          onClose={() => setDialog(null)}
        />
      )}
      {dialog?.kind === "delete" && (
        <DeleteWorkspaceDialog
          workspaces={workspaces}
          target={dialog.workspace}
          onConnectionsDeleted={onConnectionsDeleted}
          onClose={() => setDialog(null)}
        />
      )}
    </nav>
  );
}

export function NewWorkspaceDialog({
  workspaces,
  onClose,
}: {
  workspaces: WorkspacesApi;
  onClose: () => void;
}) {
  return (
    <NameDialog
      title="New workspace"
      hint="One company or client. It starts empty, and AI apps need their own setup to see it."
      action="Create"
      onSubmit={workspaces.createWorkspace}
      onClose={onClose}
    />
  );
}

/** Where you are, in the top bar: the workspace, then the open connection's group. */
export function WorkspaceCrumb({
  workspaces,
  connectionId,
  database,
}: {
  workspaces: WorkspacesApi;
  connectionId: string | null;
  database: string | null;
}) {
  if (!workspaces.several || !workspaces.workspace) return null;
  const group = connectionId ? workspaces.groupAt(connectionId, database) : null;
  const { workspace } = workspaces;
  return (
    <span className={styles.crumb}>
      <span className={styles.miniTile} style={{ background: workspaceColor(workspace.color) }}>
        {initialsOf(workspace.name)}
      </span>
      <span className={styles.crumbName}>{workspace.name}</span>
      {group && (
        <>
          <ChevronRight size={12} />
          <span className={styles.crumbGroup}>{group.name}</span>
        </>
      )}
    </span>
  );
}
