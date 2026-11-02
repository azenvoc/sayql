import { useEffect, useRef, type ReactNode } from "react";
import { X } from "lucide-react";
import { cx } from "../../lib/cx";
import { IconButton } from "./Button";
import styles from "./Dialog.module.css";

export interface DialogProps {
  title: string;
  onClose: () => void;
  children: ReactNode;
  /** Buttons on the right of the footer. */
  actions?: ReactNode;
  /** Content on the left of the footer (e.g. a destructive action). */
  secondaryActions?: ReactNode;
  size?: "sm" | "md" | "lg";
}

const FOCUSABLE =
  'a[href], button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])';

/**
 * Modal dialog. Closes on Escape or a click on the backdrop. Escape closes only the dialog, not
 * the page behind it, and Tab stays inside the dialog until it closes.
 */
export function Dialog({
  title,
  onClose,
  children,
  actions,
  secondaryActions,
  size = "md",
}: DialogProps) {
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const dialog = ref.current;
      if (!dialog) return;
      if (e.key === "Escape") {
        e.stopImmediatePropagation();
        onClose();
      } else if (e.key === "Tab") {
        const items = [...dialog.querySelectorAll<HTMLElement>(FOCUSABLE)];
        if (items.length === 0) return;
        const first = items[0];
        const last = items[items.length - 1];
        const inside = dialog.contains(document.activeElement);
        if (e.shiftKey && (!inside || document.activeElement === first)) {
          e.preventDefault();
          last.focus();
        } else if (!e.shiftKey && (!inside || document.activeElement === last)) {
          e.preventDefault();
          first.focus();
        }
      }
    };
    // Capture, so the page behind never sees an Escape meant for the dialog.
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [onClose]);

  useEffect(() => {
    const before = document.activeElement as HTMLElement | null;
    const dialog = ref.current;
    if (dialog && !dialog.contains(document.activeElement)) {
      (dialog.querySelector<HTMLElement>("[autofocus]") ?? dialog).focus();
    }
    return () => before?.focus?.();
  }, []);

  return (
    <div className={styles.backdrop} onMouseDown={onClose}>
      <div
        className={cx(styles.dialog, styles[size])}
        ref={ref}
        tabIndex={-1}
        role="dialog"
        aria-modal="true"
        aria-label={title}
        onMouseDown={(e) => e.stopPropagation()}
      >
        <header className={styles.header}>
          <h2 className={styles.title}>{title}</h2>
          <IconButton icon={X} label="Close" size="sm" onClick={onClose} />
        </header>
        <div className={styles.body}>{children}</div>
        {(actions || secondaryActions) && (
          <footer className={styles.footer}>
            {secondaryActions}
            <span className={styles.spacer} />
            {actions}
          </footer>
        )}
      </div>
    </div>
  );
}
