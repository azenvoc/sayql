import {
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type KeyboardEvent as ReactKeyboardEvent,
  type MouseEvent as ReactMouseEvent,
} from "react";
import { createPortal } from "react-dom";
import { ChevronRight, type LucideIcon } from "lucide-react";
import { cx } from "../../lib/cx";
import styles from "./ContextMenu.module.css";

export type MenuEntry =
  | {
      label: string;
      icon?: LucideIcon;
      /** Shown on the right, e.g. "⌘T". */
      shortcut?: string;
      disabled?: boolean;
      /** Destructive actions are shown in red. */
      danger?: boolean;
      onSelect?: () => void;
      submenu?: MenuEntry[];
    }
  | "separator";

export interface MenuPosition {
  x: number;
  y: number;
}

/** Opens a context menu at the pointer; `menu` is null while it's closed. */
export function useContextMenu() {
  const [menu, setMenu] = useState<{ at: MenuPosition; items: MenuEntry[] } | null>(null);
  return {
    menu,
    open: (e: ReactMouseEvent, items: MenuEntry[]) => {
      e.preventDefault();
      e.stopPropagation();
      setMenu({ at: { x: e.clientX, y: e.clientY }, items });
    },
    close: () => setMenu(null),
  };
}

export interface ContextMenuProps {
  at: MenuPosition;
  items: MenuEntry[];
  onClose: () => void;
}

/** A right-click menu with submenus. Arrow keys move, Enter or → opens, ← and Escape go back. */
export function ContextMenu({ at, items, onClose }: ContextMenuProps) {
  // Close on any outside press, scroll, resize or when the window loses focus.
  useEffect(() => {
    const close = () => onClose();
    const onDown = (e: MouseEvent) => {
      if (!(e.target as Element).closest?.("[data-context-menu]")) onClose();
    };
    window.addEventListener("mousedown", onDown, true);
    window.addEventListener("blur", close);
    window.addEventListener("resize", close);
    window.addEventListener("scroll", close, true);
    return () => {
      window.removeEventListener("mousedown", onDown, true);
      window.removeEventListener("blur", close);
      window.removeEventListener("resize", close);
      window.removeEventListener("scroll", close, true);
    };
  }, [onClose]);

  return createPortal(
    <MenuList items={items} at={at} onClose={onClose} onBack={onClose} autoFocus />,
    document.body,
  );
}

interface MenuListProps {
  items: MenuEntry[];
  at: MenuPosition;
  /** For a submenu: the parent item's left edge, so it can flip left when there's no room. */
  flipFrom?: number;
  onClose: () => void;
  /** Escape or ← in a submenu returns to its parent. */
  onBack: () => void;
  autoFocus?: boolean;
}

function MenuList({ items, at, flipFrom, onClose, onBack, autoFocus }: MenuListProps) {
  const ref = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState(at);
  const [openSub, setOpenSub] = useState<number | null>(null);
  const itemRefs = useRef<(HTMLButtonElement | null)[]>([]);

  // Keep the menu inside the window: flip left or up when it would overflow.
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const { width, height } = el.getBoundingClientRect();
    const margin = 6;
    let x = at.x;
    let y = at.y;
    if (x + width > window.innerWidth - margin)
      x = flipFrom != null ? flipFrom - width : window.innerWidth - margin - width;
    if (y + height > window.innerHeight - margin)
      y = Math.max(margin, window.innerHeight - margin - height);
    setPos({ x: Math.max(margin, x), y });
  }, [at.x, at.y, flipFrom]);

  useEffect(() => {
    if (autoFocus) ref.current?.focus();
  }, [autoFocus]);

  const enabled = items.flatMap((it, i) => (it !== "separator" && !it.disabled ? [i] : []));
  const focusItem = (i: number) => itemRefs.current[i]?.focus();
  const move = (delta: number) => {
    const current = itemRefs.current.findIndex((b) => b === document.activeElement);
    const index = enabled.indexOf(current);
    const next =
      index < 0
        ? delta > 0
          ? 0
          : enabled.length - 1
        : (index + delta + enabled.length) % enabled.length;
    focusItem(enabled[next]);
  };

  const activate = (i: number) => {
    const it = items[i];
    if (it === "separator" || it.disabled) return;
    if (it.submenu) return setOpenSub(i);
    onClose();
    it.onSelect?.();
  };

  const onKeyDown = (e: ReactKeyboardEvent) => {
    const current = itemRefs.current.findIndex((b) => b === document.activeElement);
    switch (e.key) {
      case "ArrowDown":
        move(1);
        break;
      case "ArrowUp":
        move(-1);
        break;
      case "Home":
        focusItem(enabled[0]);
        break;
      case "End":
        focusItem(enabled[enabled.length - 1]);
        break;
      case "ArrowRight":
        if (
          current >= 0 &&
          items[current] !== "separator" &&
          (items[current] as { submenu?: unknown }).submenu
        )
          setOpenSub(current);
        else return;
        break;
      case "ArrowLeft":
      case "Escape":
        onBack();
        break;
      case "Enter":
      case " ":
        if (current >= 0) activate(current);
        break;
      case "Tab":
        break; // keep focus inside the menu
      default:
        return;
    }
    e.preventDefault();
    e.stopPropagation();
  };

  return (
    <div
      ref={ref}
      className={styles.menu}
      style={{ left: pos.x, top: pos.y }}
      role="menu"
      tabIndex={-1}
      data-context-menu
      onKeyDown={onKeyDown}
      onContextMenu={(e) => e.preventDefault()}
    >
      {items.map((it, i) => {
        if (it === "separator")
          return <div key={`sep-${i}`} className={styles.separator} role="separator" />;
        const Icon = it.icon;
        const sub = openSub === i && it.submenu;
        return (
          <div key={it.label} className={styles.itemWrap}>
            <button
              ref={(el) => {
                itemRefs.current[i] = el;
              }}
              type="button"
              role="menuitem"
              aria-haspopup={it.submenu ? "menu" : undefined}
              aria-expanded={it.submenu ? openSub === i : undefined}
              className={cx(styles.item, it.danger && styles.danger, openSub === i && styles.open)}
              disabled={it.disabled}
              onMouseEnter={() => {
                setOpenSub(it.submenu ? i : null);
                itemRefs.current[i]?.focus();
              }}
              onClick={() => activate(i)}
            >
              <span className={styles.icon}>{Icon && <Icon size={14} />}</span>
              <span className={styles.label}>{it.label}</span>
              {it.shortcut && <span className={styles.shortcut}>{it.shortcut}</span>}
              {it.submenu && <ChevronRight size={13} className={styles.chevron} />}
            </button>
            {sub && (
              <SubMenu
                items={it.submenu!}
                anchor={itemRefs.current[i]}
                onClose={onClose}
                onBack={() => {
                  setOpenSub(null);
                  focusItem(i);
                }}
              />
            )}
          </div>
        );
      })}
    </div>
  );
}

function SubMenu({
  items,
  anchor,
  onClose,
  onBack,
}: {
  items: MenuEntry[];
  anchor: HTMLElement | null;
  onClose: () => void;
  onBack: () => void;
}) {
  if (!anchor) return null;
  const r = anchor.getBoundingClientRect();
  return (
    <MenuList
      items={items}
      at={{ x: r.right + 2, y: r.top - 4 }}
      flipFrom={r.left - 2}
      onClose={onClose}
      onBack={onBack}
      autoFocus
    />
  );
}
