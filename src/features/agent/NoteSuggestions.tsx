import { useState } from "react";
import { Check, Trash2, X } from "lucide-react";
import type { NoteSuggestion } from "../../api";
import { Alert, Button } from "../../components/ui";
import { errorText } from "../../lib/errors";
import styles from "./NoteSuggestions.module.css";

export interface NoteSuggestionsProps {
  suggestions: NoteSuggestion[];
  /** Where a suggestion is for ("Shop · sales"), when the list spans databases. */
  place?: (s: NoteSuggestion) => string | null;
  /** Saves it as a note. */
  onAccept: (id: string) => Promise<unknown>;
  onDismiss: (id: string) => Promise<unknown>;
}

function when(ms: number): string {
  return new Date(ms).toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short" });
}

/** Notes AI apps suggested over MCP, with what they checked, to accept as database notes or dismiss. */
export function NoteSuggestions({ suggestions, place, onAccept, onDismiss }: NoteSuggestionsProps) {
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const act = (id: string, run: (id: string) => Promise<unknown>) => {
    setBusy(id);
    setError(null);
    run(id).then(
      () => setBusy(null),
      (e) => {
        setBusy(null);
        setError(errorText(e));
      },
    );
  };
  if (suggestions.length === 0)
    return <p className={styles.empty}>No suggested notes are waiting.</p>;
  return (
    <div className={styles.wrap}>
      <ul className={styles.list} aria-label="Suggested notes">
        {suggestions.map((s) => {
          const where = place?.(s);
          return (
            <li key={s.id} className={styles.item}>
              <div className={styles.meta}>
                From <strong>{s.app}</strong>
                {where && <> · {where}</>} · {when(s.createdAt)}
              </div>
              {s.remove ? (
                <>
                  <p className={styles.change}>
                    <span className={styles.label}>Remove note:</span>{" "}
                    <span className={styles.old}>
                      {s.replacedText ?? "a note that was since deleted"}
                    </span>
                  </p>
                  <p className={styles.text}>
                    <span className={styles.label}>Why:</span> {s.text}
                  </p>
                </>
              ) : (
                <>
                  {s.replaces && (
                    <p className={styles.change}>
                      <span className={styles.label}>Replaces:</span>{" "}
                      <span className={styles.old}>
                        {s.replacedText ?? "a note that was since deleted"}
                      </span>
                    </p>
                  )}
                  <p className={styles.text}>{s.text}</p>
                </>
              )}
              {s.evidence && (
                <p className={styles.evidence}>
                  <span className={styles.label}>Evidence:</span> {s.evidence}
                </p>
              )}
              {s.check && (
                <p className={styles.evidence}>
                  <span className={styles.label}>Check:</span> <code>{s.check}</code>
                </p>
              )}
              <div className={styles.actions}>
                <Button
                  size="sm"
                  variant="ghost"
                  icon={X}
                  disabled={busy === s.id}
                  onClick={() => act(s.id, onDismiss)}
                >
                  Dismiss
                </Button>
                <Button
                  size="sm"
                  variant={s.remove ? "danger" : "primary"}
                  icon={s.remove ? Trash2 : Check}
                  loading={busy === s.id}
                  onClick={() => act(s.id, onAccept)}
                >
                  {s.remove ? "Remove note" : s.replaces ? "Replace note" : "Accept"}
                </Button>
              </div>
            </li>
          );
        })}
      </ul>
      {error && <Alert tone="danger">{error}</Alert>}
    </div>
  );
}
