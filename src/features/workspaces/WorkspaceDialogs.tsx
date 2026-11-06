import { useState } from "react";
import type { ConnectionConfig, Workspace } from "../../api";
import {
  Alert,
  Button,
  Dialog,
  Field,
  Select,
  SegmentedControl,
  TextInput,
} from "../../components/ui";
import { errorText } from "../../lib/errors";
import { PERSONAL, type WorkspacesApi } from "./useWorkspaces";

/** Asks for a name; a refusal from the backend (a name already used) shows here. */
export function NameDialog({
  title,
  hint,
  initial = "",
  action,
  onSubmit,
  onClose,
}: {
  title: string;
  hint?: string;
  initial?: string;
  action: string;
  onSubmit: (name: string) => Promise<unknown>;
  onClose: () => void;
}) {
  const [name, setName] = useState(initial);
  const [error, setError] = useState<string | null>(null);
  const submit = () => onSubmit(name).then(onClose, (e) => setError(errorText(e)));
  return (
    <Dialog
      title={title}
      size="sm"
      onClose={onClose}
      actions={
        <>
          <Button onClick={onClose}>Cancel</Button>
          <Button variant="primary" onClick={() => void submit()}>
            {action}
          </Button>
        </>
      }
    >
      <Field label="Name" hint={hint}>
        {(id) => (
          <TextInput
            id={id}
            autoFocus
            maxLength={60}
            value={name}
            onChange={(e) => {
              setName(e.target.value);
              setError(null);
            }}
            onKeyDown={(e) => {
              if (e.key === "Enter") void submit();
            }}
          />
        )}
      </Field>
      {error && <Alert tone="danger">{error}</Alert>}
    </Dialog>
  );
}

const groupOptions = (workspaces: WorkspacesApi, workspaceId: string, none: string) => [
  { value: "", label: none },
  ...workspaces.layout.groups
    .filter((g) => g.workspaceId === workspaceId)
    .map((g) => ({ value: g.id, label: g.name })),
];

/** "Move to…" on a connection: another group, or another workspace. */
export function MoveConnectionDialog({
  workspaces,
  connection,
  onClose,
}: {
  workspaces: WorkspacesApi;
  connection: ConnectionConfig;
  onClose: () => void;
}) {
  const from = workspaces.workspaceOf(connection.id);
  const [workspaceId, setWorkspaceId] = useState(from);
  const [groupId, setGroupId] = useState(workspaces.groupOf(connection.id)?.id ?? "");
  const [error, setError] = useState<string | null>(null);
  const name = (id: string) => workspaces.workspaces.find((w) => w.id === id)?.name ?? "";
  const leaving = workspaceId !== from;
  return (
    <Dialog
      title={`Move ${connection.name}`}
      size="sm"
      onClose={onClose}
      actions={
        <>
          <Button onClick={onClose}>Cancel</Button>
          <Button
            variant="primary"
            onClick={() =>
              void workspaces
                .moveConnection(connection.id, workspaceId, groupId || null)
                .then(onClose, (e) => setError(errorText(e)))
            }
          >
            Move
          </Button>
        </>
      }
    >
      {/* A team's connections stay in the workspace chosen in Settings → Team. */}
      {!connection.team && (
        <Field label="Workspace">
          {(id) => (
            <Select
              id={id}
              value={workspaceId}
              options={workspaces.workspaces.map((w) => ({ value: w.id, label: w.name }))}
              onChange={(v) => {
                setWorkspaceId(v);
                setGroupId("");
              }}
            />
          )}
        </Field>
      )}
      <Field label="Group">
        {(id) => (
          <Select
            id={id}
            value={groupId}
            options={groupOptions(workspaces, workspaceId, "No group")}
            onChange={setGroupId}
          />
        )}
      </Field>
      {leaving && (
        <Alert tone="warning">
          Its notes, scripts and access rules move with it. AI apps set up for {name(from)} stop
          seeing it, and only apps set up for {name(workspaceId)} can reach it.
        </Alert>
      )}
      {error && <Alert tone="danger">{error}</Alert>}
    </Dialog>
  );
}

