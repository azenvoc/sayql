import type { McpApp, McpApps } from "../../api";
import { Badge, Button, Checkbox } from "../../components/ui";
import styles from "./ClaudeIntegration.module.css";

export function when(ms: number | null | undefined): string {
  return ms
    ? new Date(ms).toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short" })
    : "Never";
}

/** Whether an app is let in: blocked wins, then (only while approval is required) approved. */
export function appState(app: McpApp, onlyApproved: boolean): "blocked" | "waiting" | "allowed" {
  if (app.blocked) return "blocked";
  if (onlyApproved && !app.approved) return "waiting";
  return "allowed";
}

const BADGE = {
  blocked: { tone: "danger", label: "Blocked" },
  waiting: { tone: "warning", label: "Waiting for approval" },
  allowed: { tone: "success", label: "Allowed" },
} as const;

export interface AppsListProps {
  /** `null` while loading. */
  list: McpApps | null;
  onBlock: (app: McpApp, blocked: boolean) => void;
  onApprove: (app: McpApp) => void;
  onOnlyApproved: (on: boolean) => void;
}

/** The AI apps that used SayQL over MCP (or were blocked or approved), each allowed, blocked or waiting for approval. */
export function AppsList({ list, onBlock, onApprove, onOnlyApproved }: AppsListProps) {
  const apps = list?.apps ?? null;
  const onlyApproved = !!list?.onlyApproved;
  return (
    <section className={styles.group} aria-labelledby="ai-apps-heading">
      <h3 id="ai-apps-heading" className={styles.heading}>
        Apps
      </h3>
      {list && (
        <div className={styles.name}>
          <Checkbox
            label="Only allow apps I've approved"
            checked={onlyApproved}
            onChange={onOnlyApproved}
          />
          <span className={styles.detail}>
            Turning this on approves the apps already listed that aren&apos;t blocked. Apps are
            recognised by the name they report, so this keeps out unknown apps but isn&apos;t a
            security boundary against software that lies about its name.
          </span>
        </div>
      )}
      <p className={styles.intro}>
        Block an app to refuse it everything. Apps give their own name when they connect, so a block
        stops an app, not software that pretends to be it.
      </p>
      {apps === null ? null : apps.length === 0 ? (
        <p className={styles.empty}>
          No AI app has connected yet. Connect one below; it shows up here after its first request.
        </p>
      ) : (
        <ul className={styles.list} aria-label="AI apps">
          {apps.map((a) => {
            const state = appState(a, onlyApproved);
            return (
              <li key={a.name} className={styles.row}>
                <div className={styles.name}>
                  <span className={styles.appName}>
                    {a.name}
                    <Badge tone={BADGE[state].tone}>{BADGE[state].label}</Badge>
                  </span>
                  <span className={styles.detail}>
                    {a.queries} {a.queries === 1 ? "request" : "requests"} in the log · last seen{" "}
                    {when(a.lastUsed)}
                  </span>
                </div>
                <div className={styles.rowActions}>
                  {state === "waiting" && (
                    <Button
                      size="sm"
                      variant="secondary"
                      aria-label={`Approve ${a.name}`}
                      onClick={() => onApprove(a)}
                    >
                      Approve
                    </Button>
                  )}
                  <Button
                    size="sm"
                    variant={a.blocked ? "secondary" : "dangerGhost"}
                    aria-label={`${a.blocked ? "Allow" : "Block"} ${a.name}`}
                    onClick={() => onBlock(a, !a.blocked)}
                  >
                    {a.blocked ? "Allow" : "Block"}
                  </Button>
                </div>
              </li>
            );
          })}
        </ul>
      )}
    </section>
  );
}
