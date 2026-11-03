import { openUrl } from "@tauri-apps/plugin-opener";
import { ArrowDownToLine, ExternalLink } from "lucide-react";
import { Alert, BrandMark, Button, Checkbox, Dialog, Markdown } from "../../components/ui";
import type { UpdateState } from "./useUpdate";
import styles from "./UpdateDialog.module.css";

/** The release page of a version, with its downloads and full notes. */
export function releaseUrl(version: string): string {
  return `https://github.com/azenvoc/sayql-releases/releases/tag/v${version}`;
}

/** A byte count as MB with one decimal. */
function mb(bytes: number): string {
  return `${(bytes / 1_000_000).toFixed(1)} MB`;
}

/**
 * "A new version of SayQL is available": the current and new version, the release notes, and
 * Upgrade, which downloads, installs and restarts. Opens on its own after the startup check, and
 * from the status bar's update button.
 */
export function UpdateDialog({ updates }: { updates: UpdateState }) {
  const { update, installing, progress, error } = updates;
  if (!update) return null;
  const percent =
    progress?.total && progress.total > 0
      ? Math.min(100, Math.round((progress.received / progress.total) * 100))
      : null;
  const status = progress?.installing
    ? "Installing… SayQL restarts when it's done."
    : progress
      ? `Downloading… ${mb(progress.received)}${progress.total ? ` of ${mb(progress.total)}` : ""}`
      : "Starting the download…";
  const openLink = (url: string) => void openUrl(url);

  return (
    <Dialog
      title="Version update"
      size="lg"
      onClose={installing ? () => {} : updates.close}
      secondaryActions={
        <Checkbox
          label={`Don't show for version ${update.version}`}
          checked={updates.skipped}
          onChange={updates.setSkipped}
        />
      }
      actions={
        <>
          <Button icon={ExternalLink} onClick={() => openLink(releaseUrl(update.version))}>
            Details…
          </Button>
          <Button onClick={updates.close} disabled={installing}>
            Close
          </Button>
          <Button
            variant="primary"
            icon={ArrowDownToLine}
            loading={installing}
            onClick={() => void updates.install()}
          >
            {error ? "Try again" : "Upgrade…"}
          </Button>
        </>
      }
    >
      <div className={styles.head}>
        <BrandMark size={44} />
        <div>
          <p className={styles.lead}>A new version of SayQL is available.</p>
          <dl className={styles.versions}>
            <dt>Current version</dt>
            <dd>{update.currentVersion}</dd>
            <dt>New version</dt>
            <dd>
              {update.version}
              {update.date && <span className={styles.date}> ({update.date})</span>}
            </dd>
          </dl>
        </div>
      </div>
      <section className={styles.notes} aria-label="Release notes" tabIndex={0}>
        {update.notes?.trim() ? (
          <Markdown text={update.notes} onLink={openLink} />
        ) : (
          <p className={styles.noNotes}>This release has no notes. Details… opens its page.</p>
        )}
      </section>
      {installing && (
        <div className={styles.progress} role="status" aria-live="polite">
          <progress
            className={styles.bar}
            max={100}
            value={progress?.installing ? 100 : (percent ?? undefined)}
            aria-label="Update download"
          />
          <span className={styles.status}>
            {status}
            {percent !== null && !progress?.installing && ` (${percent}%)`}
          </span>
        </div>
      )}
      {error && <Alert tone="danger">{error}</Alert>}
    </Dialog>
  );
}
