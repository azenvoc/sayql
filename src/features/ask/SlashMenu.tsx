import { useLayoutEffect, useRef, useState, type RefObject } from "react";
import { createPortal } from "react-dom";
import {
  BookOpen,
  Bot,
  CornerDownRight,
  Plus,
  RotateCcw,
  ScrollText,
  Wand2,
  Wrench,
  type LucideIcon,
} from "lucide-react";
import { cx } from "../../lib/cx";
import type { BuiltinCommand, MenuItem } from "./slashCommands";
import styles from "./SlashMenu.module.css";

const ICONS: Record<BuiltinCommand, LucideIcon> = {
  investigate: Bot,
  followup: CornerDownRight,
  new: RotateCcw,
  explain: BookOpen,
  fix: Wrench,
  improve: Wand2,
};

export const itemId = (listId: string, i: number) => `${listId}-${i}`;

export interface SlashMenuProps {
  id: string;
  items: MenuItem[];
  active: number;
  /** The Ask box; the menu opens just below it. */
  anchor: RefObject<HTMLElement | null>;
  onActiveChange: (i: number) => void;
  onPick: (item: MenuItem) => void;
}

/**
 * The "/" autocomplete under the Ask box. Focus stays in the text box, which handles the keys
 * and points at the highlighted item with aria-activedescendant. Looks like ContextMenu.
 */
export function SlashMenu({ id, items, active, anchor, onActiveChange, onPick }: SlashMenuProps) {
  const [pos, setPos] = useState<{ x: number; y: number; width: number } | null>(null);
  const list = useRef<HTMLDivElement>(null);

  useLayoutEffect(() => {
    const r = anchor.current?.getBoundingClientRect();
    if (r) setPos({ x: r.left, y: r.bottom + 4, width: r.width });
  }, [anchor, items.length]);

  useLayoutEffect(() => {
    list.current
      ?.querySelector(`#${CSS.escape(itemId(id, active))}`)
      ?.scrollIntoView({ block: "nearest" });
  }, [id, active]);

  if (!pos) return null;
  return createPortal(
    <div
      ref={list}
      id={id}
      className={styles.menu}
      style={{ left: pos.x, top: pos.y, maxWidth: Math.max(pos.width, 320) }}
      role="listbox"
      aria-label="Commands"
      // Keep focus in the text box.
      onMouseDown={(e) => e.preventDefault()}
    >
      {items.map((it, i) => {
        const create = it.kind === "create";
        const Icon = create
          ? Plus
          : it.kind === "builtin"
            ? ICONS[it.slug as BuiltinCommand]
            : ScrollText;
        return (
          <div key={create ? "+create" : it.slug}>
            {create && <div className={styles.separator} role="separator" />}
            <div
              id={itemId(id, i)}
              role="option"
              aria-selected={i === active}
              className={cx(styles.item, i === active && styles.active)}
              onMouseMove={() => i !== active && onActiveChange(i)}
              onClick={() => onPick(it)}
            >
              <span className={styles.icon}>
                <Icon size={14} />
              </span>
              {create ? (
                <span className={styles.label}>Create skill…</span>
              ) : (
                <>
                  <span className={styles.slug}>/{it.slug}</span>
                  <span className={styles.description}>{it.description}</span>
                </>
              )}
            </div>
          </div>
        );
      })}
    </div>,
    document.body,
  );
}
