import { useEffect, useState } from "react";
import { createLucideIcon, Lock } from "lucide-react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { commands, type AppInfo } from "../../api";
import { Button, Dialog, Field, SegmentedControl } from "../../components/ui";
import {
  FEEDBACK_FORM_URL,
  githubIssueUrl,
  osLabel,
  privateFormUrl,
  type FeedbackKind,
} from "./links";
import styles from "./FeedbackDialog.module.css";

// lucide-react 1.0 dropped brand icons; this is its last GitHub outline (ISC licence).
const Github = createLucideIcon("Github", [
  [
    "path",
    {
      d: "M15 22v-4a4.8 4.8 0 0 0-1-3.5c3 0 6-2 6-5.5.08-1.25-.27-2.48-1-3.5.28-1.15.28-2.35 0-3.5 0 0-1 0-3 1.5-2.64-.5-5.36-.5-8 0C6 2 5 2 5 2c-.3 1.15-.3 2.35 0 3.5A5.403 5.403 0 0 0 4 9c0 3.5 3 5.5 6 5.5-.39.49-.68 1.05-.85 1.65-.17.6-.22 1.23-.15 1.85v4",
      key: "tonef",
    },
  ],
  ["path", { d: "M9 18c-4.51 2-5-2-7-2", key: "9comsn" }],
]);

const KINDS: { value: FeedbackKind; label: string }[] = [
  { value: "feedback", label: "Feedback" },
  { value: "bug", label: "Something's wrong" },
  { value: "idea", label: "Idea" },
];

/** Opens a pre-filled GitHub issue (public) or the private form, in the browser. */
export function FeedbackDialog({ onClose }: { onClose: () => void }) {
  const [kind, setKind] = useState<FeedbackKind>("feedback");
  const [text, setText] = useState("");
  const [info, setInfo] = useState<AppInfo | null>(null);

  useEffect(() => {
    commands
      .appInfo()
      .then(setInfo, () => setInfo({ version: "unknown", os: "unknown", arch: "unknown" }));
  }, []);

  const open = (url: string) => {
    void openUrl(url);
    onClose();
  };

  return (
    <Dialog
      title="Send feedback"
      onClose={onClose}
      secondaryActions={<Button onClick={onClose}>Cancel</Button>}
      actions={
        <>
          {FEEDBACK_FORM_URL && (
            <Button
              icon={Lock}
              disabled={!info}
              onClick={() => info && open(privateFormUrl(FEEDBACK_FORM_URL, kind, info))}
            >
              Send privately
            </Button>
          )}
          <Button
            variant="primary"
            icon={Github}
            disabled={!info}
            onClick={() => info && open(githubIssueUrl(kind, text, info))}
          >
            Post on GitHub
          </Button>
        </>
      }
    >
      <p className={styles.intro}>
        Every report is read. What you asked, and whether SayQL got the SQL right, helps most.
      </p>
      <SegmentedControl
        ariaLabel="Kind of feedback"
        value={kind}
        options={KINDS}
        onChange={setKind}
      />
      <Field label="What's on your mind?" optional hint="You can add more on the next page.">
        {(id) => (
          <textarea
            id={id}
            className={styles.text}
            rows={5}
            value={text}
            onChange={(e) => setText(e.target.value)}
            placeholder={
              kind === "bug"
                ? "What happened, and what did you expect?"
                : kind === "idea"
                  ? "What are you trying to do?"
                  : "What worked, what didn't?"
            }
          />
        )}
      </Field>
      <p className={styles.privacy}>
        {FEEDBACK_FORM_URL
          ? "Post on GitHub is public and needs a GitHub account; Send privately needs no account. "
          : "This opens a public GitHub issue (you'll need a GitHub account). "}
        Only your SayQL version{info ? ` (${info.version})` : ""} and operating system
        {info ? ` (${osLabel(info)})` : ""} are added. Never include passwords, keys or private
        data.
      </p>
    </Dialog>
  );
}
