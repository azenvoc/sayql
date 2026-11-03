import { AlertTriangle } from "lucide-react";
import { Alert, Button, Dialog } from "../../components/ui";
import styles from "./ConfirmWriteDialog.module.css";

export interface ConfirmWriteDialogProps {
  sql: string;
  connectionName: string;
  onCancel: () => void;
  onConfirm: () => void;
}

/** Shown before any statement SayQL can't confirm only reads data. */
export function ConfirmWriteDialog({
  sql,
  connectionName,
  onCancel,
  onConfirm,
}: ConfirmWriteDialogProps) {
  return (
    <Dialog
      title="Run a statement that may change data?"
      size="sm"
      onClose={onCancel}
      actions={
        <>
          <Button onClick={onCancel} autoFocus>
            Cancel
          </Button>
          <Button variant="danger" icon={AlertTriangle} onClick={onConfirm}>
            Run it
          </Button>
        </>
      }
    >
      <Alert tone="warning">
        SayQL can’t confirm this only reads data on <strong>{connectionName}</strong>. If it changes
        data, SayQL can’t undo it. Review it before running.
      </Alert>
      <pre className={styles.sql}>{sql}</pre>
    </Dialog>
  );
}
