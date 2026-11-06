import { Plus, Settings as SettingsIcon } from "lucide-react";
import type { ConnectionConfig, Schema } from "../api";
import { Badge, Wordmark, Button, IconButton, Logo, ToolbarDivider } from "../components/ui";
import { DATABASE_KINDS, ProdBadge } from "../features/connections";
import { ThemeToggle } from "../features/theme";
import { WorkspaceCrumb, type WorkspacesApi } from "../features/workspaces";
import styles from "./Workbench.module.css";

export interface TopBarProps {
  active: ConnectionConfig | null;
  databaseName: string | null;
  schema: Schema | null;
  onNewConnection: () => void;
  onOpenSettings: () => void;
  /** Notes AI apps suggested, waiting; shown as a count on the settings button. */
  pendingSuggestions?: number;
  workspaces?: WorkspacesApi;
}

export function TopBar({
  active,
  databaseName,
  schema,
  onNewConnection,
  onOpenSettings,
  pendingSuggestions = 0,
  workspaces,
}: TopBarProps) {
  return (
    <header className={styles.topbar}>
      <div className={styles.brand}>
        <Wordmark height={18} />
      </div>
      <ToolbarDivider />
      <Button size="sm" variant="ghost" icon={Plus} onClick={onNewConnection}>
        New connection
      </Button>
      {workspaces && (
        <WorkspaceCrumb
          workspaces={workspaces}
          connectionId={active?.id ?? null}
          database={databaseName}
        />
      )}
      {active && (
        <div
          className={styles.activeConnection}
          title={active.kind === "sqlite" ? active.file : `${active.host}:${active.port}`}
        >
          <Logo id={DATABASE_KINDS[active.kind].logo} size={14} />
          <span className={styles.activeName}>{active.name}</span>
          {databaseName && active.kind !== "sqlite" && (
            <span className={styles.activeDatabase}>{databaseName}</span>
          )}
          {schema && <Badge>{schema.dialect}</Badge>}
          <ProdBadge connection={active} />
        </div>
      )}
      <span className={styles.spacer} />
      <ThemeToggle />
      <span className={styles.settingsButton}>
        <IconButton
          icon={SettingsIcon}
          label={
            pendingSuggestions > 0
              ? `Settings: ${pendingSuggestions} suggested ${pendingSuggestions === 1 ? "note" : "notes"} waiting`
              : "Settings"
          }
          onClick={onOpenSettings}
        />
        {pendingSuggestions > 0 && (
          <span className={styles.settingsBadge} aria-hidden="true">
            {pendingSuggestions > 9 ? "9+" : pendingSuggestions}
          </span>
        )}
      </span>
    </header>
  );
}
