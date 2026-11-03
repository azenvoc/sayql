import { useEffect, useState } from "react";
import {
  commands,
  subscribeToUpdateProgress,
  type UpdateInfo,
  type UpdateProgress,
} from "../../api";
import { errorText } from "../../lib/errors";
import { storage } from "../../lib/storage";

/** The version whose dialog the person asked not to see again; a newer version shows it again. */
const SKIPPED_KEY = "sayql.updates.skippedVersion";

/**
 * Checks the update feed once at startup and opens the update dialog when there's a newer
 * version, unless the person chose "Don't show for version X". `install` downloads, installs
 * and restarts; the status bar's update button opens the dialog again.
 */
export function useUpdate() {
  const [update, setUpdate] = useState<UpdateInfo | null>(null);
  const [open, setOpen] = useState(false);
  const [skipped, setSkippedVersion] = useState<string | null>(() =>
    storage.get<string | null>(SKIPPED_KEY, null),
  );
  const [installing, setInstalling] = useState(false);
  const [progress, setProgress] = useState<UpdateProgress | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    commands.checkUpdate().then(
      (u) => {
        setUpdate(u);
        if (u && u.version !== storage.get<string | null>(SKIPPED_KEY, null)) setOpen(true);
      },
      () => setUpdate(null),
    );
  }, []);

  /** Ticks "Don't show for version X" on or off for the update on offer. */
  const setSkipped = (skip: boolean) => {
    const value = skip && update ? update.version : null;
    storage.set(SKIPPED_KEY, value);
    setSkippedVersion(value);
  };

  const install = async () => {
    setInstalling(true);
    setProgress(null);
    setError(null);
    const unlisten = await subscribeToUpdateProgress(setProgress).catch(() => null);
    try {
      await commands.installUpdate(); // restarts the app on success
    } catch (e) {
      setError(errorText(e));
      setInstalling(false);
    } finally {
      unlisten?.();
    }
  };

  return {
    update,
    /** The update dialog is showing. */
    open,
    show: () => setOpen(true),
    close: () => setOpen(false),
    skipped: update !== null && skipped === update.version,
    setSkipped,
    installing,
    progress,
    error,
    install,
  };
}

export type UpdateState = ReturnType<typeof useUpdate>;
