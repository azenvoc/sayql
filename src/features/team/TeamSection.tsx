import { useEffect, useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import {
  TEAM_SIGNED_OUT,
  commands,
  type TeamAccount,
  type TeamAiUsage,
  type TeamStatus,
} from "../../api";
import { Alert, Button, Spinner } from "../../components/ui";
import { errorText } from "../../lib/errors";
import { TeamSignInDialog } from "./TeamSignInDialog";
import { TEAMS_URL, licenceText, teamAiName, teamAiOn, usageText } from "./teamText";
import styles from "./Team.module.css";

export interface TeamSectionProps {
  /** Who is signed in, as saved on this computer; null when signed out. */
  account: TeamAccount | null;
  /** Signed in or out. */
  onChange: (account: TeamAccount | null) => void;
}

/** One line of usage: "You: 120k of 2M tokens", with a bar when there's a limit. */
function UsageBar({ label, usage }: { label: string; usage: TeamAiUsage | null }) {
  const share = usage?.limit ? Math.min(1, usage.tokens / usage.limit) : null;
  return (
    <div className={styles.usage}>
      <span>
        {label}: {usageText(usage)}
      </span>
      {share != null && (
        <span
          className={styles.bar}
          role="meter"
          aria-label={`${label}: AI tokens used this month`}
          aria-valuenow={Math.round(share * 100)}
          aria-valuemin={0}
          aria-valuemax={100}
        >
          <span className={styles.fill} style={{ width: `${share * 100}%` }} />
        </span>
      )}
    </div>
  );
}

/** Settings → Team. Signed out it sends nothing anywhere; signed in it reads `/me` once. */
export function TeamSection({ account, onChange }: TeamSectionProps) {
  const [signingIn, setSigningIn] = useState(false);
  const [status, setStatus] = useState<TeamStatus | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    setStatus(null);
    setError(null);
    if (!account) return;
    commands.teamStatus().then(
      // null: the server said the sign-in is no longer valid, and it was forgotten.
      (s) => (s ? setStatus(s) : onChange(null)),
      // A revoked sign-in has been forgotten on this computer by now.
      (e) => (errorText(e) === TEAM_SIGNED_OUT ? onChange(null) : setError(errorText(e))),
    );
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [account?.server, account?.email]);

  const signOut = () => {
    setBusy(true);
    commands
      .teamSignOut()
      .then(
        () => onChange(null),
        (e) => setError(errorText(e)),
      )
      .finally(() => setBusy(false));
  };

  if (!account) {
    return (
      <div className={styles.body}>
        <p className={styles.lead}>
          Using SayQL with a team? Sign in to your company's SayQL server.
        </p>
        <div className={styles.row}>
          <Button variant="primary" onClick={() => setSigningIn(true)}>
            Sign in
          </Button>
          <button type="button" className={styles.link} onClick={() => void openUrl(TEAMS_URL)}>
            SayQL for teams
          </button>
        </div>
        {signingIn && (
          <TeamSignInDialog
            onClose={() => setSigningIn(false)}
            onSignedIn={(a) => {
              setSigningIn(false);
              onChange(a);
            }}
          />
        )}
      </div>
    );
  }

  const shown = status?.account ?? account;
  return (
    <div className={styles.body}>
      <dl className={styles.facts}>
        <dt>Server</dt>
        <dd>{shown.server}</dd>
        <dt>Signed in as</dt>
        <dd>
          {shown.email}
          {shown.role === "admin" ? " (admin)" : ""}
        </dd>
        <dt>Licence</dt>
        <dd>{status ? licenceText(status.server) : error ? "Unknown" : <Spinner size={12} />}</dd>
        {teamAiOn(status?.ai) && (
          <>
            <dt>AI</dt>
            <dd>Provided by your team ({teamAiName(status.ai)})</dd>
            <dt>{status.ai.month ? `Used in ${status.ai.month}` : "Used this month"}</dt>
            <dd>
              <UsageBar label="You" usage={status.ai.you} />
              <UsageBar label="Your team" usage={status.ai.team} />
            </dd>
          </>
        )}
      </dl>
      <p className={styles.lead}>
        Your team's connections are in the Database Navigator under Team. Queries on them, and the
        questions they answer, go to this server, which keeps them in its audit log. Your own
        connections never go there.
        {teamAiOn(status?.ai) &&
          " On team connections, AI requests (your question, the schema and, in Investigate if allowed, masked rows) also go to this server, which forwards them to your team's AI provider and keeps only token counts. Your own connections use your own AI provider."}
      </p>
      {error && <Alert tone="danger">{error}</Alert>}
      <div>
        <Button loading={busy} onClick={signOut}>
          Sign out
        </Button>
      </div>
    </div>
  );
}
