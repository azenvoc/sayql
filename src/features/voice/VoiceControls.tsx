import { Download, Mic, Square, X } from "lucide-react";
import { Button, IconButton, Kbd, Spinner } from "../../components/ui";
import { cx } from "../../lib/cx";
import type { VoiceModel } from "./voiceModel";
import { megabytes } from "./voiceModel";
import { SHORTCUT, type VoicePhase } from "./voicePhase";
import styles from "./VoiceControls.module.css";

/** The mic on the question line: starts and stops recording. */
export function VoiceButton({
  phase,
  disabled,
  onToggle,
}: {
  phase: VoicePhase;
  disabled?: boolean;
  onToggle: () => void;
}) {
  const recording = phase.kind === "recording" || phase.kind === "starting";
  return (
    <IconButton
      icon={recording ? Square : Mic}
      label={recording ? `Stop (${SHORTCUT})` : `Say your question (${SHORTCUT})`}
      size="sm"
      className={cx(styles.mic, recording && styles.micOn)}
      active={recording}
      disabled={disabled || phase.kind === "transcribing"}
      onClick={onToggle}
    />
  );
}

/** Shown in place of the question while recording and transcribing. */
export function VoiceStrip({ phase }: { phase: VoicePhase }) {
  if (phase.kind === "transcribing") {
    return (
      <span className={styles.strip} role="status">
        <Spinner size={13} /> Transcribing…
      </span>
    );
  }
  if (phase.kind !== "recording" && phase.kind !== "starting") return null;
  const level = phase.kind === "recording" ? phase.level : 0;
  return (
    <span className={styles.strip} role="status">
      <span className={styles.dot} aria-hidden />
      Listening…
      <LevelMeter level={level} />
      <span className={styles.stripHint}>
        <Kbd>{SHORTCUT}</Kbd> done <Kbd>esc</Kbd> cancel
      </span>
    </span>
  );
}

/** Five bars that follow the microphone's loudness. */
function LevelMeter({ level }: { level: number }) {
  // Speech sits around 0.02–0.2 RMS; a square root spreads that across the bars.
  const loud = Math.min(1, Math.sqrt(level) * 2.2);
  return (
    <span className={styles.meter} aria-hidden>
      {[0.5, 0.8, 1, 0.8, 0.5].map((shape, i) => (
        <span
          key={i}
          className={styles.bar}
          style={{ transform: `scaleY(${0.2 + 0.8 * loud * shape})` }}
        />
      ))}
    </span>
  );
}

/** Below the question: the first-use download prompt and its progress, or a short message. */
export function VoicePanel({
  phase,
  model,
  onDownload,
  onCancelDownload,
  onClose,
}: {
  phase: VoicePhase;
  model: VoiceModel;
  onDownload: () => void;
  onCancelDownload: () => void;
  onClose: () => void;
}) {
  if (phase.kind === "message") {
    return (
      <div
        className={cx(styles.panel, phase.tone === "error" && styles.error)}
        role={phase.tone === "error" ? "alert" : "status"}
      >
        <span className={styles.text}>{phase.text}</span>
        <IconButton icon={X} label="Close" size="sm" onClick={onClose} />
      </div>
    );
  }
  if (phase.kind !== "offer") return null;
  if (model.downloading) {
    const pct = model.total ? Math.min(100, Math.round((model.received / model.total) * 100)) : 0;
    return (
      <div className={styles.panel} role="status">
        <span className={styles.text}>
          Downloading voice model… {megabytes(model.received)} of{" "}
          {megabytes(model.total || model.bytes)}
        </span>
        <span
          className={styles.progress}
          role="progressbar"
          aria-valuenow={pct}
          aria-valuemin={0}
          aria-valuemax={100}
        >
          <span className={styles.progressFill} style={{ width: `${pct}%` }} />
        </span>
        <Button size="sm" variant="ghost" onClick={onCancelDownload}>
          Cancel
        </Button>
      </div>
    );
  }
  return (
    <div
      className={cx(styles.panel, model.error && styles.error)}
      role={model.error ? "alert" : "status"}
    >
      <Mic size={13} className={styles.icon} />
      <span className={styles.text}>
        {model.error ??
          `Voice needs a one-time ${megabytes(model.bytes)} download. It runs on this computer.`}
      </span>
      <Button size="sm" variant="primary" icon={Download} onClick={onDownload}>
        {model.error ? "Try again" : "Download"}
      </Button>
      <Button size="sm" variant="ghost" onClick={onClose}>
        Cancel
      </Button>
    </div>
  );
}
