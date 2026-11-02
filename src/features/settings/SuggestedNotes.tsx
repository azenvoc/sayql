import type { ConnectionConfig, NoteSuggestion } from "../../api";
import { NoteSuggestions, TeachSayql, type NoteSuggestionsApi } from "../agent";
import styles from "./ClaudeIntegration.module.css";

export type SuggestedNotesProps = Pick<NoteSuggestionsApi, "suggestions" | "accept" | "dismiss"> & {
  connections: ConnectionConfig[];
  /** Opens the setup on the Access tab, from the empty state. */
  onSetUp?: () => void;
};

/** Notes AI apps suggested over MCP on any connection, waiting to be accepted or dismissed. */
export function SuggestedNotes({
  connections,
  suggestions,
  accept,
  dismiss,
  onSetUp,
}: SuggestedNotesProps) {
  const place = (s: NoteSuggestion) => {
    const name = connections.find((c) => c.id === s.connectionId)?.name ?? "A deleted connection";
    return s.database ? `${name} · ${s.database}` : name;
  };
  return (
    <div className={styles.activity}>
      <p className={styles.intro}>
        AI apps can suggest a note about a database (a table that's no longer used, what a code
        means) with what they checked, and suggest fixing or removing a note that looks out of date.
        Nothing changes until you accept it; accepted notes go to the Ask box, Investigate and every
        AI app using SayQL. You also see a database's suggestions under Notes in the navigator.
      </p>
      <NoteSuggestions
        suggestions={suggestions}
        place={place}
        onAccept={accept}
        onDismiss={dismiss}
      />
      {suggestions.length === 0 && <TeachSayql onSetUp={onSetUp} connections={connections} />}
    </div>
  );
}
