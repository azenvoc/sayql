import { ArrowDownToLine, Circle, MessageSquare, Plug, ShieldCheck, Sparkles } from "lucide-react";
import type { ConnectionConfig, Schema } from "../api";
import type { ModelNames } from "../features/settings";
import type { UpdateState } from "../features/updates";
import { Button, Logo } from "../components/ui";
import { ProdBadge } from "../features/connections";
import { privacySummary, useAgentAccess } from "../features/agent";
import { cx } from "../lib/cx";
import styles from "./Workbench.module.css";

export interface StatusBarProps {
  active: ConnectionConfig | null;
  databaseName: string | null;
  schema: Schema | null;
  loading: boolean;
  /** Whether the active connection is connected; nothing connects on its own at start. */
  connected: boolean;
  onConnect: () => void;
  modelNames: ModelNames;
  aiReady: boolean;
  updates: UpdateState;
  onFeedback: () => void;
  /** The AI provider the Ask box and Investigate use, for the privacy indicator. */
  provider: { label: string; local: boolean } | null;
  /** Opens the connection's Investigate settings, which also set what AI apps get. */
  onPrivacy: () => void;
  /** With several workspaces: what AI apps set up for this one can see. */
  aiScope?: { workspaceName: string; onOpen: () => void };
}

/**
 * What can reach an AI from the active connection, in a few words; the tooltip has a line each
 * for the Ask box, Investigate, AI apps and never-send columns. Click to change it.
 */
function PrivacyChip({
  connectionId,
  provider,
  onClick,
}: {
  connectionId: string;
  provider: StatusBarProps["provider"];
  onClick: () => void;
}) {
  const access = useAgentAccess(connectionId);
  if (!access) return null;
  const s = privacySummary(access, provider);
  return (
    <button
      type="button"
      className={cx(styles.privacy, styles[`privacy-${s.level}`])}
      title={`${s.lines.join("\n")}\nClick to change.`}
      aria-label={`${s.label}. ${s.lines.join(" ")} Click to change.`}
      onClick={onClick}
      data-testid="privacy-chip"
    >
      <ShieldCheck size={11} />
      {s.label}
    </button>
  );
}

export function StatusBar({
  active,
  databaseName,
  schema,
  loading,
  connected,
  onConnect,
  modelNames,
  aiReady,
  updates,
  onFeedback,
  provider,
  onPrivacy,
  aiScope,
}: StatusBarProps) {
  const columns = schema?.tables.reduce((n, t) => n + t.columns.length, 0) ?? 0;
  const state =
    !active || !connected
      ? "Not connected"
      : loading
        ? "Connecting…"
        : schema
          ? "Connected"
          : "Disconnected";
  return (
    <footer className={styles.statusbar}>
      <span className={styles.statusItem}>
        <Circle
          size={8}
          className={cx(styles.dot, schema && !loading && styles.dotOk)}
          fill="currentColor"
        />
        {state}
        {active && ` · ${active.name}`}
        {active && databaseName && active.kind !== "sqlite" && ` / ${databaseName}`}
        <ProdBadge connection={active} />
      </span>
      {active && <PrivacyChip connectionId={active.id} provider={provider} onClick={onPrivacy} />}
      {aiScope && (
        <button
          type="button"
          className={styles.privacy}
          title={`AI apps set up for ${aiScope.workspaceName} can't see your other workspaces. Click to manage.`}
          onClick={aiScope.onOpen}
        >
          <Plug size={11} />
          AI apps: {aiScope.workspaceName} only
        </button>
      )}
      {active && !connected && (
        <Button
          size="sm"
          variant="ghost"
          icon={Plug}
          onClick={onConnect}
          title={`Connect to ${active.name}`}
        >
          Connect
        </Button>
      )}
      {schema && (
        <span className={styles.statusItem}>
          {schema.tables.length} tables · {columns} columns
        </span>
      )}
      <span className={styles.spacer} />
      <Button
        size="sm"
        variant="ghost"
        icon={MessageSquare}
        onClick={onFeedback}
        title="Send feedback"
      >
        Feedback
      </Button>
      {updates.update && (
        <Button
          size="sm"
          variant="primary"
          icon={ArrowDownToLine}
          loading={updates.installing}
          title={
            updates.error ?? `SayQL ${updates.update.version} is available. Click for details.`
          }
          onClick={updates.show}
        >
          {updates.error ? "Update failed — retry" : `Update to ${updates.update.version}`}
        </Button>
      )}
      <span className={styles.statusItem}>
        {aiReady ? <Logo id={modelNames.logo} size={11} /> : <Sparkles size={11} />}
        {aiReady
          ? `${modelNames.typing} while typing · ${modelNames.expert} for fixes`
          : "AI off — set up an AI provider in Settings"}
      </span>
    </footer>
  );
}
