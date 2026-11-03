import { useEffect, useState } from "react";
import { RefreshCw } from "lucide-react";
import { commands, type PlanCheck } from "../../api";
import { Alert, Button } from "../../components/ui";
import { errorText } from "../../lib/errors";

/** "Pro", "Max"… for a plan name Claude Code reports. */
export function planName(check: PlanCheck): string {
  if (check.authMethod !== "claude.ai") return "an API key or cloud account, set up in Claude Code";
  return check.plan
    ? `your Claude ${check.plan[0].toUpperCase()}${check.plan.slice(1)} plan`
    : "your Claude plan";
}

/**
 * Checks the installed Claude Code for "Your Claude plan": recent enough and signed in. Runs on
 * open and on request; `onChecked` hears whether it's ready.
 */
export function ClaudePlanCheck({ onChecked }: { onChecked?: (ok: boolean) => void }) {
  const [check, setCheck] = useState<PlanCheck | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(true);

  const run = async () => {
    setBusy(true);
    setError(null);
    try {
      const c = await commands.checkClaudePlan();
      setCheck(c);
      onChecked?.(true);
    } catch (e) {
      setCheck(null);
      setError(errorText(e));
      onChecked?.(false);
    } finally {
      setBusy(false);
    }
  };

  useEffect(() => {
    void run();
    // Checks once when shown; Check again re-runs it.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  return (
    <div data-testid="claude-plan-check">
      {busy && !check && !error && (
        <Alert tone="info">Checking Claude Code on this computer…</Alert>
      )}
      {check && (
        <Alert tone="success">
          Claude Code {check.version} is signed in with {planName(check)}. SayQL starts it for each
          request, with no tools; it reads only its version and sign-in status, never your
          credentials.
        </Alert>
      )}
      {error && (
        <Alert
          tone="warning"
          actions={
            <Button
              size="sm"
              variant="ghost"
              icon={RefreshCw}
              loading={busy}
              onClick={() => void run()}
            >
              Check again
            </Button>
          }
        >
          {error}
        </Alert>
      )}
    </div>
  );
}
