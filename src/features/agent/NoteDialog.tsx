import { useState } from "react";
import type { DbNote, NoteSuggestion } from "../../api";
import { Alert, Badge, Button, Dialog, Field } from "../../components/ui";
import base from "./AgentAccessDialog.module.css";
import styles from "./NoteDialog.module.css";

export interface NoteDialogProps {
  /** The note when editing or reviewing; `null` for a new note. */
  note: DbNote | null;
  databaseName: string;
  /** A suggested fix to this note waiting for the person, if any. */
  fix?: NoteSuggestion;
  /**
   * Saves and closes; rejects with a message to show (a note that holds a value is refused).
   * With `suggest`, resolves with what to tell the person, and the dialog stays open to say it.
   */
  onSave: (text: string, check: string) => Promise<string | void>;
  /** A team connection's shared notes: the note is suggested to the team's knowledge owners, not saved here. */
  suggest?: boolean;
  /** The note is still true: it's no longer doubtful. */
  onConfirm?: () => Promise<void>;
  /** Runs the note's check; resolves with the note and the result. */
  onCheck?: () => Promise<DbNote>;
  onAcceptFix?: (id: string) => Promise<void>;
  onDismissFix?: (id: string) => Promise<void>;
  onDelete?: () => void;
  onClose: () => void;
}

const day = (ms: number) => new Date(ms).toLocaleDateString();

/** The last check, in words. */
function checkResult(n: DbNote): { tone: "success" | "warning" | "neutral"; text: string } | null {
  const last = n.check?.last;
  if (!last) return null;
  if (last.ok === true) return { tone: "success", text: `Passed ${day(last.at)}` };
  if (last.ok === false) return { tone: "warning", text: `Failed ${day(last.at)}` };
  return { tone: "neutral", text: `Couldn't run: ${last.error ?? "unknown error"}` };
}

/**
 * Adds, edits or reviews a database note: a fact about tables, columns or codes, never values
 * from rows. A doubtful note shows why, and the fix SayQL or an AI app suggested for it.
 */
