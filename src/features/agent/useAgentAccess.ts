import { useEffect, useState } from "react";
import { commands, type AgentAccess, type AgentAccessSettings } from "../../api";
import { agentLevelShort } from "./resultLevels";

const OFF: AgentAccess = {
  enabled: false,
  results: "off",
  apps: "counts",
  askBeforeSend: true,
  neverSend: [],
  team: null,
};

// Shared by every tab and the navigator, so a change in one shows in all.
const cache = new Map<string, AgentAccess>();
const listeners = new Set<() => void>();
const notify = () => listeners.forEach((l) => l());

/** Saves a connection's agent access and updates everyone showing it. */
export async function saveAgentAccess(
  connectionId: string,
  access: AgentAccessSettings,
): Promise<AgentAccess> {
  const saved = await commands.saveAgentAccess(connectionId, access);
  cache.set(connectionId, saved);
  notify();
  return saved;
}

/** Agent mode's access to a connection: off until the person turns it on. `null` while loading. */
export function useAgentAccess(connectionId: string | null): AgentAccess | null {
  const [, rerender] = useState(0);
  useEffect(() => {
    const l = () => rerender((n) => n + 1);
    listeners.add(l);
    return () => void listeners.delete(l);
  }, []);
  useEffect(() => {
    if (!connectionId || cache.has(connectionId)) return;
    commands.agentAccess(connectionId).then(
      (a) => {
        cache.set(connectionId, a);
        notify();
      },
      () => {
        cache.set(connectionId, OFF);
        notify();
      },
    );
  }, [connectionId]);
  return connectionId ? (cache.get(connectionId) ?? null) : null;
}

/** The setting as the hint under the Ask box says it. */
export function describeAccess(a: AgentAccess): string {
  const results = agentLevelShort(a.results);
  return `Results to AI: ${results}${a.askBeforeSend && a.results !== "off" ? " · Ask before send" : ""}${a.team ? " · Team rules apply" : ""}`;
}
