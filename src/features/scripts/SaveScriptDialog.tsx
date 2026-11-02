import { useState } from "react";
import { Save } from "lucide-react";
import { Alert, Button, Dialog, Field, TextInput } from "../../components/ui";
import { errorText } from "../../lib/errors";

export interface SaveScriptDialogProps {
  initialName: string;
  onCancel: () => void;
  /** Resolves when saved; a rejection is shown in the dialog. */
  onSave: (name: string) => Promise<void>;
}

/** Asks for a name the first time a tab is saved as a script. */
export function SaveScriptDialog({ initialName, onCancel, onSave }: SaveScriptDialogProps) {
  const [name, setName] = useState(initialName);
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  const save = async () => {
    setSaving(true);
    try {
      await onSave(name);
    } catch (e) {
      setError(errorText(e));
      setSaving(false);
    }
  };

  return (
    <Dialog
      title="Save script"
      size="sm"
      onClose={onCancel}
      actions={
        <>
          <Button onClick={onCancel}>Cancel</Button>
          <Button
            variant="primary"
            icon={Save}
            onClick={save}
            loading={saving}
            disabled={!name.trim()}
          >
            Save
          </Button>
        </>
      }
    >
      <Field
        label="Name"
        hint="Saved scripts appear in the Database Navigator and keep their database."
      >
        {(id) => (
          <TextInput
            id={id}
            autoFocus
            value={name}
            onChange={(e) => setName(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && name.trim() && save()}
          />
        )}
      </Field>
      {error && <Alert tone="danger">{error}</Alert>}
    </Dialog>
  );
}
