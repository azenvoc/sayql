import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import {
  AGENT_EVENT,
  AI_DELTA_EVENT,
  AI_DONE_EVENT,
  AI_RESET_EVENT,
  AUDIT_EVENT,
  type Reset,
} from "./bindings";
import type { AgentEvent, AiDelta, AiDone, AuditEvent, VoiceProgress } from "./types";

export { AGENT_EVENT, AUDIT_EVENT };

/** Subscribes to agent mode's events; filter them by `runId`. */
export function subscribeToAgent(handler: (e: AgentEvent) => void): Promise<UnlistenFn> {
  return listen<AgentEvent>(AGENT_EVENT, (e) => handler(e.payload));
}

export interface AiEventHandlers {
  delta: (e: AiDelta) => void;
  done: (e: AiDone) => void;
  /** The first model declined and a fallback model restarts the answer. */
  reset: (reqId: number) => void;
}

/** Subscribes to the AI streaming events emitted by src-tauri/src/ai/mod.rs. */
export async function subscribeToAi(handlers: AiEventHandlers): Promise<UnlistenFn> {
  const unlisten = await Promise.all([
    listen<AiDelta>(AI_DELTA_EVENT, (e) => handlers.delta(e.payload)),
    listen<AiDone>(AI_DONE_EVENT, (e) => handlers.done(e.payload)),
    listen<Reset>(AI_RESET_EVENT, (e) => handlers.reset(e.payload.reqId)),
  ]);
  return () => unlisten.forEach((u) => u());
}

/** A menu bar item the page handles (macOS); must match src-tauri/src/app_menu.rs. */
export type MenuAction = "close-tab" | "settings" | "shortcuts" | "whats-new" | "docs";

/** Subscribes to the macOS menu bar's items (src-tauri/src/app_menu.rs `EVENT`). */
export function subscribeToMenu(handler: (action: MenuAction) => void): Promise<UnlistenFn> {
  return listen<MenuAction>("menu://action", (e) => handler(e.payload));
}

/** Calls `onNew` when an AI app hands work to SayQL; fetch it with `commands.takeHandoffs`. */
export function subscribeToHandoffs(onNew: () => void): Promise<UnlistenFn> {
  return listen("handoff://new", onNew);
}

export interface VoiceEventHandlers {
  progress?: (e: VoiceProgress) => void;
  /** Microphone loudness, 0–1, about 20 times a second while recording. */
  level?: (level: number) => void;
  /** The system is withholding the microphone's audio (access denied). */
  blocked?: (message: string) => void;
}

/** Subscribes to the voice input events emitted by src-tauri/src/commands/voice.rs. */
export async function subscribeToVoice(handlers: VoiceEventHandlers): Promise<UnlistenFn> {
  const unlisten = await Promise.all([
    listen<VoiceProgress>("voice://progress", (e) => handlers.progress?.(e.payload)),
    listen<number>("voice://level", (e) => handlers.level?.(e.payload)),
    listen<string>("voice://blocked", (e) => handlers.blocked?.(e.payload)),
  ]);
  return () => unlisten.forEach((u) => u());
}

/** Subscribes to "Run audit" runs' progress; filter it by `runId`. */
export function subscribeToAudit(handler: (e: AuditEvent) => void): Promise<UnlistenFn> {
  return listen<AuditEvent>(AUDIT_EVENT, (e) => handler(e.payload));
}
