import { useEffect } from "react";
import {
  commands,
  TEAM_SIGNED_OUT,
  type ConnectionConfig,
  type TeamAccount,
  type TeamAiStatus,
} from "../../api";
import { useLatest } from "../../hooks/useLatest";
import { errorText } from "../../lib/errors";

/**
 * Loads the team's shared connections, and team AI, whenever someone signs in, and clears them on
 * sign-out. Signed out, it asks nothing of anyone: no team server is contacted.
 *
 * Access changes on the server, so it asks again when SayQL comes to the front. A list that hasn't
 * changed isn't applied, since applying one reloads the team's schemas. `onLost` names connections
 * this person no longer has.
 */
export function useTeamConnections(
  account: TeamAccount | null,
  apply: (list: ConnectionConfig[]) => void,
  onError: (message: string) => void,
  onAi: (ai: TeamAiStatus | null) => void = () => {},
  onLost: (lost: ConnectionConfig[]) => void = () => {},
) {
  const latest = useLatest({ apply, onError, onAi, onLost });
  const key = account ? `${account.server} ${account.email}` : null;
  useEffect(() => {
    if (!key) {
      latest.current.onAi(null);
      return latest.current.apply([]);
    }
    let live = true;
    let current: ConnectionConfig[] | null = null;
    const load = (first: boolean) =>
      commands.teamConnections().then(
        (list) => {
          if (!live) return;
          if (JSON.stringify(list) !== JSON.stringify(current)) {
            const lost = (current ?? []).filter((c) => !list.some((n) => n.id === c.id));
            current = list;
            latest.current.apply(list);
            if (lost.length) latest.current.onLost(lost);
          }
          // Fetched with the list on the server side; this just reads what it said.
          commands.teamAi().then(
            (ai) => live && latest.current.onAi(ai),
            () => live && latest.current.onAi(null),
          );
        },
        (e) => {
          const text = errorText(e);
          // Coming back to a server that's out of reach isn't worth a message; being signed out is.
          if (live && (first || text === TEAM_SIGNED_OUT)) latest.current.onError(text);
        },
      );
    void load(true);
    const again = () => void load(false);
    window.addEventListener("focus", again);
    return () => {
      live = false;
      window.removeEventListener("focus", again);
    };
  }, [key, latest]);
}
