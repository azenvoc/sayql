import type { AiProvider } from "../../api";
import { Logo } from "../../components/ui";
import { cx } from "../../lib/cx";
import { providerLogo } from "./aiLogos";
import styles from "./ProviderChooser.module.css";

/** Settings → AI provider's choice of provider: a tile with its logo for each one. */
export function ProviderChooser({
  providers,
  value,
  onChange,
}: {
  providers: AiProvider[];
  value: string;
  onChange: (id: string) => void;
}) {
  return (
    <div className={styles.grid} role="radiogroup" aria-label="AI provider">
      {providers.map((p) => (
        <button
          key={p.id}
          type="button"
          role="radio"
          aria-checked={p.id === value}
          className={cx(styles.tile, p.id === value && styles.selected)}
          onClick={() => onChange(p.id)}
          data-testid={`provider-${p.id}`}
        >
          <Logo id={providerLogo(p.id)} size={18} />
          <span className={styles.label}>{p.label}</span>
          {p.local && <span className={styles.local}>On this computer</span>}
        </button>
      ))}
    </div>
  );
}