export function NoteDialog({
  note: initial,
  databaseName,
  fix,
  onSave,
  suggest,
  onConfirm,
  onCheck,
  onAcceptFix,
  onDismissFix,
  onDelete,
  onClose,
}: NoteDialogProps) {
  const [sent, setSent] = useState<string | null>(null);
  const [note, setNote] = useState(initial);
  const [text, setText] = useState(initial?.text ?? "");
  const [check, setCheck] = useState(initial?.check?.sql ?? "");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const act = (f: () => Promise<unknown>, close = true) => {
    setBusy(true);
    setError(null);
    f()
      .then(
        () => (close ? onClose() : undefined),
        (e) => setError(String(e)),
      )
      .finally(() => setBusy(false));
  };
  const result = note && checkResult(note);
  const edited =
    text.trim() !== (initial?.text ?? "") || check.trim() !== (initial?.check?.sql ?? "");
  const title = suggest
    ? `Suggest a note for ${databaseName}`
    : !initial
      ? `Add a note to ${databaseName}`
      : initial.suspect
        ? "Review database note"
        : "Edit database note";
  if (suggest && sent) {
    return (
      <Dialog
        title={title}
        size="md"
        onClose={onClose}
        actions={
          <Button variant="primary" onClick={onClose}>
            Done
          </Button>
        }
      >
        <div className={base.body}>
          <Alert tone="success">{sent}</Alert>
        </div>
      </Dialog>
    );
  }
  return (
    <Dialog
      title={title}
      size="md"
      onClose={onClose}
      actions={
        <>
          {onDelete && (
            <Button variant="ghost" className={styles.left} onClick={onDelete}>
              Delete note
            </Button>
          )}
          <Button onClick={onClose}>Cancel</Button>
          <Button
            variant="primary"
            disabled={!text.trim() || busy || (!!initial && !edited)}
            onClick={() =>
              suggest
                ? act(() => onSave(text, check).then((m) => setSent(m || "Sent.")), false)
                : act(() => onSave(text, check))
            }
          >
            {suggest ? "Send" : "Save"}
          </Button>
        </>
      }
    >
      <div className={base.body}>
        {suggest && (
          <p className={styles.meta}>
            Shared by your team. Owners approve changes on the team server.
          </p>
        )}
        {note?.suspect && (
          <Alert
            tone="warning"
            actions={
              onConfirm && (
                <Button size="sm" disabled={busy} onClick={() => act(onConfirm)}>
                  It's still true
                </Button>
              )
            }
          >
            May be out of date: {note.suspect.reason} (since {day(note.suspect.since)}). Until it's
            reviewed, the AI is told to check it before relying on it.
          </Alert>
        )}
        {fix && (
          <div className={styles.fix}>
            <div className={styles.fixHead}>
              <Badge tone="ai">{fix.remove ? "Suggested: remove it" : "Suggested fix"}</Badge>
              <span className={styles.meta}>from {fix.app}</span>
            </div>
            <p className={styles.fixText}>{fix.text}</p>
            {fix.evidence && <p className={styles.meta}>{fix.evidence}</p>}
            <div className={styles.fixActions}>
              <Button
                size="sm"
                variant={fix.remove ? "danger" : "primary"}
                disabled={busy}
                onClick={() => onAcceptFix && act(() => onAcceptFix(fix.id))}
              >
                {fix.remove ? "Remove note" : "Use this text"}
              </Button>
              <Button
                size="sm"
                variant="ghost"
                disabled={busy}
                onClick={() => onDismissFix && act(() => onDismissFix(fix.id), false)}
              >
                Dismiss
              </Button>
            </div>
          </div>
        )}
        <Field
          label="Note"
          hint={
            suggest
              ? "A fact the schema can't tell, e.g. “the owner of a tenant is memberships.role = 'owner'”. Once approved, everyone's Ask box and Investigate use it."
              : "A fact the schema can't tell, e.g. “the owner of a tenant is memberships.role = 'owner'”. The Ask box, Investigate and MCP all use it."
          }
        >
          {(id) => (
            <textarea
              id={id}
              className={base.textarea}
              rows={3}
              maxLength={300}
              value={text}
              onChange={(e) => setText(e.target.value)}
              autoFocus
            />
          )}
        </Field>
        {!!note?.anchors?.length && (
          <p className={styles.meta}>
            About{" "}
            {note.anchors.map((a) => (
              <code key={`${a.table}.${a.column ?? ""}`} className={styles.chip}>
                {a.column ? `${a.table}.${a.column}` : a.table}
              </code>
            ))}
          </p>
        )}
        <Field
          label="Check"
          optional
          hint={
            suggest
              ? "Read-only SQL that returns one row and one column: true while the note holds. Owners run it on the team server. Only true or false is kept."
              : "Read-only SQL that returns one row and one column: true while the note holds. SayQL runs it when the note is saved, weekly, and when Investigate asks. Only true or false is kept."
          }
        >
          {(id) => (
            <textarea
              id={id}
              className={base.textarea}
              rows={2}
              maxLength={2000}
              spellCheck={false}
              placeholder="SELECT count(*) = 0 FROM orders WHERE status NOT IN ('new', 'paid', 'cancelled')"
              value={check}
              onChange={(e) => setCheck(e.target.value)}
            />
          )}
        </Field>
        {note?.check && (result || onCheck) && (
          <div className={styles.checkRow}>
            {result && <Badge tone={result.tone}>{result.text}</Badge>}
            {onCheck && (
              <Button
                size="sm"
                variant="ghost"
                disabled={busy || check.trim() !== note.check.sql}
                onClick={() => act(() => onCheck().then(setNote), false)}
              >
                Check now
              </Button>
            )}
          </div>
        )}
        {note?.source && (
          <p className={styles.meta}>
            From {note.source === "SayQL" ? "SayQL" : note.source}
            {note.evidence ? `: ${note.evidence}` : ""}
          </p>
        )}
        {!!note?.history?.length && (
          <details className={styles.history}>
            <summary>Earlier versions ({note.history.length})</summary>
            <ul>
              {note.history.map((h) => (
                <li key={h.at + h.text}>
                  <span className={styles.meta}>{day(h.at)}</span> {h.text}
                </li>
              ))}
            </ul>
          </details>
        )}
        {error && <Alert tone="danger">{error}</Alert>}
      </div>
    </Dialog>
  );
}
