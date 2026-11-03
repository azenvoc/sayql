import { Monitor, Moon, Sun } from "lucide-react";
import { IconButton } from "../../components/ui";
import { useTheme, type ThemePreference } from "./ThemeProvider";

const NEXT: Record<ThemePreference, ThemePreference> = {
  system: "light",
  light: "dark",
  dark: "system",
};
const ICON = { system: Monitor, light: Sun, dark: Moon };
const LABEL = { system: "Theme: match system", light: "Theme: light", dark: "Theme: dark" };

/** Cycles system → light → dark. */
export function ThemeToggle() {
  const { preference, setPreference } = useTheme();
  return (
    <IconButton
      icon={ICON[preference]}
      label={LABEL[preference]}
      onClick={() => setPreference(NEXT[preference])}
    />
  );
}
