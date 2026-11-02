import { useCallback, useEffect, useState } from "react";
import { commands, type DbNote, type NoteSuggestion, type Target } from "../../api";

/** Sent on `window` when a suggestion is accepted or dismissed, so every list and the notes refresh. */
export const SUGGESTIONS_CHANGED = "sayql:note-suggestions-changed";
/** AI apps suggest notes from their own process (sayql-mcp), so the list is read again now and then. */
const REFRESH_MS = 20_000;

/** Whether a suggestion is for the database open in `target`. */
export function suggestionFor(s: NoteSuggestion, target: Target | null): boolean {
  return (
    !!target &&
    s.connectionId === target.connectionId &&
    (s.database ?? null) === (target.database ?? null)
  );
}

/**
 * The notes AI apps suggested over MCP, waiting for the person, read again when SayQL comes to
 * the front and every 20 seconds. Accept saves one as a note; Dismiss drops it.
 */
export function useNoteSuggestions() {
  const [suggestions, setSuggestions] = useState<NoteSuggestion[]>([]);
  const load = useCallback(() => {
    commands.listNoteSuggestions().then(setSuggestions, () => setSuggestions([]));
  }, []);
  useEffect(() => {
    load();
    const timer = window.setInterval(load, REFRESH_MS);
    window.addEventListener("focus", load);
    window.addEventListener(SUGGESTIONS_CHANGED, load);
    return () => {
      window.clearInterval(timer);
      window.removeEventListener("focus", load);
      window.removeEventListener(SUGGESTIONS_CHANGED, load);
    };
  }, [load]);
  const changed = () => window.dispatchEvent(new Event(SUGGESTIONS_CHANGED));
  const accept = async (id: string): Promise<DbNote | null> => {
    const note = await commands.acceptNoteSuggestion(id);
    changed();
    return note;
  };
  const dismiss = async (id: string): Promise<void> => {
    await commands.dismissNoteSuggestion(id);
    changed();
  };
  return { suggestions, accept, dismiss, reload: load };
}

/** What `useNoteSuggestions` returns, for views that show the list one hook already holds. */
export type NoteSuggestionsApi = ReturnType<typeof useNoteSuggestions>;
