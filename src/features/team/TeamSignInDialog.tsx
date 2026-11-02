import { useState } from "react";
import { commands, type TeamAccount } from "../../api";
import { Alert, Button, Dialog, Field, TextInput } from "../../components/ui";
import { errorText } from "../../lib/errors";
import { parseInviteLink, serverHost } from "./teamText";
import styles from "./Team.module.css";

export interface TeamSignInDialogProps {
  server?: string;
  code?: string;
  /** Opened from a sayql://team-join link: anyone can make one, so the person checks the server. */
  fromLink?: boolean;
  onSignedIn: (account: TeamAccount) => void;
  onClose: () => void;
}

/**
 * "Sign in to your team": a server address, then the company's sign-in page in the browser or an
 * invite code. Nothing is sent until one of them is clicked. The browser comes back through
 * `sayql://team-sso`, which the window finishes (and closes this dialog).
 */
export function TeamSignInDialog({
  server: initialServer = "",
  code: initialCode = "",
  fromLink,
  onSignedIn,
  onClose,
}: TeamSignInDialogProps) {
  const [server, setServer] = useState(initialServer);
  const [code, setCode] = useState(initialCode);
  /** The server came from a link (opened or pasted), so the person checks it. */
  const [linked, setLinked] = useState(!!fromLink);

  // The whole invite link pasted into either field fills both.
  const onPaste = (value: string, set: (v: string) => void) => {
    const invite = parseInviteLink(value);
    if (!invite) return set(value);
    setServer(invite.server);
    setCode(invite.code);
    setLinked(true);
  };
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  /** "Sign in with your company" opened the browser; the window finishes it when it comes back. */
  const [waiting, setWaiting] = useState(false);
  const [starting, setStarting] = useState(false);

  const signInWithCompany = () => {
    setStarting(true);
    setError(null);
    commands
      .teamSsoStart(server)
      .then(
        () => setWaiting(true),
        (e) => setError(errorText(e)),
      )
      .finally(() => setStarting(false));
  };

  const signIn = () => {
    setBusy(true);
    setError(null);
    commands.teamSignIn(server, code).then(onSignedIn, (e) => {
      setBusy(false);
      setError(errorText(e));
    });
  };

  return (
    <Dialog
      title="Sign in to your team"
      size="sm"
      onClose={onClose}
      actions={
        <>
          <Button onClick={onClose}>Cancel</Button>
          <Button
            variant="primary"
            loading={busy}
            disabled={!server.trim() || !code.trim()}
            onClick={signIn}
          >
            Sign in
          </Button>
        </>
      }
    >
      <div className={styles.body}>
        {linked && (
          <Alert tone="warning">
            An invite link asks you to sign in to <strong>{serverHost(server)}</strong>. Continue
            only if your team admin sent it to you.
          </Alert>
        )}
        <p className={styles.lead}>
          Your company's SayQL server shares its database connections with you. Queries on them, and
          the questions they answer, go to that server, which keeps them in its audit log.
        </p>
        <Field
          label="Server"
          hint="From your team admin, e.g. https://sayql.example.com. Or paste the whole invite link here."
        >
          {(id) => (
            <TextInput
              id={id}
              mono
              value={server}
              placeholder="https://sayql.example.com"
              onChange={(e) => onPaste(e.target.value, setServer)}
              autoFocus={!fromLink}
            />
          )}
        </Field>
        <div className={styles.company}>
          <Button loading={starting} disabled={!server.trim() || busy} onClick={signInWithCompany}>
            Sign in with your company
          </Button>
          <span className={styles.hint}>
            Google, Microsoft or Okta, if your team admin turned it on.
          </span>
        </div>
        {waiting && (
          <Alert tone="info">
            Finish signing in in your browser. SayQL picks up from there when it asks to open; this
            window can stay open.
          </Alert>
        )}
        <Field
          label="Invite code"
          hint="Or an invite code from your team admin. It works once and expires after 7 days."
        >
          {(id) => (
            <TextInput
              id={id}
              mono
              value={code}
              onChange={(e) => onPaste(e.target.value, setCode)}
            />
          )}
        </Field>
        {error && <Alert tone="danger">{error}</Alert>}
      </div>
    </Dialog>
  );
}
