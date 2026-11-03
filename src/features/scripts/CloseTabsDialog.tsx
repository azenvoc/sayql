import { Button, Dialog } from "../../components/ui";

export interface CloseTabsDialogProps {
  /** How many tabs are closing. */
  count: number;
  /** The names of those with SQL that isn't saved. */
  unsaved: string[];
  onCancel: () => void;
  /** Closes them all, losing the unsaved SQL. */
  onDiscard: () => void;
  /** Closes only the saved ones and keeps the rest open. */
  onKeepUnsaved: () => void;
}

/** Shown when closing several tabs at once and some have SQL that isn't saved. */
export function CloseTabsDialog({
  count,
  unsaved,
  onCancel,
  onDiscard,
  onKeepUnsaved,
}: CloseTabsDialogProps) {
  const some = unsaved.length === 1 ? "1 tab has" : `${unsaved.length} tabs have`;
  return (
    <Dialog
      title={`Close ${count} tabs?`}
      size="sm"
      onClose={onCancel}
      secondaryActions={
        <Button variant="dangerGhost" onClick={onDiscard}>
          Close all without saving
        </Button>
      }
      actions={
        <>
          <Button onClick={onCancel}>Cancel</Button>
          <Button variant="primary" onClick={onKeepUnsaved} autoFocus>
            Keep unsaved tabs open
          </Button>
        </>
      }
    >
      <p>
        {some} SQL that isn’t saved: {unsaved.map((n) => `“${n}”`).join(", ")}. Closed without
        saving, it’s gone.
      </p>
    </Dialog>
  );
}
