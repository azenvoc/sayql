import { useEffect, useState } from "react";
import { commands, type Bootstrap } from "../api";
import { errorText } from "../lib/errors";

/**
 * Loads saved connections and settings once at startup. Once the first real screen (the
 * workbench, or why SayQL couldn't start) is on the page, tells the app to show this window and
 * close the splash.
 */
export function useBootstrap() {
  const [data, setData] = useState<Bootstrap | null>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    commands.init().then(setData, (e) => setError(errorText(e)));
  }, []);
  const loaded = data !== null || error !== null;
  useEffect(() => {
    // The app shows the window anyway after a few seconds, so a failure here changes nothing.
    if (loaded) commands.appReady().catch(() => undefined);
  }, [loaded]);
  return { data, error };
}
