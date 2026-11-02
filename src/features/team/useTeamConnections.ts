import { useEffect } from "react";
import { commands, type ConnectionConfig, type TeamAccount, type TeamAiStatus } from "../../api";
import { useLatest } from "../../hooks/useLatest";
import { errorText } from "../../lib/errors";

/**
 * Loads the team's shared connections, and team AI, whenever someone signs in, and clears them on
 * sign-out. Signed out, it asks nothing of anyone: no team server is contacted.
 */
export function useTeamConnections(
  account: TeamAccount | null,
  apply: (list: ConnectionConfig[]) => void,
  onError: (message: string) => void,
  onAi: (ai: TeamAiStatus | null) => void = () => {},
) {
  const latest = useLatest({ apply, onError, onAi });
  const key = account ? `${account.server} ${account.email}` : null;
  useEffect(() => {
    if (!key) {
      latest.current.onAi(null);
      return latest.current.apply([]);
    }
    let live = true;
    commands.teamConnections().then(
      (list) => {
        if (!live) return;
        latest.current.apply(list);
        // Fetched with the list on the server side; this just reads what it said.
        commands.teamAi().then(
          (ai) => live && latest.current.onAi(ai),
          () => live && latest.current.onAi(null),
        );
      },
      (e) => live && latest.current.onError(errorText(e)),
    );
    return () => {
      live = false;
    };
  }, [key, latest]);
}
