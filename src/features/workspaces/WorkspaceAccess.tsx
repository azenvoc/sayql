import { useEffect, useState } from "react";
import { commands } from "../../api";
import { Alert, Button, Field, Select } from "../../components/ui";
import { errorText } from "../../lib/errors";
import type { WorkspacesApi } from "./useWorkspaces";
import styles from "./Workspaces.module.css";

/** Settings → AI apps → Access: what one AI app's setup can see. */
export function WorkspaceAccess({ workspaces }: { workspaces: WorkspacesApi }) {
  const [workspaceId, setWorkspaceId] = useState(workspaces.activeId);
  const [groupId, setGroupId] = useState("");
  const [binary, setBinary] = useState("sayql-mcp");
  useEffect(() => {
    commands.mcpSetup().then(
      (s) => setBinary(s.path),
      () => undefined,
    );
  }, []);
  const groups = workspaces.layout.groups.filter((g) => g.workspaceId === workspaceId);
  const group = groups.find((g) => g.id === groupId);
  const quoted = binary.includes(" ") ? `"${binary}"` : binary;
  const command =
    `claude mcp add sayql-${group?.id ?? workspaceId} -- ${quoted} --workspace ${workspaceId}` +
    (group ? ` --group ${group.id}` : "");
  return (
    <section className={styles.scope}>
      <h3 className={styles.scopeHeading}>What each app can see</h3>
      <p className={styles.scopeIntro}>
        An app sees one workspace, or one group in it. Everything else replies as if it doesn&apos;t
        exist. An app set up without a workspace sees Personal only. Set up one entry per company,
        with this command or these flags in another app&apos;s MCP settings.
      </p>
      <div className={styles.scopeFields}>
        <Field label="Workspace" grow>
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
        <Field label="Limit to a group" optional grow>
          {(id) => (
            <Select
              id={id}
              value={groupId}
              options={[
                { value: "", label: "Whole workspace" },
                ...groups.map((g) => ({ value: g.id, label: g.name })),
              ]}
              onChange={setGroupId}
            />
          )}
        </Field>
      </div>
      <div className={styles.commandRow}>
        <code className={styles.command}>{command}</code>
        <Button size="sm" onClick={() => void navigator.clipboard?.writeText(command)}>
          Copy
        </Button>
      </div>
    </section>
  );
}

/** Settings → Team: which workspace the team's connections appear in. */
export function TeamWorkspace({ workspaces }: { workspaces: WorkspacesApi }) {
  const [error, setError] = useState<string | null>(null);
  if (!workspaces.several) return null;
  return (
    <>
      <Field
        label="Show your team's connections in"
        hint="They appear in one workspace. AI apps over MCP use team keys from your team server, not this."
      >
        {(id) => (
          <Select
            id={id}
            value={workspaces.layout.teamWorkspace ?? "personal"}
            options={workspaces.workspaces.map((w) => ({ value: w.id, label: w.name }))}
            onChange={(v) =>
              void workspaces.setTeamWorkspace(v).catch((e) => setError(errorText(e)))
            }
          />
        )}
      </Field>
      {error && <Alert tone="danger">{error}</Alert>}
    </>
  );
}
