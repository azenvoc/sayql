import { Button, Dialog, Kbd } from "../../components/ui";
import { SHORTCUTS, keyLabel } from "./shortcuts";
import { WHATS_NEW, whatsNewFor } from "./whatsNew";
import styles from "./HelpDialogs.module.css";

/** Every keyboard shortcut, opened with `?` (outside a text field), the Help menu or Settings → About. */
export function ShortcutSheet({ onClose }: { onClose: () => void }) {
  return (
    <Dialog
      title="Keyboard shortcuts"
      size="lg"
      onClose={onClose}
      actions={<Button onClick={onClose}>Done</Button>}
    >
      <div className={styles.groups}>
        {SHORTCUTS.map((g) => (
          <section key={g.title} aria-label={g.title}>
            <h3 className={styles.groupTitle}>{g.title}</h3>
            <dl className={styles.list}>
              {g.items.map((s) => (
                <div key={s.what} className={styles.row}>
                  <dt className={styles.keys}>
                    {s.keys.map((k, i) => (
                      <Kbd key={i}>{keyLabel(k)}</Kbd>
                    ))}
                  </dt>
                  <dd className={styles.what}>{s.what}</dd>
                </div>
              ))}
            </dl>
          </section>
        ))}
      </div>
    </Dialog>
  );
}

/** What changed in this version: shown once after an update, and from the Help menu or Settings → About. */
export function WhatsNewDialog({ version, onClose }: { version: string; onClose: () => void }) {
  const entry = whatsNewFor(version) ?? WHATS_NEW[0];
  return (
    <Dialog
      title={`What's new in SayQL ${entry.version}`}
      onClose={onClose}
      actions={
        <Button variant="primary" onClick={onClose}>
          Got it
        </Button>
      }
    >
      <ul className={styles.news}>
        {entry.items.map((it) => (
          <li key={it.title}>
            <strong>{it.title}.</strong> {it.text}
          </li>
        ))}
      </ul>
      <p className={styles.more}>
        Every change is listed in the changelog (CHANGELOG.md in SayQL&apos;s source).
      </p>
    </Dialog>
  );
}
