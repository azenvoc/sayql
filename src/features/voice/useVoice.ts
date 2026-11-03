import { useEffect, useRef, useState } from "react";
import { commands, subscribeToVoice } from "../../api";
import { errorText } from "../../lib/errors";
import { downloadVoiceModel, useVoiceModel } from "./voiceModel";
import { NOTHING_HEARD, voiceReducer, type VoiceAction, type VoicePhase } from "./voicePhase";

/** Recordings stop on their own after this long, matching the backend's limit. */
const MAX_MS = 120_000;

export interface VoiceOptions {
  /** Whether this Ask box is on the visible tab; only that one answers the shortcut. */
  active: boolean;
  /** Table and column names, to help Whisper spell them. */
  prompt: string;
  onTranscript: (text: string) => void;
}

/** Voice input for one Ask box: the mic button, ⌘⇧M and esc. */
export function useVoice({ active, prompt, onTranscript }: VoiceOptions) {
  const model = useVoiceModel();
  const [phase, setPhase] = useState<VoicePhase>({ kind: "idle" });
  const current = useRef(phase);
  const dispatch = (action: VoiceAction) => {
    current.current = voiceReducer(current.current, action);
    setPhase(current.current);
  };
  const latest = useRef({ prompt, onTranscript, ready: model.ready });
  latest.current = { prompt, onTranscript, ready: model.ready };

  const start = async () => {
    try {
      await commands.voiceStart();
      // Canceled while the microphone was opening.
      if (current.current.kind !== "starting") return void commands.voiceCancel();
      dispatch({ type: "started" });
    } catch (e) {
      dispatch({ type: "failed", message: errorText(e) });
    }
  };

  const stop = async () => {
    try {
      const text = await commands.voiceStop(latest.current.prompt);
      if (current.current.kind !== "transcribing") return;
      if (text.trim()) {
        dispatch({ type: "transcribed" });
        latest.current.onTranscript(text.trim());
      } else {
        dispatch({ type: "failed", message: NOTHING_HEARD });
      }
    } catch (e) {
      if (current.current.kind === "transcribing")
        dispatch({ type: "failed", message: errorText(e) });
    }
  };

  /** The mic button or shortcut: start, stop, or (first time) offer the download. */
  const toggle = () => {
    const before = current.current.kind;
    dispatch({ type: "press", ready: latest.current.ready });
    const after = current.current.kind;
    if (after === "starting" && before !== "starting") void start();
    else if (after === "transcribing" && before === "recording") void stop();
  };

  /** Esc: drops the recording, the prompt or the message. */
  const cancel = () => {
    if (current.current.kind === "recording" || current.current.kind === "starting")
      void commands.voiceCancel();
    dispatch({ type: "cancel" });
  };

  const download = async () => {
    if (await downloadVoiceModel()) dispatch({ type: "downloaded" });
  };

  // Loudness for the meter, and the system blocking the microphone.
  const recording = phase.kind === "recording";
  useEffect(() => {
    if (!recording) return;
    const unlisten = subscribeToVoice({
      level: (level) => dispatch({ type: "level", level }),
      blocked: (message) => {
        void commands.voiceCancel();
        dispatch({ type: "failed", message });
      },
    });
    const limit = setTimeout(toggle, MAX_MS);
    return () => {
      clearTimeout(limit);
      void unlisten.then((u) => u());
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [recording]);

  // ⌘⇧M / Ctrl+Shift+M anywhere on the visible tab; esc while voice has something showing.
  useEffect(() => {
    if (!active) return;
    const onKey = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.shiftKey && e.key.toLowerCase() === "m") {
        e.preventDefault();
        toggle();
      } else if (e.key === "Escape" && !e.defaultPrevented && current.current.kind !== "idle") {
        e.preventDefault();
        cancel();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [active]);

  // A short info message goes away by itself.
  useEffect(() => {
    if (phase.kind !== "message" || phase.tone !== "info") return;
    const t = setTimeout(() => current.current === phase && dispatch({ type: "cancel" }), 4000);
    return () => clearTimeout(t);
  }, [phase]);

  // Leaving the tab or closing it never leaves the microphone on.
  useEffect(() => {
    const kind = current.current.kind;
    if (!active && (kind === "recording" || kind === "starting")) cancel();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [active]);
  useEffect(
    () => () => {
      const kind = current.current.kind;
      if (kind === "recording" || kind === "starting") void commands.voiceCancel();
    },
    [],
  );

  return { phase, model, toggle, cancel, download };
}

export type VoiceApi = ReturnType<typeof useVoice>;
