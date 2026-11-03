import { useId, useState } from "react";
import { Check, ChevronRight, Copy, ExternalLink } from "lucide-react";
import { commands } from "../../api";
import { Button, IconButton, Select } from "../../components/ui";
import { cx } from "../../lib/cx";
import { errorText } from "../../lib/errors";
import { AGENTS, type CodingAgent, type ConfigSetup, type SetupContext } from "./agents";
import styles from "./AgentSetup.module.css";

/** "Your coding agent": the same list in the card and in Settings. */
export function AgentPicker({
  agent,
  onChoose,
}: {
  agent: CodingAgent;
  onChoose: (id: string) => void;
}) {
  return (
    <label className={styles.picker}>
      <span className={styles.pickerLabel}>Your coding agent</span>
      <Select
        aria-label="Your coding agent"
        className={styles.select}
        value={agent.id}
        options={AGENTS.map((a) => ({ value: a.id, label: a.name }))}
        onChange={onChoose}
      />
    </label>
  );
}

/** Copies `text`, showing "Copied" for a moment; errors go to `onError`. */
function useCopy(onError: (e: string) => void) {
  const [copied, setCopied] = useState<string | null>(null);
  const copy = (what: string, text: string) =>
    navigator.clipboard.writeText(text).then(
      () => {
        setCopied(what);
        window.setTimeout(() => setCopied(null), 1500);
      },
      (e) => onError(errorText(e)),
    );
  return { copied, copy };
}

function CopyBlock({
  label,
  text,
  copied,
  onCopy,
}: {
  label: string;
  text: string;
  copied: boolean;
  onCopy: () => void;
}) {
  return (
    <div className={styles.block}>
      <div className={styles.blockHead}>
        <span>{label}</span>
        <Button size="sm" variant="ghost" icon={copied ? Check : Copy} onClick={onCopy}>
          {copied ? "Copied" : "Copy"}
        </Button>
      </div>
      <pre className={styles.code}>{text}</pre>
    </div>
  );
}

/** "Add to ~/.cursor/mcp.json", or how to open the file when it has no fixed path. */
function configLabel(config: ConfigSetup): string {
  if (config.file) return `Add to ${config.file}`;
  return config.open ?? "Add to the config";
}

export interface AgentSetupViewProps {
  agent: CodingAgent;
  context: SetupContext;
  /** In the card: the main way first, the config behind a disclosure. */
  compact?: boolean;
}

/** How to add SayQL to the agent: its install link, a command, or its config file. */
export function AgentSetupView({ agent, context, compact = false }: AgentSetupViewProps) {
  const setup = agent.setup(context);
  const [error, setError] = useState<string | null>(null);
  const { copied, copy } = useCopy(setError);
  const [open, setOpen] = useState(false);
  const bodyId = useId();
  const openLink = () => {
    if (!setup.link) return;
    setError(null);
    Promise.resolve()
      .then(() => commands.openInstallLink(setup.link!))
      .catch((e) => setError(errorText(e)));
  };
  const config = setup.config;
  const merge = config?.format === "toml" ? "Add these lines." : "Merge it with what's there.";
  const notes = [
    config?.file && config.open ? `Or open it with ${config.open}.` : null,
    setup.also,
    setup.after,
  ].filter(Boolean);

  if (!compact) {
    return (
      <div className={styles.full}>
        {setup.link && (
          <div className={styles.row}>
            <Button size="sm" variant="primary" icon={ExternalLink} onClick={openLink}>
              Open in {agent.short}
            </Button>
            <span className={styles.hint}>{agent.short} asks before it adds SayQL.</span>
          </div>
        )}
        {setup.command && (
          <CopyBlock
            label="Run in a terminal"
            text={setup.command}
            copied={copied === "command"}
            onCopy={() => copy("command", setup.command!)}
          />
        )}
        {config && (
          <CopyBlock
            label={`${setup.link || setup.command ? "Or add it yourself: " : ""}${configLabel(config)}. ${merge}`}
            text={config.text}
            copied={copied === "config"}
            onCopy={() => copy("config", config.text)}
          />
        )}
        {notes.length > 0 && <p className={styles.hint}>{notes.join(" ")}</p>}
        {error && <p className={styles.error}>{error}</p>}
      </div>
    );
  }

  // Compact: the link or the command, with the config file behind "Or edit the config"; a
  // config-only agent gets Copy config and a disclosure to see it.
  const main = setup.link || setup.command;
  return (
    <div className={styles.compact}>
      {setup.link && (
        <div className={styles.row}>
          <Button size="sm" variant="primary" icon={ExternalLink} onClick={openLink}>
            Open in {agent.short}
          </Button>
          <span className={styles.hint}>It asks before adding SayQL.</span>
        </div>
      )}
      {setup.command && (
        <div className={styles.row}>
          {setup.link ? (
            <span className={styles.hint}>Or run in a terminal:</span>
          ) : (
            <span>Run in a terminal:</span>
          )}
          <code className={styles.line} title={setup.command}>
            {setup.command}
          </code>
          <IconButton
            icon={copied === "command" ? Check : Copy}
            label={copied === "command" ? "Copied" : "Copy the setup command"}
            size="sm"
            onClick={() => copy("command", setup.command!)}
          />
        </div>
      )}
      {config && !main && (
        <div className={styles.row}>
          <span>
            {config.file ? (
              <>
                Add SayQL to <code className={styles.file}>{config.file}</code>
              </>
            ) : (
              <>{config.open}:</>
            )}
          </span>
          <Button
            size="sm"
            variant="secondary"
            icon={copied === "config" ? Check : Copy}
            onClick={() => copy("config", config.text)}
          >
            {copied === "config" ? "Copied" : "Copy config"}
          </Button>
        </div>
      )}
      {config && (
        <>
          <button
            type="button"
            className={styles.disclosure}
            aria-expanded={open}
            aria-controls={bodyId}
            onClick={() => setOpen(!open)}
          >
            <ChevronRight size={12} className={cx(styles.chevron, open && styles.chevronOpen)} />
            {main ? "Or edit the config file" : "Show the config"}
          </button>
          {open && (
            <div id={bodyId} className={styles.compactBody}>
              {main && (
                <div className={styles.row}>
                  <span className={styles.hint}>{configLabel(config)}.</span>
                  <Button
                    size="sm"
                    variant="ghost"
                    icon={copied === "config" ? Check : Copy}
                    onClick={() => copy("config", config.text)}
                  >
                    {copied === "config" ? "Copied" : "Copy config"}
                  </Button>
                </div>
              )}
              <pre className={styles.code}>{config.text}</pre>
              <span className={styles.hint}>{[merge, ...notes].join(" ")}</span>
            </div>
          )}
        </>
      )}
      {!config && notes.length > 0 && <span className={styles.hint}>{notes.join(" ")}</span>}
      {config && !open && setup.after && <span className={styles.hint}>{setup.after}</span>}
      {error && <p className={styles.error}>{error}</p>}
    </div>
  );
}
