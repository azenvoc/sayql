import { createContext, useContext, useEffect, useMemo, useState, type ReactNode } from "react";
import { usePersistentState } from "../../hooks/usePersistentState";

export type ThemePreference = "system" | "light" | "dark";
export type ResolvedTheme = "light" | "dark";

interface ThemeContextValue {
  preference: ThemePreference;
  resolved: ResolvedTheme;
  setPreference: (p: ThemePreference) => void;
}

const ThemeContext = createContext<ThemeContextValue | null>(null);
const darkQuery = () => window.matchMedia("(prefers-color-scheme: dark)");

/** Applies the theme to <html data-theme> and follows the OS setting when set to "system". */
export function ThemeProvider({ children }: { children: ReactNode }) {
  const [preference, setPreference] = usePersistentState<ThemePreference>("theme", "system");
  const [systemDark, setSystemDark] = useState(() => darkQuery().matches);

  useEffect(() => {
    const q = darkQuery();
    const onChange = (e: MediaQueryListEvent) => setSystemDark(e.matches);
    q.addEventListener("change", onChange);
    return () => q.removeEventListener("change", onChange);
  }, []);

  const resolved: ResolvedTheme =
    preference === "system" ? (systemDark ? "dark" : "light") : preference;

  useEffect(() => {
    document.documentElement.dataset.theme = resolved;
  }, [resolved]);

  const value = useMemo(
    () => ({ preference, resolved, setPreference }),
    [preference, resolved, setPreference],
  );
  return <ThemeContext.Provider value={value}>{children}</ThemeContext.Provider>;
}

export function useTheme(): ThemeContextValue {
  const ctx = useContext(ThemeContext);
  if (!ctx) throw new Error("useTheme must be used inside <ThemeProvider>");
  return ctx;
}
