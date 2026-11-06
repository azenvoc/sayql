import { useState } from "react";
import { commands, type ConnectionConfig } from "../../api";
import { Alert, Button, Checkbox, Field, SegmentedControl } from "../../components/ui";
import { errorText } from "../../lib/errors";
import styles from "./ConnectionDialog.module.css";

/**
 * Which of a server's databases this connection may use, for a server shared with other
 * companies or clients. `allowed: null` is every database, the default.
 */
export function AllowedDatabases({
  connection,
  password,
  workspaceName,
  sharedWith,
  allowed,
  onChange,
}: {
  connection: ConnectionConfig;
  password: string | null;
  workspaceName: string;
  /** Another workspace that has a connection to this host. */
  sharedWith: string | null;
  allowed: string[] | null;
  onChange: (allowed: string[] | null) => void;
}) {
  const [listed, setListed] = useState<string[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const limited = allowed !== null;
  const shown = [...new Set([...(allowed ?? []), ...listed])].sort();
  const others = listed.filter((d) => !allowed?.includes(d)).length;

  const load = () => {
    setLoading(true);
    setError(null);
    commands
      .serverDatabases(connection, password)
      .then(
        (list) => {
          setListed(list?.databases ?? []);
          if (!list) setError("This connection has no list of databases.");
        },
        (e) => setError(errorText(e)),
      )
      .finally(() => setLoading(false));
  };

  return (
    <Field
      label="Databases this workspace can use"
      hint="For a server shared with other companies or clients."
    >
      {() => (
        <div className={styles.allowed}>
          <SegmentedControl<"all" | "only">
            ariaLabel="Databases this workspace can use"
            value={limited ? "only" : "all"}
            options={[
              { value: "all", label: "Everything on this server" },
              { value: "only", label: "Only the ones I pick" },
            ]}
            onChange={(mode) => onChange(mode === "only" ? (allowed ?? []) : null)}
          />
          {limited && (
            <>
              <div className={styles.allowedActions}>
                <Button size="sm" loading={loading} onClick={load}>
                  List databases on this server
                </Button>
                <span className={styles.allowedCount}>{allowed.length} selected</span>
              </div>
              {shown.length > 0 && (
                <div className={styles.allowedList}>
                  {shown.map((name) => (
                    <Checkbox
                      key={name}
                      label={name}
                      checked={allowed.includes(name)}
                      onChange={(checked) =>
                        onChange(checked ? [...allowed, name] : allowed.filter((d) => d !== name))
                      }
                    />
                  ))}
                </div>
              )}
              {listed.length > 0 && others > 0 && (
                <Alert tone="warning">
                  This login can still reach {others} other{" "}
                  {others === 1 ? "database" : "databases"}. SayQL hides them in {workspaceName} and
                  refuses AI queries that name them, but only a separate login for this company
                  guarantees it.
                </Alert>
              )}
            </>
          )}
          {!limited && sharedWith && (
            <Alert tone="warning">
              {sharedWith} also uses this server. With everything allowed, its databases would
              appear in {workspaceName} and AI apps could query them. Pick databases, or use a
              separate login.
            </Alert>
          )}
          {error && <Alert tone="danger">{error}</Alert>}
        </div>
      )}
    </Field>
  );
}
