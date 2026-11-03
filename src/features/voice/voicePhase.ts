import type { Schema } from "../../api";
import { qualifiedName } from "../schema";

/** Where voice input is, for one Ask box. */
export type VoicePhase =
  | { kind: "idle" }
  /** First use: the download prompt, then its progress. */
  | { kind: "offer" }
  | { kind: "starting" }
  | { kind: "recording"; level: number }
  | { kind: "transcribing" }
  | { kind: "message"; text: string; tone: "error" | "info" };

export type VoiceAction =
  /** The mic button or shortcut. `ready`: the model is downloaded. */
  | { type: "press"; ready: boolean }
  | { type: "started" }
  | { type: "level"; level: number }
  | { type: "transcribed" }
  | { type: "failed"; message: string }
  | { type: "downloaded" }
  | { type: "cancel" };

export const SHORTCUT = "⌘⇧M";
export const NOTHING_HEARD = "Didn't catch that. Try again.";

export function voiceReducer(phase: VoicePhase, action: VoiceAction): VoicePhase {
  switch (action.type) {
    case "press":
      if (phase.kind === "recording") return { kind: "transcribing" };
      if (phase.kind === "starting" || phase.kind === "transcribing") return phase;
      return action.ready ? { kind: "starting" } : { kind: "offer" };
    case "started":
      return phase.kind === "starting" ? { kind: "recording", level: 0 } : phase;
    case "level":
      return phase.kind === "recording" ? { kind: "recording", level: action.level } : phase;
    case "transcribed":
      return { kind: "idle" };
    case "failed":
      return { kind: "message", text: action.message, tone: "error" };
    case "downloaded":
      return phase.kind === "offer"
        ? {
            kind: "message",
            text: `Voice is ready. Click the mic or press ${SHORTCUT}.`,
            tone: "info",
          }
        : phase;
    case "cancel":
      return { kind: "idle" };
  }
}

/** Whether esc should go to voice input rather than clear the question. */
export function voiceTakesEscape(phase: VoicePhase): boolean {
  return phase.kind !== "idle";
}

/**
 * Words Whisper should expect: table names, then column names (picked tables' first). Whisper
 * leans toward words and spellings it saw in its prompt, so names come out closer to the schema's.
 */
export function voicePrompt(schema: Schema | null, focusTables: string[], maxChars = 600): string {
  if (!schema) return "";
  const picked = schema.tables.filter((t) => focusTables.includes(qualifiedName(t)));
  const words = new Set<string>();
  for (const t of schema.tables) words.add(t.name);
  for (const t of [...picked, ...schema.tables]) t.columns.forEach((c) => words.add(c.name));
  let out = "";
  for (const w of words) {
    const next = out ? `${out}, ${w}` : w;
    if (next.length > maxChars) break;
    out = next;
  }
  return out;
}

/** The question after dictating `transcript`: added after what's typed, if anything. */
export function withTranscript(rest: string, transcript: string): string {
  const text = transcript.trim();
  if (!text) return rest;
  return rest.trim() ? `${rest.trimEnd()} ${text}` : text;
}
