import { Download, RefreshCw, Trash2 } from "lucide-react";
import { Alert, Button } from "../../components/ui";
import {
  cancelVoiceDownload,
  downloadVoiceModel,
  megabytes,
  removeVoiceModel,
  useVoiceModel,
} from "./voiceModel";
import { SHORTCUT } from "./voicePhase";
import styles from "./VoiceControls.module.css";

/** Settings → Voice: the on-device speech model. */
export function VoiceSection({ className }: { className?: string }) {
  const model = useVoiceModel();
  const pct = model.total ? Math.min(100, Math.round((model.received / model.total) * 100)) : 0;
  const redownload = async () => {
    await removeVoiceModel();
    await downloadVoiceModel();
  };
  return (
    <>
      <p className={className}>
        Click the mic in the Ask box or press {SHORTCUT} to say your question. Speech is turned into
        text on this computer. Audio is never saved or sent anywhere.
      </p>
      <div className={styles.panel}>
        {model.downloading ? (
          <>
            <span className={styles.text}>
              Downloading… {megabytes(model.received)} of {megabytes(model.total || model.bytes)}
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
            <Button size="sm" variant="ghost" onClick={cancelVoiceDownload}>
              Cancel
            </Button>
          </>
        ) : model.ready ? (
          <>
            <span className={styles.text}>
              Speech model (Whisper base.en, {megabytes(model.bytes)}) is downloaded.
            </span>
            <Button size="sm" icon={RefreshCw} onClick={redownload}>
              Redownload
            </Button>
            <Button size="sm" variant="dangerGhost" icon={Trash2} onClick={removeVoiceModel}>
              Remove
            </Button>
          </>
        ) : (
          <>
            <span className={styles.text}>
              Speech model (Whisper base.en, {megabytes(model.bytes)}) isn't downloaded.
            </span>
            <Button
              size="sm"
              variant="primary"
              icon={Download}
              onClick={() => void downloadVoiceModel()}
            >
              Download
            </Button>
          </>
        )}
      </div>
      {model.error && <Alert tone="danger">{model.error}</Alert>}
    </>
  );
}
