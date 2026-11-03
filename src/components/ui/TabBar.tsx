import type { ReactNode } from "react";
import { Plus, X } from "lucide-react";
import { cx } from "../../lib/cx";
import { IconButton } from "./Button";
import { ContextMenu, useContextMenu, type MenuEntry } from "./ContextMenu";
import styles from "./TabBar.module.css";

export interface TabItem {
  id: string;
  label: string;
  /** Secondary text after the label, e.g. the database. */
  detail?: string;
  icon?: ReactNode;
  title?: string;
  /** Shows a dot until hovered, like an unsaved file. */
  dirty?: boolean;
  /** A small badge after the label, e.g. PROD. */
  badge?: ReactNode;
  /** Shown dimmed, e.g. a tab whose connection isn't connected yet. */
  muted?: boolean;
}

export interface TabBarProps {
  items: TabItem[];
  activeId: string | null;
  onSelect: (id: string) => void;
  onClose: (id: string) => void;
  onAdd?: () => void;
  addLabel?: string;
  /** The right-click menu of a tab. */
  menu?: (id: string) => MenuEntry[];
}

/** Editor tabs: click to switch, × or middle-click to close, right-click for more. */
export function TabBar({
  items,
  activeId,
  onSelect,
  onClose,
  onAdd,
  addLabel = "New tab",
  menu,
}: TabBarProps) {
  const context = useContextMenu();
  return (
    <div className={styles.bar}>
      <div className={styles.tabs} role="tablist">
        {items.map((t) => (
          <div
            key={t.id}
            role="tab"
            aria-selected={t.id === activeId}
            title={t.title}
            className={cx(styles.tab, t.id === activeId && styles.active, t.muted && styles.muted)}
            onClick={() => onSelect(t.id)}
            onAuxClick={(e) => e.button === 1 && onClose(t.id)}
            onContextMenu={menu && ((e) => context.open(e, menu(t.id)))}
          >
            {t.icon && <span className={styles.icon}>{t.icon}</span>}
            <span className={styles.label}>
              {t.label}
              {t.detail && <span className={styles.detail}> · {t.detail}</span>}
            </span>
            {t.badge && <span className={styles.badge}>{t.badge}</span>}
            {t.dirty && <span className={styles.dirty} aria-label="Unsaved changes" />}
            <IconButton
              icon={X}
              label={`Close ${t.label}`}
              size="sm"
              className={cx(styles.close, t.dirty && styles.closeDirty)}
              onClick={(e) => {
                e.stopPropagation();
                onClose(t.id);
              }}
            />
          </div>
        ))}
      </div>
      {onAdd && (
        <IconButton icon={Plus} label={addLabel} size="sm" className={styles.add} onClick={onAdd} />
      )}
      {context.menu && (
        <ContextMenu at={context.menu.at} items={context.menu.items} onClose={context.close} />
      )}
    </div>
  );
}
