import { useState } from "react";
import { Check, ChevronDown, ChevronRight, X } from "lucide-react";
import { cx } from "../../lib/cx";
import styles from "./GetStarted.module.css";

export interface ChecklistItem {
  id: string;
  title: string;
  done: boolean;
  /** What the button does, e.g. "Set up AI". */
  action: string;
  onAction: () => void;
}

/**
 * "Get started" under the navigator: the few steps that make SayQL useful, ticked off as they
 * happen. Collapsible; Hide removes it for good. Gone once every step is done.
 */
export function GetStarted({ items, onHide }: { items: ChecklistItem[]; onHide: () => void }) {
  const [open, setOpen] = useState(true);
  const done = items.filter((i) => i.done).length;
  if (done === items.length) return null;
  const next = items.find((i) => !i.done);
  return (
    <section className={styles.panel} aria-label="Get started">
      <header className={styles.header}>
        <button
          type="button"
          className={styles.toggle}
          aria-expanded={open}
          onClick={() => setOpen(!open)}
        >
          {open ? <ChevronDown size={13} /> : <ChevronRight size={13} />}
          Get started
          <span className={styles.count}>
            {done} of {items.length}
          </span>
        </button>
        <button
          type="button"
          className={styles.hide}
          title="Hide Get started"
          aria-label="Hide Get started"
          onClick={onHide}
        >
          <X size={12} />
        </button>
      </header>
      <div className={styles.bar} aria-hidden="true">
        <span style={{ width: `${(done / items.length) * 100}%` }} />
      </div>
      {open && (
        <ol className={styles.items}>
          {items.map((i) => (
            <li key={i.id} className={cx(styles.item, i.done && styles.done)}>
              <span className={styles.check} aria-label={i.done ? "Done" : "To do"}>
                {i.done && <Check size={10} strokeWidth={3} />}
              </span>
              <span className={styles.title}>{i.title}</span>
              {!i.done && (
                <button
                  type="button"
                  className={cx(styles.action, i === next && styles.actionNext)}
                  onClick={i.onAction}
                >
                  {i.action}
                </button>
              )}
            </li>
          ))}
        </ol>
      )}
    </section>
  );
}
