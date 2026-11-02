import { Eye, EyeOff, Table2, X } from "lucide-react";
import { cx } from "../../lib/cx";
import styles from "./ContextPill.module.css";

export interface ContextPillProps {
  tables: string[];
  /** Off: the selection stays, but the AI doesn't get it. */
  enabled: boolean;
  onEnabledChange: (enabled: boolean) => void;
  onClear: () => void;
}

/** The picked tables as one chip inside the Ask box, like a context attachment in Claude. */
export function ContextPill({ tables, enabled, onEnabledChange, onClear }: ContextPillProps) {
  const label = tables.length <= 2 ? tables.join(", ") : `${tables.length} tables`;
  const list = tables.join("\n");
  return (
    <span
      className={cx(styles.pill, !enabled && styles.off)}
      title={enabled ? `The AI builds on:\n${list}` : `Hidden from the AI:\n${list}`}
    >
      <Table2 size={12} />
      <span className={styles.label}>{label}</span>
      <button
        type="button"
        className={styles.button}
        aria-pressed={enabled}
        aria-label={enabled ? "Hide the selected tables from the AI" : "Use the selected tables"}
        title={enabled ? "Hide from the AI (keeps the selection)" : "Use these tables"}
        onClick={() => onEnabledChange(!enabled)}
      >
        {enabled ? <Eye size={12} /> : <EyeOff size={12} />}
      </button>
      <button
        type="button"
        className={styles.button}
        aria-label="Clear the selected tables"
        title="Clear selection"
        onClick={onClear}
      >
        <X size={12} />
      </button>
    </span>
  );
}
