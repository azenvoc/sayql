import { useId, useState } from "react";
import type { AgentAccess, AgentAccessSettings, AgentResults, Target } from "../../api";
import { Alert, Button, Checkbox, Dialog } from "../../components/ui";
import { cx } from "../../lib/cx";
import styles from "./AgentAccessDialog.module.css";
import { NeverSendPicker } from "./NeverSendPicker";
import { AGENT_OPTIONS, NEVER_SEND_SCOPE, appLevelShort } from "./resultLevels";

export interface AgentAccessDialogProps {
  connectionName: string;
  /** The database whose tables the never-send picker shows (access itself is per connection). */
  target: Target;
  provider: { label: string; local: boolean };
  access: AgentAccess;
  /** Saves and closes; rejects with a message to show. */
  onSave: (access: AgentAccessSettings) => Promise<void>;
  onClose: () => void;
  /**
   * Only the never-send columns (Settings → AI apps): agent mode stays as it is, on or off.
   * Never-send applies to agent mode and to AI apps over MCP alike.
   */
  neverSendOnly?: boolean;
}

const RANK: Record<AgentResults, number> = { off: 0, counts: 1, rows: 2 };

/** Turns agent mode on for a connection and says exactly what it may send to the AI. */
export function AgentAccessDialog({
  connectionName,
  target,
  provider,
  access,
  onSave,
  onClose,
  neverSendOnly = false,
}: AgentAccessDialogProps) {
  const first = !access.enabled && !neverSendOnly;
  const resultsLabel = useId();
  const [results, setResults] = useState<AgentResults>(access.results);
  // Ask before send starts on for cloud providers, off for ones on this computer.
  const [ask, setAsk] = useState(first ? !provider.local : access.askBeforeSend);
  // On a team connection the admin's rules win: results no higher than theirs, their columns always kept.
  const team = access.team ?? null;
  const adminColumn = (c: string) =>
    !!team?.neverSend.some((t) => t.toLowerCase() === c.toLowerCase());
  const allowed = (r: AgentResults) => !team || RANK[r] <= RANK[team.maxResults];
  const [neverSend, setNeverSend] = useState(() => access.neverSend.filter((c) => !adminColumn(c)));
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  const save = (enabled: boolean) => {
    setSaving(true);
    setError(null);
    onSave({ enabled, results, apps: access.apps, askBeforeSend: ask, neverSend }).then(
      onClose,
      (e) => {
        setSaving(false);
        setError(String(e));
      },
    );
  };

  const goesOut =
    results === "off"
      ? null
      : `${results === "rows" ? "Masked rows (up to 20 per query)" : "Counts and column summaries"} from this database will go to ${provider.label}${provider.local ? ", which runs on this computer" : ". Their terms apply"}.`;

  return (
    <Dialog
      title={
        neverSendOnly
          ? `Never send on ${connectionName}`
          : first
            ? `Turn on Investigate for ${connectionName}`
            : `Investigate settings for ${connectionName}`
      }
      onClose={onClose}
      size="lg"
      secondaryActions={
        !first &&
        !neverSendOnly && (
          <Button variant="dangerGhost" disabled={saving} onClick={() => save(false)}>
            Turn off Investigate
          </Button>
        )
      }
      actions={
        <>
          <Button onClick={onClose}>Cancel</Button>
          <Button
            variant="primary"
            disabled={saving}
            onClick={() => save(neverSendOnly ? access.enabled : true)}
          >
            {first ? "Turn on" : "Save"}
          </Button>
        </>
      }
    >
      <div className={styles.body}>
        {neverSendOnly ? (
          <p className={styles.lead}>
            Columns picked here never leave SayQL. {NEVER_SEND_SCOPE} Investigate stays{" "}
            {access.enabled ? "on" : "off"} for this connection.
          </p>
        ) : (
          <>
            <p className={styles.lead}>
              The agent answers in several read-only steps with <strong>{provider.label}</strong>.
              It never changes data: a write is shown as SQL for you to review.
            </p>
            <h3 className={styles.groupLabel} id={resultsLabel}>
              What the AI may see of query results
            </h3>
            <div role="radiogroup" aria-labelledby={resultsLabel} className={styles.options}>
              {AGENT_OPTIONS.map((r) => (
                <label
                  key={r.value}
                  className={cx(
                    styles.option,
                    results === r.value && styles.chosen,
                    !allowed(r.value) && styles.locked,
                  )}
                >
                  <input
                    type="radio"
                    name="results"
                    value={r.value}
                    checked={results === r.value}
                    disabled={!allowed(r.value)}
                    onChange={() => setResults(r.value)}
                  />
                  <span>
                    <strong>
                      {r.label}
                      {r.isDefault && <span className={styles.default}> · default</span>}
                      {!allowed(r.value) && (
                        <span className={styles.admin}> · Set by your team admin</span>
                      )}
                    </strong>
                    <span className={styles.detail}>{r.detail}</span>
                  </span>
                </label>
              ))}
            </div>
            {goesOut && <Alert tone="warning">{goesOut}</Alert>}
            {!target.connectionId.startsWith("team:") && (
              <p className={styles.detail}>
                Separate from this: AI apps over MCP get back{" "}
                <strong>{appLevelShort(access.apps)}</strong> on this connection (Settings → AI apps
                → Access).
              </p>
            )}
            <Checkbox
              label="Ask before each send (shows exactly what would go, with Send and Skip)"
              checked={ask}
              onChange={setAsk}
            />
          </>
        )}
        {team && team.neverSend.length > 0 && (
          <p className={styles.detail}>
            Set by your team admin, never sent:{" "}
            <span className={styles.adminColumns}>{team.neverSend.join(", ")}</span>. Your team's
            server also leaves them out of results.
          </p>
        )}
        {!neverSendOnly && <p className={styles.detail}>{NEVER_SEND_SCOPE}</p>}
        <NeverSendPicker
          target={target}
          label={
            team?.neverSend.length ? "Also never send these columns" : "Never send these columns"
          }
          value={neverSend}
          onChange={setNeverSend}
          locked={team?.neverSend ?? []}
          autoFocus={neverSendOnly}
        />
        {!neverSendOnly && (
          <p className={styles.detail}>
            Masking is best effort: a free-text column can hold anything. For health or production
            data, keep results at Nothing or Counts and summaries, or mark such columns never send.
          </p>
        )}
        {error && <Alert tone="danger">{error}</Alert>}
      </div>
    </Dialog>
  );
}
