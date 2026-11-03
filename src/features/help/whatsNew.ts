import { storage } from "../../lib/storage";

/**
 * "What's new", shown once after an update. A short, hand-written summary per version (the
 * full list is CHANGELOG.md); add one when releasing. A version without an entry shows nothing.
 */
export interface WhatsNewEntry {
  version: string;
  items: { title: string; text: string }[];
}

export const WHATS_NEW: WhatsNewEntry[] = [
  {
    version: "0.4.10",
    items: [
      {
        title: "Safety fixes",
        text: "SQL Server's read-only check reads nested comments correctly, and AI apps see saved queries with every value masked.",
      },
    ],
  },
  {
    version: "0.4.9",
    items: [
      {
        title: "Agent mode is now Investigate",
        text: "Same feature, clearer name: switch the Ask box to Investigate or type /investigate (/agent still works).",
      },
      {
        title: "What can reach an AI, at a glance",
        text: "The status bar shows the most an AI can get from the connection you're on. Click it to change it.",
      },
      {
        title: "Connection overview",
        text: "Right-click a connection and choose Overview for its tables, AI access, AI app requests, last audit and your queries.",
      },
      {
        title: "Your activity",
        text: "Settings → Your activity lists the queries you ran, and for Investigate what went to the AI. It stays on this computer.",
      },
    ],
  },
  {
    version: "0.4.8",
    items: [
      {
        title: "Use your Claude plan",
        text: "No API key needed: pick Your Claude plan in Settings → AI provider and SayQL uses your installed Claude Code, on your own Pro or Max plan.",
      },
      {
        title: "Nothing connects on its own",
        text: "Tabs you had open come back, but SayQL connects only when you click Connect, the connection, Run or ask.",
      },
      {
        title: "Production tag",
        text: "Tick Production in a connection's settings and it shows PROD in the navigator, its tabs and the status bar.",
      },
      {
        title: "One set of words for what the AI gets back",
        text: "Agent mode and AI apps now use the same choices: Nothing, Counts and summaries, Rows, masked and All rows. Your settings are unchanged.",
      },
      {
        title: "Help",
        text: "Press ? for keyboard shortcuts, ⌘W closes a tab and ⌘, opens Settings. Each Settings section links to its docs.",
      },
    ],
  },
];

export const WHATS_NEW_SEEN_KEY = "sayql.whatsNewSeen";

/** The entry for `version`, if it has one. */
export const whatsNewFor = (version: string) =>
  WHATS_NEW.find((e) => e.version === version) ?? null;

/**
 * Whether to show What's new on start: once per version, after an update. Someone new
 * (`existingUser` false: never finished or needed the setup) doesn't get it; their version is
 * remembered so the next update shows.
 */
export function shouldShowWhatsNew(
  version: string,
  lastSeen: string | null,
  existingUser: boolean,
): boolean {
  if (lastSeen === version) return false;
  if (lastSeen === null && !existingUser) return false;
  return whatsNewFor(version) !== null;
}

/** Reads and updates the last version whose What's new was seen (or skipped, for someone new). */
export function whatsNewOnStart(
  version: string,
  existingUser: boolean,
): { show: boolean; markSeen: () => void } {
  const lastSeen = storage.get<string | null>(WHATS_NEW_SEEN_KEY, null);
  const show = shouldShowWhatsNew(version, lastSeen, existingUser);
  // Someone new starts counting from this version.
  if (!show && lastSeen !== version) storage.set(WHATS_NEW_SEEN_KEY, version);
  return { show, markSeen: () => storage.set(WHATS_NEW_SEEN_KEY, version) };
}
