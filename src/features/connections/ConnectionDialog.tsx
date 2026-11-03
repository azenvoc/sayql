import { useState } from "react";
import { ArrowLeft, PlugZap, Trash2 } from "lucide-react";
import { commands, type ConnectionConfig, type DatabaseKind } from "../../api";
import {
  Alert,
  Button,
  Checkbox,
  Dialog,
  Field,
  FieldRow,
  Logo,
  SegmentedControl,
  TextInput,
} from "../../components/ui";
import { errorText } from "../../lib/errors";
import { parseConnectionString } from "./connectionString";
import { DatabasePicker } from "./DatabasePicker";
import { DATABASE_KINDS, KIND_OPTIONS, type DatabaseChoice } from "./kinds";
import styles from "./ConnectionDialog.module.css";

export function newConnection(): ConnectionConfig {
  return {
    id: "",
    name: "",
    kind: "postgres",
    host: "localhost",
    port: 5432,
    database: "",
    user: "",
    file: "",
    ssl: false,
    trustCert: false,
    production: false,
  };
}

function defaultName(c: ConnectionConfig): string {
  if (c.kind === "sqlite") return c.file.split("/").pop() || "SQLite";
  return `${c.database || DATABASE_KINDS[c.kind].label}@${c.host || "localhost"}`;
}

export interface ConnectionDialogProps {
  initial: ConnectionConfig;
  /** Where the details came from, e.g. "From Claude (MCP)". */
  note?: string;
  onClose: () => void;
  onSaved: (c: ConnectionConfig) => void;
  onDeleted: (id: string) => void;
}

