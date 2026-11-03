import { useCallback, useEffect, useState } from "react";
import { commands, type McpApp, type McpSetup } from "../../api";
import { storage } from "../../lib/storage";
import {
  agentById,
  agentForClient,
  preselectAgent,
  type CodingAgent,
  type Os,
  type SetupContext,
} from "./agents";

/** Where the person's pick is kept. */
export const CODING_AGENT_KEY = "sayql.codingAgent";
/** Fired on window when the pick changes, so every picker shows it. */
const CHANGED = "sayql:coding-agent-changed";

// Finding installed agents only checks for files, but once a session is enough.
let installedOnce: Promise<string[]> | null = null;
function installedAgents(): Promise<string[]> {
  installedOnce ??= Promise.resolve()
    .then(() => commands.installedCodingAgents())
    .catch(() => []);
  return installedOnce;
}

/** Looks for installed agents again next time (for tests). */
export function forgetInstalledAgents() {
  installedOnce = null;
}

/** The OS the window runs on, for when the backend doesn't say. */
function guessOs(): Os {
  const p = typeof navigator === "undefined" ? "" : navigator.userAgent;
  return /Windows/i.test(p) ? "windows" : /Mac/i.test(p) ? "macos" : "linux";
}

export interface CodingAgentState {
  agent: CodingAgent;
  /** Remembers the person's pick. */
  choose: (id: string) => void;
  /** Where SayQL's MCP server is; null when it couldn't be found; undefined while loading. */
  setup: McpSetup | null | undefined;
  /** For the setup templates; null without a server path. */
  context: SetupContext | null;
  /** The AI apps that used SayQL over MCP; null while loading. */
  apps: McpApp[] | null;
  /** An app that connected and is this agent, if one did. */
  connectedAs: McpApp | undefined;
}

/** The person's coding agent: picked before, else guessed from what's installed and connected. */
export function useCodingAgent(): CodingAgentState {
  const [saved, setSaved] = useState<string | null>(() =>
    storage.get<string | null>(CODING_AGENT_KEY, null),
  );
  const [installed, setInstalled] = useState<string[]>([]);
  const [apps, setApps] = useState<McpApp[] | null>(null);
  const [setup, setSetup] = useState<McpSetup | null | undefined>(undefined);

  useEffect(() => {
    let live = true;
    installedAgents().then((list) => live && setInstalled(list));
    Promise.resolve()
      .then(() => commands.mcpApps())
      .then(
        (list) => live && setApps(list.apps),
        () => live && setApps([]),
      );
    Promise.resolve()
      .then(() => commands.mcpSetup())
      .then(
        (s) => live && setSetup(s ?? null),
        () => live && setSetup(null),
      );
    const onChange = () => setSaved(storage.get<string | null>(CODING_AGENT_KEY, null));
    window.addEventListener(CHANGED, onChange);
    return () => {
      live = false;
      window.removeEventListener(CHANGED, onChange);
    };
  }, []);

  const choose = useCallback((id: string) => {
    storage.set(CODING_AGENT_KEY, id);
    setSaved(id);
    window.dispatchEvent(new Event(CHANGED));
  }, []);

  const agent = agentById(preselectAgent({ saved, installed, apps: apps ?? [] }))!;
  const context = setup?.path ? { path: setup.path, os: setup.os ?? guessOs() } : null;
  const connectedAs = (apps ?? []).find(
    (a) => a.lastUsed != null && agentForClient(a.name)?.id === agent.id,
  );
  return { agent, choose, setup, context, apps, connectedAs };
}
