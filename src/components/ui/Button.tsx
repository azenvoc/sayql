import type { ButtonHTMLAttributes } from "react";
import { Loader2, type LucideIcon } from "lucide-react";
import { cx } from "../../lib/cx";
import styles from "./Button.module.css";

export type ButtonVariant = "primary" | "secondary" | "ghost" | "danger" | "dangerGhost";
export type ControlSize = "sm" | "md";

export interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: ButtonVariant;
  size?: ControlSize;
  icon?: LucideIcon;
  loading?: boolean;
  /** Keyboard shortcut shown after the label, e.g. "⌘↵". */
  shortcut?: string;
}

export function Button({
  variant = "secondary",
  size = "md",
  icon: Icon,
  loading = false,
  shortcut,
  className,
  children,
  disabled,
  type = "button",
  ...rest
}: ButtonProps) {
  const iconSize = size === "sm" ? 13 : 15;
  return (
    <button
      type={type}
      className={cx(styles.button, styles[variant], size === "sm" && styles.sm, className)}
      disabled={disabled || loading}
      {...rest}
    >
      {loading ? (
        <Loader2 size={iconSize} className={styles.spin} />
      ) : (
        Icon && <Icon size={iconSize} />
      )}
      {children}
      {shortcut && <span className={styles.shortcut}>{shortcut}</span>}
    </button>
  );
}

export interface IconButtonProps extends Omit<ButtonHTMLAttributes<HTMLButtonElement>, "children"> {
  icon: LucideIcon;
  /** Accessible name; also shown as the tooltip. */
  label: string;
  size?: ControlSize;
  active?: boolean;
}

export function IconButton({
  icon: Icon,
  label,
  size = "md",
  active,
  className,
  type = "button",
  ...rest
}: IconButtonProps) {
  return (
    <button
      type={type}
      aria-label={label}
      title={label}
      aria-pressed={active}
      className={cx(
        styles.button,
        styles.ghost,
        styles.iconButton,
        size === "sm" && styles.sm,
        active && styles.active,
        className,
      )}
      {...rest}
    >
      <Icon size={size === "sm" ? 14 : 16} />
    </button>
  );
}
