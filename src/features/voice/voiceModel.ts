import { useEffect, useSyncExternalStore } from "react";
import { commands, subscribeToVoice } from "../../api";
import { errorText } from "../../lib/errors";

/** The on-device speech model, shared by every Ask box and Settings. */
export interface VoiceModel {
  /** Whether the status has been read from the backend yet. */
  known: boolean;
  ready: boolean;
  downloading: boolean;
  received: number;
  total: number;
  /** Download size, for the first-use prompt. */
  bytes: number;
  error: string | null;
}

let model: VoiceModel = {
  known: false,
  ready: false,
  downloading: false,
  received: 0,
  total: 0,
  bytes: 81_781_811,
  error: null,
};
const listeners = new Set<() => void>();
let canceled = false;
let listening: Promise<unknown> | null = null;

function set(patch: Partial<VoiceModel>) {
  model = { ...model, ...patch };
  listeners.forEach((l) => l());
}

function subscribe(l: () => void) {
  listeners.add(l);
  return () => listeners.delete(l);
}

/** Reads whether the model is downloaded, and starts following download progress. */
export function refreshVoiceModel() {
  listening ??= subscribeToVoice({ progress: ({ received, total }) => set({ received, total }) });
  commands.voiceStatus().then(
    (s) =>
      set({
        known: true,
        ready: s.ready,
        downloading: s.downloading || model.downloading,
        bytes: s.modelBytes,
      }),
    () => set({ known: true }),
  );
}

export async function downloadVoiceModel(): Promise<boolean> {
  if (model.downloading) return false;
  canceled = false;
  set({ downloading: true, received: 0, total: model.bytes, error: null });
  try {
    await commands.voiceDownload();
    set({ downloading: false, ready: true });
    return true;
  } catch (e) {
    set({ downloading: false, error: canceled ? null : errorText(e) });
    return false;
  }
}

export function cancelVoiceDownload() {
  canceled = true;
  void commands.voiceCancelDownload();
}

export async function removeVoiceModel() {
  try {
    await commands.voiceDeleteModel();
    set({ ready: false, error: null });
  } catch (e) {
    set({ error: errorText(e) });
  }
}

export function clearVoiceModelError() {
  set({ error: null });
}

export function useVoiceModel(): VoiceModel {
  useEffect(() => {
    if (!model.known) refreshVoiceModel();
  }, []);
  return useSyncExternalStore(subscribe, () => model);
}

/** "82 MB". */
export function megabytes(bytes: number): string {
  return `${Math.round(bytes / 1_000_000)} MB`;
}