export function ConnectionDialog({
  initial,
  note,
  onClose,
  onSaved,
  onDeleted,
}: ConnectionDialogProps) {
  const isNew = !initial.id;
  const [c, setC] = useState(initial);
  // null = leave the password saved in the keychain unchanged.
  const [password, setPassword] = useState<string | null>(isNew ? "" : null);
  const [status, setStatus] = useState<{ ok: boolean; text: string } | null>(null);
  const [testing, setTesting] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [pasted, setPasted] = useState("");
  const [pasteStatus, setPasteStatus] = useState<{ ok: boolean; text: string } | null>(null);
  // A new connection starts on the page of database logos; one prefilled by an AI app doesn't.
  const [picking, setPicking] = useState(isNew && !note);
  const [filter, setFilter] = useState("");
  const [choice, setChoice] = useState<DatabaseChoice | null>(null);
  const set = (patch: Partial<ConnectionConfig>) => setC((prev) => ({ ...prev, ...patch }));

  // Fills the form from a pasted connection string, then clears it so the password isn't left on screen.
  // The password only reaches the keychain when the person clicks Save.
  const fillFrom = (text: string) => {
    const parsed = parseConnectionString(text);
    if (!parsed) {
      setPasteStatus({
        ok: false,
        text: "That doesn't look like a connection string SayQL can read.",
      });
      return;
    }
    set(parsed.fields);
    if (parsed.password !== null) setPassword(parsed.password);
    setPicking(false);
    setChoice(null);
    setPasted("");
    setStatus(null);
    setPasteStatus({
      ok: true,
      text:
        parsed.password !== null
          ? "Filled in, including the password. Check the details, then Test or Save."
          : "Filled in. Enter the password, then Test or Save.",
    });
  };

  const pick = (picked: DatabaseChoice) => {
    const kind = DATABASE_KINDS[picked.kind];
    set({ kind: picked.kind, port: kind.defaultPort, ssl: kind.defaultSsl, ...picked.preset });
    setChoice(picked);
    setPicking(false);
  };

  const test = async () => {
    setTesting(true);
    setStatus(null);
    try {
      setStatus({ ok: true, text: await commands.testConnection(c, password) });
    } catch (e) {
      setStatus({ ok: false, text: errorText(e) });
    } finally {
      setTesting(false);
    }
  };

  const save = async () => {
    try {
      onSaved(
        await commands.saveConnection({ ...c, name: c.name.trim() || defaultName(c) }, password),
      );
    } catch (e) {
      setStatus({ ok: false, text: errorText(e) });
    }
  };

  const remove = async () => {
    if (!confirmDelete) return setConfirmDelete(true);
    try {
      await commands.deleteConnection(c.id);
    } catch (e) {
      setConfirmDelete(false);
      return setStatus({ ok: false, text: `Could not delete the connection: ${errorText(e)}` });
    }
    onDeleted(c.id);
  };

  const connectionString = (
    <>
      <Field
        label="Connection string"
        optional
        hint={`Paste one to fill in ${picking ? "the connection's details" : "the fields below"}. The password is saved to your keychain only when you click Save.`}
      >
        {(id) => (
          <TextInput
            id={id}
            mono
            type="password"
            autoComplete="off"
            spellCheck={false}
            value={pasted}
            placeholder="postgres://…, Server=…;Initial Catalog=…, jdbc:…"
            onChange={(e) => {
              setPasted(e.target.value);
              setPasteStatus(null);
            }}
            onPaste={(e) => {
              const text = e.clipboardData.getData("text");
              if (!text.trim()) return;
              e.preventDefault();
              fillFrom(text);
            }}
            onKeyDown={(e) => {
              if (e.key === "Enter" && pasted.trim()) {
                e.preventDefault();
                fillFrom(pasted);
              }
            }}
          />
        )}
      </Field>
      {pasteStatus && (
        <Alert tone={pasteStatus.ok ? "success" : "warning"}>{pasteStatus.text}</Alert>
      )}
    </>
  );

  if (picking) {
    return (
      <Dialog
        title="New connection"
        size="lg"
        onClose={onClose}
        actions={<Button onClick={onClose}>Cancel</Button>}
      >
        <DatabasePicker filter={filter} onFilter={setFilter} onPick={pick} />
        <div className={styles.or} role="separator">
          or
        </div>
        {connectionString}
      </Dialog>
    );
  }

  // The card picked, while the type still matches it; otherwise the type's own logo and name.
  const shown = choice?.kind === c.kind ? choice : null;
  return (
    <Dialog
      title={isNew ? "New connection" : "Edit connection"}
      onClose={onClose}
      secondaryActions={
        isNew && !note ? (
          <Button variant="ghost" icon={ArrowLeft} onClick={() => setPicking(true)}>
            All databases
          </Button>
        ) : (
          !isNew && (
            <Button
              variant="dangerGhost"
              icon={Trash2}
              onClick={remove}
              title="Removes the saved connection; the database is not touched"
            >
              {confirmDelete ? "Click again to delete" : "Delete"}
            </Button>
          )
        )
      }
      actions={
        <>
          <Button icon={PlugZap} onClick={test} loading={testing}>
            Test
          </Button>
          <Button onClick={onClose}>Cancel</Button>
          <Button variant="primary" onClick={save}>
            Save
          </Button>
        </>
      }
    >
      {note && <Alert tone="info">{note}. Check the details, then enter the password.</Alert>}
      <div className={styles.heading}>
        <Logo id={shown?.logo ?? DATABASE_KINDS[c.kind].logo} size={28} />
        <span>{shown?.name ?? DATABASE_KINDS[c.kind].label}</span>
      </div>
      {connectionString}
      <Field label="Database type">
        {() => (
          <SegmentedControl<DatabaseKind>
            ariaLabel="Database type"
            value={c.kind}
            options={KIND_OPTIONS}
            onChange={(kind) =>
              set({
                kind,
                port: DATABASE_KINDS[kind].defaultPort,
                ssl: DATABASE_KINDS[kind].defaultSsl,
              })
            }
          />
        )}
      </Field>
      <Field label="Name" optional>
        {(id) => (
          <TextInput
            id={id}
            value={c.name}
            placeholder={defaultName(c)}
            onChange={(e) => set({ name: e.target.value })}
          />
        )}
      </Field>
      {c.kind === "sqlite" ? (
        <Field label="Database file">
          {(id) => (
            <TextInput
              id={id}
              mono
              value={c.file}
              placeholder="/path/to/database.sqlite"
              onChange={(e) => set({ file: e.target.value })}
            />
          )}
        </Field>
      ) : (
        <>
          <FieldRow>
            <Field label="Host" grow>
              {(id) => (
                <TextInput
                  id={id}
                  value={c.host}
                  placeholder={DATABASE_KINDS[c.kind].hostPlaceholder}
                  onChange={(e) => set({ host: e.target.value })}
                />
              )}
            </Field>
            <Field label="Port">
              {(id) => (
                <TextInput
                  id={id}
                  style={{ width: 90 }}
                  inputMode="numeric"
                  value={c.port ?? ""}
                  onChange={(e) =>
                    set({ port: e.target.value ? Number(e.target.value.replace(/\D/g, "")) : null })
                  }
                />
              )}
            </Field>
          </FieldRow>
          <Field label="Database">
            {(id) => (
              <TextInput
                id={id}
                value={c.database}
                onChange={(e) => set({ database: e.target.value })}
              />
            )}
          </Field>
          <FieldRow>
            <Field label="User" grow>
              {(id) => (
                <TextInput
                  id={id}
                  autoComplete="off"
                  value={c.user}
                  onChange={(e) => set({ user: e.target.value })}
                />
              )}
            </Field>
            <Field label="Password" grow>
              {(id) => (
                <TextInput
                  id={id}
                  type="password"
                  value={password ?? ""}
                  placeholder={password === null ? "Saved in Keychain" : ""}
                  onChange={(e) => setPassword(e.target.value)}
                />
              )}
            </Field>
          </FieldRow>
          <Checkbox
            label={c.kind === "mssql" ? "Encrypt connection" : "Require SSL"}
            checked={c.ssl}
            onChange={(ssl) => set({ ssl })}
          />
          {c.kind === "mssql" && (
            <Field
              label="Server certificate"
              hint="Internal SQL Servers often use a self-signed certificate. Trust it only for servers you know, never for public ones."
            >
              {() => (
                <Checkbox
                  label="Trust server certificate"
                  checked={c.trustCert}
                  onChange={(trustCert) => set({ trustCert })}
                />
              )}
            </Field>
          )}
        </>
      )}
      <Field
        label="Production"
        hint="Shows a PROD badge on this connection in the navigator, its tabs and the status bar, so it's never mistaken for a test database. It doesn't change how queries run."
      >
        {() => (
          <Checkbox
            label="This is a production database"
            checked={!!c.production}
            onChange={(production) => set({ production })}
          />
        )}
      </Field>
      {status && <Alert tone={status.ok ? "success" : "danger"}>{status.text}</Alert>}
    </Dialog>
  );
}
