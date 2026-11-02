import type { ReactNode } from "react";
import { AlertTriangle, CheckCircle2, Info, Loader2, XCircle, type LucideIcon } from "lucide-react";
import { cx } from "../../lib/cx";
import styles from "./Feedback.module.css";

export type Tone = "neutral" | "accent" | "ai" | "success" | "warning" | "danger";

export function Badge({
  tone = "neutral",
  children,
  title,
}: {
  tone?: Tone;
  children: ReactNode;
  title?: string;
}) {
  return (
    <span className={cx(styles.badge, styles[`badge-${tone}`])} title={title}>
      {children}
    </span>
  );
}

type AlertTone = "info" | "success" | "warning" | "danger";
const ALERT_ICON: Record<AlertTone, LucideIcon> = {
  info: Info,
  success: CheckCircle2,
  warning: AlertTriangle,
  danger: XCircle,
};

export function Alert({
  tone,
  children,
  actions,
}: {
  tone: AlertTone;
  children: ReactNode;
  actions?: ReactNode;
}) {
  const Icon = ALERT_ICON[tone];
  return (
    <div
      className={cx(styles.alert, styles[`alert-${tone}`])}
      role={tone === "danger" ? "alert" : "status"}
    >
      <span className={styles.alertIcon}>
        <Icon size={15} />
      </span>
      <div className={styles.alertBody}>{children}</div>
      {actions && <div className={styles.alertActions}>{actions}</div>}
    </div>
  );
}

export function EmptyState({
  icon: Icon,
  title,
  children,
  action,
}: {
  icon: LucideIcon;
  title: string;
  children?: ReactNode;
  action?: ReactNode;
}) {
  return (
    <div className={styles.empty}>
      <Icon size={32} strokeWidth={1.5} className={styles.emptyIcon} />
      <p className={styles.emptyTitle}>{title}</p>
      {children && <p className={styles.emptyText}>{children}</p>}
      {action}
    </div>
  );
}

export function Spinner({ size = 14 }: { size?: number }) {
  return <Loader2 size={size} className={styles.spinner} aria-label="Loading" />;
}

export function Kbd({ children }: { children: ReactNode }) {
  return <kbd className={styles.kbd}>{children}</kbd>;
}

export function Toolbar({ children, className }: { children: ReactNode; className?: string }) {
  return (
    <div className={cx(styles.toolbar, className)} role="toolbar">
      {children}
    </div>
  );
}

export function ToolbarDivider() {
  return <span className={styles.toolbarDivider} aria-hidden />;
}

export function ToolbarSpacer() {
  return <span className={styles.toolbarSpacer} />;
}
