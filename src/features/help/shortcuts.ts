/** Keyboard shortcuts, as the shortcut sheet lists them. Every row must be wired in the code it names. */

export interface Shortcut {
  /** Keys with `Mod` for ⌘ on macOS and Ctrl elsewhere, e.g. ["Mod", "T"]. */
  keys: string[];
  what: string;
}

export interface ShortcutGroup {
  title: string;
  items: Shortcut[];
}

export const SHORTCUTS: ShortcutGroup[] = [
  {
    // src/app/Workbench.tsx (and the macOS menu in src-tauri/src/app_menu.rs for ⌘W and ⌘,)
    title: "Anywhere",
    items: [
      { keys: ["Mod", "T"], what: "New SQL tab on the current database" },
      { keys: ["Mod", "W"], what: "Close the tab" },
      { keys: ["Mod", "S"], what: "Save the tab as a script" },
      { keys: ["Mod", ","], what: "Open Settings" },
      { keys: ["?"], what: "Show these shortcuts (outside a text field)" },
      { keys: ["Esc"], what: "Close Settings or a dialog" },
    ],
  },
  {
    // src/features/ask/AskBar.tsx, src/features/voice
    title: "Ask box",
    items: [
      { keys: ["↵"], what: "Run, once the SQL is written" },
      { keys: ["⇧", "↵"], what: "New line" },
      { keys: ["Esc"], what: "Clear the question" },
      { keys: ["/"], what: "Commands and skills, at the start" },
      { keys: ["Mod", "⇧", "A"], what: "Switch between Quick and Investigate" },
      { keys: ["Mod", "⇧", "M"], what: "Start or stop saying your question" },
    ],
  },
  {
    // src/features/editor (Monaco)
    title: "SQL editor",
    items: [
      { keys: ["Mod", "↵"], what: "Run the query, or only the selected SQL" },
      { keys: ["Ctrl", "Space"], what: "Suggest tables and columns" },
      { keys: ["Mod", "F"], what: "Find" },
      { keys: ["Mod", "/"], what: "Comment or uncomment lines" },
    ],
  },
  {
    // src/features/navigator/DatabaseNavigator.tsx
    title: "Navigator",
    items: [
      { keys: ["Mod", "Click"], what: "Add a table to or remove it from AI context" },
      { keys: ["⇧", "Click"], what: "Pick a range of tables" },
      { keys: ["Double-click"], what: "Preview a table's rows" },
    ],
  },
];

export const isMac = () =>
  typeof navigator !== "undefined" &&
  /Mac|iPhone|iPad/.test(navigator.platform || navigator.userAgent);

/** A key as shown: `Mod` is ⌘ on macOS, Ctrl elsewhere. */
export const keyLabel = (key: string, mac = isMac()) =>
  key === "Mod" ? (mac ? "⌘" : "Ctrl") : key;

/** Whether a key press belongs to a text field (or the SQL editor), so `?` there is just a character. */
export function isTypingTarget(target: EventTarget | null): boolean {
  const el = target as Element | null;
  if (!el || typeof el.closest !== "function") return false;
  return !!el.closest(
    "input, textarea, select, [contenteditable=''], [contenteditable='true'], .monaco-editor",
  );
}