/** "Move to group…" on one database of a server. */
export function MoveDatabaseDialog({
  workspaces,
  connection,
  database,
  onClose,
}: {
  workspaces: WorkspacesApi;
  connection: ConnectionConfig;
  database: string;
  onClose: () => void;
}) {
  const current = workspaces.layout.databaseGroups.find(
    (d) => d.connectionId === connection.id && d.database === database,
  );
  const [groupId, setGroupId] = useState(current?.groupId ?? "");
  const [error, setError] = useState<string | null>(null);
  return (
    <Dialog
      title={`Move ${database}`}
      size="sm"
      onClose={onClose}
      actions={
        <>
          <Button onClick={onClose}>Cancel</Button>
          <Button
            variant="primary"
            onClick={() =>
              void workspaces
                .setDatabaseGroup(connection.id, database, groupId || null)
                .then(onClose, (e) => setError(errorText(e)))
            }
          >
            Move
          </Button>
        </>
      }
    >
      <Field label="Group">
        {(id) => (
          <Select
            id={id}
            value={groupId}
            options={groupOptions(
              workspaces,
              workspaces.workspaceOf(connection.id),
              `With ${connection.name}`,
            )}
            onChange={setGroupId}
          />
        )}
      </Field>
      {error && <Alert tone="danger">{error}</Alert>}
    </Dialog>
  );
}

/** Deleting a workspace: its connections move somewhere else, unless the person says delete them. */
export function DeleteWorkspaceDialog({
  workspaces,
  target,
  onConnectionsDeleted,
  onClose,
}: {
  workspaces: WorkspacesApi;
  target: Workspace;
  /** The connections that went with it, so their tabs close. */
  onConnectionsDeleted: (ids: string[]) => void;
  onClose: () => void;
}) {
  const others = workspaces.workspaces.filter((w) => w.id !== target.id);
  const ids = Object.entries(workspaces.layout.placement)
    .filter(([id, p]) => p.workspaceId === target.id && !id.startsWith("team:"))
    .map(([id]) => id);
  const saved = ids.length;
  const [mode, setMode] = useState<"move" | "delete">("move");
  const [moveTo, setMoveTo] = useState(PERSONAL);
  const [error, setError] = useState<string | null>(null);
  const deleting = mode === "delete" && saved > 0;
  const submit = () =>
    workspaces
      .deleteWorkspace(target.id, mode === "move" ? moveTo : null)
      .then(() => {
        if (mode === "delete") onConnectionsDeleted(ids);
        onClose();
      })
      .catch((e) => setError(errorText(e)));
  return (
    <Dialog
      title={`Delete ${target.name}`}
      size="sm"
      onClose={onClose}
      actions={
        <>
          <Button onClick={onClose}>Cancel</Button>
          <Button variant={deleting ? "danger" : "primary"} onClick={() => void submit()}>
            {deleting
              ? `Delete ${saved} ${saved === 1 ? "connection" : "connections"}`
              : "Delete workspace"}
          </Button>
        </>
      }
    >
      {saved > 0 ? (
        <>
          <Field label={`Its ${saved} saved ${saved === 1 ? "connection" : "connections"}`}>
            {() => (
              <SegmentedControl<"move" | "delete">
                ariaLabel="What happens to its connections"
                value={mode}
                options={[
                  { value: "move", label: "Move them" },
                  { value: "delete", label: "Delete them" },
                ]}
                onChange={setMode}
              />
            )}
          </Field>
          {mode === "move" ? (
            <Field label="Move to">
              {(id) => (
                <Select
                  id={id}
                  value={moveTo}
                  options={others.map((w) => ({ value: w.id, label: w.name }))}
                  onChange={setMoveTo}
                />
              )}
            </Field>
          ) : (
            <Alert tone="danger">
              The saved connections, their passwords, notes and access rules are removed from SayQL.
              The databases themselves aren&apos;t touched.
            </Alert>
          )}
        </>
      ) : (
        <p>It has no saved connections, so only the workspace and its groups go.</p>
      )}
      {error && <Alert tone="danger">{error}</Alert>}
    </Dialog>
  );
}
