import { Cloud, Database, type LucideIcon } from "lucide-react";
import { cx } from "../../lib/cx";
import LOGOS from "../../assets/logos/logos.json";
import styles from "./Logo.module.css";

/** The single-colour logo files, as text: each is one `<path>` on a 24×24 grid (see src/assets/logos/README.md). */
const FILES = import.meta.glob<string>("../../assets/logos/*.svg", {
  query: "?raw",
  import: "default",
  eager: true,
});

/**
 * The brands' own full-colour logos, where their terms allow it, as bundled URLs: `<id>.svg`,
 * and `<id>-dark.svg` when the brand has a version for dark backgrounds.
 */
const COLOR = import.meta.glob<string>("../../assets/logos/color/*.svg", {
  query: "?url",
  import: "default",
  eager: true,
});
const COLOR_URLS = new Map(
  Object.entries(COLOR).map(([file, url]) => [file.replace(/^.*\/|\.svg$/g, ""), url]),
);

interface LogoInfo {
  title: string;
  color: string;
  /** Too dark for the dark theme, where it's drawn in the text color. */
  dark?: boolean;
}

/**
 * Brands with no logo SayQL can ship (no clean, licensed source), shown with a neutral icon in a
 * color of their own instead. The colors are theme tokens.
 */
const NEUTRAL = {
  sqlserver: { title: "SQL Server", icon: Database, color: "var(--color-kind-mssql)" },
  azuresql: { title: "Azure SQL", icon: Cloud, color: "var(--color-kind-mssql)" },
} satisfies Record<string, { title: string; icon: LucideIcon; color: string }>;

export type LogoId = keyof typeof LOGOS | keyof typeof NEUTRAL;

/** The path of each logo file, read once. */
const PATHS = new Map(
  Object.entries(FILES).map(([file, svg]) => [
    file.replace(/^.*\/|\.svg$/g, ""),
    / d="([^"]+)"/.exec(svg)?.[1] ?? "",
  ]),
);

/** A logo's name, e.g. for a label next to it. */
export function logoTitle(id: LogoId): string {
  return id in NEUTRAL
    ? NEUTRAL[id as keyof typeof NEUTRAL].title
    : LOGOS[id as keyof typeof LOGOS].title;
}

/**
 * A database's or AI provider's logo, in its brand color, from the SVGs bundled in
 * src/assets/logos (nothing is fetched). Logos only name the product they belong to.
 * Decorative unless `label` is given, since a name usually sits next to it.
 */
export function Logo({
  id,
  size = 16,
  label,
  className,
}: {
  id: LogoId;
  size?: number;
  label?: string;
  className?: string;
}) {
  const a11y = label ? { role: "img", "aria-label": label } : { "aria-hidden": true as const };
  const color = COLOR_URLS.get(id);
  if (color) {
    const dark = COLOR_URLS.get(`${id}-dark`);
    return (
      <span className={cx(styles.logo, className)} {...a11y}>
        <img
          src={color}
          width={size}
          height={size}
          alt=""
          className={dark ? styles.lightOnly : undefined}
        />
        {dark && <img src={dark} width={size} height={size} alt="" className={styles.darkOnly} />}
      </span>
    );
  }
  if (id in NEUTRAL) {
    const n = NEUTRAL[id as keyof typeof NEUTRAL];
    const Icon = n.icon;
    return (
      <span className={cx(styles.logo, className)} style={{ color: n.color }} {...a11y}>
        <Icon size={size} strokeWidth={1.75} />
      </span>
    );
  }
  const info: LogoInfo = LOGOS[id as keyof typeof LOGOS];
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      className={cx(styles.logo, info.dark && styles.dark, className)}
      {...a11y}
    >
      <path d={PATHS.get(id)} fill={info.color} />
    </svg>
  );
}
