import { useEffect, useState } from "react";
import { commands } from "../../api";
import { usePersistentState } from "../../hooks/usePersistentState";
import type { ChecklistItem } from "./GetStarted";

export interface GetStartedState {
  aiReady: boolean;
  /** A tab has an answered question or an investigation now. */
  askedNow: boolean;
  /** A connection other than the demo store exists. */
  ownDatabase: boolean;
  /** Some database has been audited or has notes. */
  taught: boolean;
}

export interface GetStartedActions {
  setUpAi: () => void;
  tryIt: () => void;
  connectDatabase: () => void;
  connectApp: () => void;
  teach: () => void;
}

/**
 * The Get started checklist's steps, ticked off from what has happened. Remembers a first
 * question across launches (tabs come and go), and asks whether an AI app has used SayQL on
 * start and whenever SayQL comes back to the front.
 */
export function useGetStarted(state: GetStartedState, actions: GetStartedActions) {
  const [hidden, setHidden] = usePersistentState("sayql.checklistHidden", false);
  const [askedOnce, setAskedOnce] = usePersistentState("sayql.askedOnce", false);
  const [appConnected, setAppConnected] = useState(false);

  useEffect(() => {
    if (state.askedNow && !askedOnce) setAskedOnce(true);
  }, [state.askedNow, askedOnce, setAskedOnce]);

  useEffect(() => {
    if (hidden) return;
    const check = () =>
      void commands.mcpApps().then(
        (a) => setAppConnected(a.apps.length > 0),
        () => {},
      );
    check();
    window.addEventListener("focus", check);
    return () => window.removeEventListener("focus", check);
  }, [hidden]);

  const items: ChecklistItem[] = [
    {
      id: "ai",
      title: "Choose how to use AI",
      done: state.aiReady,
      action: "Set up",
      onAction: actions.setUpAi,
    },
    {
      id: "ask",
      title: "Ask your first question",
      done: askedOnce || state.askedNow,
      action: "Try it",
      onAction: actions.tryIt,
    },
    {
      id: "database",
      title: "Connect your own database",
      done: state.ownDatabase,
      action: "Connect",
      onAction: actions.connectDatabase,
    },
    {
      id: "app",
      title: "Let an AI app in safely",
      done: appConnected,
      action: "Connect",
      onAction: actions.connectApp,
    },
    {
      id: "teach",
      title: "Teach SayQL a database",
      done: state.taught,
      action: "Teach",
      onAction: actions.teach,
    },
  ];
  return { items, hidden, hide: () => setHidden(true) };
}
