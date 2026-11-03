import { useEffect, useId, useState } from "react";
import {
  ArrowLeft,
  BookOpen,
  Bot,
  ExternalLink,
  History,
  Info,
  Keyboard,
  Mic,
  Plug,
  RefreshCw,
  ScrollText,
  Sparkles,
  Users,
  type LucideIcon,
} from "lucide-react";
import { openUrl } from "@tauri-apps/plugin-opener";
import {
  commands,
  type AiProvider,
  type ConnectionConfig,
  type Settings,
  type TeamAccount,
} from "../../api";
import { Alert, Button, Field, Select, TextInput } from "../../components/ui";
import { cx } from "../../lib/cx";
import { errorText } from "../../lib/errors";
import { AgentSettingsSection, type NoteSuggestionsApi } from "../agent";
import { SkillsSection, type SkillsApi } from "../skills";
import { TeamSection } from "../team";
import { VoiceSection } from "../voice";
import { ClaudeIntegration, type AiAppsTab } from "./ClaudeIntegration";
import { ClaudePlanCheck } from "./ClaudePlanCheck";
import { CreditsBalance } from "./CreditsBalance";
import { MyActivityLog } from "./MyActivity";
import { version } from "../../../package.json";
import { docsUrl } from "../help";
import styles from "./SettingsPage.module.css";

import type { SettingsSection } from "./sections";
export type { SettingsSection };

const SECTIONS: { id: SettingsSection; label: string; icon: LucideIcon }[] = [
  { id: "ai", label: "AI provider", icon: Sparkles },
  { id: "agent", label: "Investigate", icon: Bot },
  { id: "skills", label: "Skills", icon: ScrollText },
  { id: "voice", label: "Voice", icon: Mic },
  { id: "mcp", label: "AI apps", icon: Plug },
  { id: "activity", label: "Your activity", icon: History },
  { id: "team", label: "Team", icon: Users },
  { id: "about", label: "About", icon: Info },
];

export interface SettingsPageProps {
  settings: Settings;
  /** Saved connections, for what AI apps may see on each. */
  connections: ConnectionConfig[];
  onClose: () => void;
  onSaved: (s: Settings) => void;
  /** Reopens the first-run setup. */
  onShowSetup: () => void;
  /** Opens the keyboard shortcut sheet (Settings closes). */
  onShowShortcuts: () => void;
  /** Opens What's new (Settings closes). */
  onShowWhatsNew: () => void;
  skills: SkillsApi;
  /** The section to open on; defaults to the AI provider. */
  section?: SettingsSection;
  /** With section "mcp": the AI apps tab to open on. */
  aiAppsTab?: AiAppsTab;
  /** Notes AI apps suggested, waiting: counted on "AI apps" and listed under its Suggestions tab. */
  noteSuggestions: NoteSuggestionsApi;
  /** Opened from "Create skill…" in the Ask box: start a new skill. */
  createSkill?: boolean;
  /** Signed in to or out of a SayQL Team server. */
  onTeamChange: (account: TeamAccount | null) => void;
}

/** What the form holds for one provider; kept per provider so switching back loses nothing. */
interface Draft {
  baseUrl: string;
  typingModel: string;
  expertModel: string;
  apiKey: string;
}

const draftOf = (p: AiProvider): Draft => ({
  baseUrl: p.baseUrl,
  typingModel: p.typingModel,
  expertModel: p.expertModel,
  apiKey: "",
});

/** SayQL AI credits: Claude through SayQL's service, with a prepaid credit key. */
const SAYQL_CREDITS = "sayql";
/** Your own Claude plan, through your installed Claude Code (src-tauri/src/ai/providers.rs `CLAUDE_PLAN`). */
export const CLAUDE_PLAN = "claude-plan";

/**
 * What the Ask box sends, and where, in two sentences. It must match
 * src-tauri/src/commands/generation.rs: the question, the schema (names, types, keys, enum
 * values, row estimates) and saved notes; the editor's SQL for follow-ups and fixes; a failed
 * query's error with quoted values masked (ai/redact.rs). Rows only go in agent mode, per
 * connection (ai/agent/run.rs).
 */
export function privacyNote(p: Pick<AiProvider, "id" | "key" | "label" | "local">): string {
  const to =
    p.id === SAYQL_CREDITS
      ? "SayQL, which forwards it to Anthropic and keeps only token counts"
      : p.id === CLAUDE_PLAN
        ? "Anthropic by your own Claude Code, on your Claude plan"
        : p.key === "optional"
          ? "your server"
          : p.local
            ? `${p.label}, on this computer`
            : p.label;
  return (
    `Your question, table and column names, your database notes and the SQL you're working on (with a failed query's error, values masked) are sent to ${to}. ` +
    "Data from your tables is sent only if you turn on Investigate for a connection and allow it in its settings."
  );
}

/** Full-window settings, with sections in a sidebar. Esc or Back returns to the workbench. */
export function SettingsPage({
  settings,
  connections,
  onClose,
  onSaved,
  onShowSetup,
  onShowShortcuts,
  onShowWhatsNew,
  skills,
  section: initial = "ai",
  aiAppsTab,
  noteSuggestions,
  createSkill,
  onTeamChange,
}: SettingsPageProps) {
  const [section, setSection] = useState<SettingsSection>(initial);
  const [saved, setSaved] = useState(false);
  const [providerId, setProviderId] = useState(settings.provider);
  const [drafts, setDrafts] = useState<Record<string, Draft>>(() =>
    Object.fromEntries(settings.providers.map((p) => [p.id, draftOf(p)])),
  );
  const [workspaceId, setWorkspaceId] = useState(settings.workspaceId);
  const [models, setModels] = useState<Record<string, string[]>>({});
  const [loadingModels, setLoadingModels] = useState(false);
  const [modelsError, setModelsError] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const listId = useId();

  const provider = settings.providers.find((p) => p.id === providerId) ?? settings.providers[0];
  const draft = drafts[provider.id];
  const update = (patch: Partial<Draft>) => {
    setSaved(false);
    setDrafts((d) => ({ ...d, [provider.id]: { ...d[provider.id], ...patch } }));
  };
  const claudeModels = settings.models.map((m) => ({ value: m.id, label: m.label }));
  // Where agent mode sends results, for agent settings opened from AI apps (local connections only).
  const current = settings.providers.find((p) => p.id === settings.provider);
  const agentProvider = {
    label: current?.label ?? settings.provider,
    local: current?.local ?? false,
  };
  const waiting = noteSuggestions.suggestions.length;
  const found = models[provider.id];

  const chooseProvider = (id: string) => {
    setProviderId(id);
    setModelsError(null);
    setError(null);
    setSaved(false);
  };

  const loadModels = async () => {
    setLoadingModels(true);
    setModelsError(null);
    try {
      const ids = await commands.listModels(
        provider.id,
        draft.baseUrl,
        draft.apiKey.trim() || null,
      );
      setModels((m) => ({ ...m, [provider.id]: ids }));
      if (!draft.typingModel && ids.length > 0) update({ typingModel: ids[0] });
    } catch (e) {
      setModelsError(errorText(e));
    } finally {
      setLoadingModels(false);
    }
  };

  // Esc closes, unless it's meant for a text field.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !(e.target as Element).closest?.("input, textarea, select"))
        onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  const save = async () => {
    setError(null);
    try {
      onSaved(
        await commands.saveSettings({
          provider: provider.id,
          typingModel: draft.typingModel,
          expertModel: draft.expertModel,
          baseUrl: draft.baseUrl,
          apiKey: draft.apiKey.trim() || null,
          workspaceId,
        }),
      );
      setSaved(true);
    } catch (e) {
      setError(errorText(e));
    }
  };

  const modelHint = found
    ? `${found.length} model${found.length === 1 ? "" : "s"} available. Type to filter, or enter any model id.`
    : "Enter a model id, or load the list from the provider.";

  return (
    <div className={styles.page} role="dialog" aria-modal="true" aria-label="Settings">
      <nav className={styles.sidebar} aria-label="Settings sections">
        <button type="button" className={styles.back} onClick={onClose}>
          <ArrowLeft size={14} /> Back
        </button>
        <h1 className={styles.heading}>Settings</h1>
        {SECTIONS.map(({ id, label, icon: Icon }) => (
          <button
            key={id}
            type="button"
            className={cx(styles.navItem, section === id && styles.navActive)}
            aria-current={section === id ? "page" : undefined}
            onClick={() => setSection(id)}
          >
            <Icon size={15} /> {label}
            {id === "mcp" && waiting > 0 && (
              <span
                className={styles.navCount}
                aria-label={`${waiting} suggested ${waiting === 1 ? "note" : "notes"} waiting`}
              >
                {waiting}
              </span>
            )}
          </button>
        ))}
      </nav>
      <main className={styles.content}>
        <div
          className={cx(styles.inner, (section === "mcp" || section === "activity") && styles.wide)}
        >
          {section !== "about" && (
            <button
              type="button"
              className={styles.docsLink}
              title={docsUrl(section)}
              onClick={() => void openUrl(docsUrl(section))}
            >
              Docs <ExternalLink size={12} />
            </button>
          )}
          {section === "ai" && (
            <>
              <h2 className={styles.title}>AI provider</h2>
              <Field label="Provider" hint={provider.note}>
                {(id) => (
                  <Select
                    id={id}
                    value={provider.id}
                    onChange={chooseProvider}
                    options={settings.providers.map((p) => ({
                      value: p.id,
                      label: p.local ? `${p.label} (on this computer)` : p.label,
                    }))}
                  />
                )}
              </Field>
              <p className={styles.intro} data-testid="ai-privacy-note">
                {privacyNote(provider)}
              </p>

              {provider.key !== "none" && (
                <Field
                  label={provider.id === SAYQL_CREDITS ? "Credit key" : `${provider.label} API key`}
                  optional={provider.key === "optional"}
                  hint={`${provider.id === SAYQL_CREDITS ? "From SayQL (hello@azenvoc.com). " : ""}Stored in your system keychain.`}
                >
                  {(id) => (
                    <TextInput
                      id={id}
                      type="password"
                      mono
                      value={draft.apiKey}
                      placeholder={
                        provider.hasKey
                          ? "Saved in Keychain — paste a new key to replace it"
                          : provider.keyPlaceholder || "Paste your API key"
                      }
                      onChange={(e) => update({ apiKey: e.target.value })}
                    />
                  )}
                </Field>
              )}

              {provider.api !== "openai" ? (
                <>
                  {provider.api === "plan" && <ClaudePlanCheck />}
                  {provider.id === SAYQL_CREDITS && provider.hasKey && <CreditsBalance />}
                  {provider.id === "anthropic" && (
                    <Field
                      label="Workspace ID"
                      optional
                      hint="Only for API keys that aren't tied to a workspace. Use the workspace's ID from the Claude Console, not its name."
                    >
                      {(id) => (
                        <TextInput
                          id={id}
                          mono
                          value={workspaceId}
                          placeholder="wrkspc_…"
                          onChange={(e) => setWorkspaceId(e.target.value)}
                        />
                      )}
                    </Field>
                  )}
                  <Field
                    label="Model while typing"
                    hint="Writes SQL as you type. A fast model keeps it feeling instant."
                  >
                    {(id) => (
                      <Select
                        id={id}
                        value={draft.typingModel}
                        onChange={(v) => update({ typingModel: v })}
                        options={claudeModels}
                      />
                    )}
                  </Field>
                  <Field
                    label="Model for fixes and improvements"
                    hint="Fixes a query the database rejected, and reviews a query when you click Improve. A stronger model is worth the extra seconds here."
                  >
                    {(id) => (
                      <Select
                        id={id}
                        value={draft.expertModel}
                        onChange={(v) => update({ expertModel: v })}
                        options={claudeModels}
                      />
                    )}
                  </Field>
                </>
              ) : (
                <>
                  <Field
                    label="Base URL"
                    optional={!!provider.defaultBaseUrl}
                    hint={
                      provider.local
                        ? `Where ${provider.label} listens. Change it only if you run it on another port or machine.`
                        : "The address of the OpenAI-compatible API, usually ending in /v1."
                    }
                  >
                    {(id) => (
                      <TextInput
                        id={id}
                        mono
                        value={draft.baseUrl}
                        placeholder={
                          provider.defaultBaseUrl || "https://your-server.example.com/v1"
                        }
                        onChange={(e) => update({ baseUrl: e.target.value })}
                      />
                    )}
                  </Field>
                  <Field label="Model while typing" hint={modelHint}>
                    {(id) => (
                      <div className={styles.inline}>
                        <TextInput
                          id={id}
                          mono
                          list={listId}
                          value={draft.typingModel}
                          placeholder="Model id"
                          onChange={(e) => update({ typingModel: e.target.value })}
                        />
                        <Button icon={RefreshCw} loading={loadingModels} onClick={loadModels}>
                          Load models
                        </Button>
                      </div>
                    )}
                  </Field>
                  <Field
                    label="Model for fixes and improvements"
                    optional
                    hint="A stronger model for fixing rejected queries and for Improve. Leave empty to use the model above."
                  >
                    {(id) => (
                      <TextInput
                        id={id}
                        mono
                        list={listId}
                        value={draft.expertModel}
                        placeholder="Same as above"
                        onChange={(e) => update({ expertModel: e.target.value })}
                      />
                    )}
                  </Field>
                  <datalist id={listId}>
                    {found?.map((m) => (
                      <option key={m} value={m} />
                    ))}
                  </datalist>
                  {modelsError && <Alert tone="danger">{modelsError}</Alert>}
                </>
              )}

              {error && <Alert tone="danger">{error}</Alert>}
              <div className={styles.actions}>
                {saved && <span className={styles.saved}>Saved</span>}
                <Button variant="primary" onClick={save}>
                  Save
                </Button>
              </div>
            </>
          )}
          {section === "agent" && (
            <>
              <h2 className={styles.title}>Investigate (beta)</h2>
              <AgentSettingsSection settings={settings} onSaved={onSaved} />
            </>
          )}
          {section === "skills" && (
            <>
              <h2 className={styles.title}>Skills</h2>
              <p className={styles.intro}>
                Saved instructions you add to a question by typing / and the name. They're sent to
                the AI with that question, and saved right away.
              </p>
              <SkillsSection skills={skills} startCreating={createSkill} />
            </>
          )}
          {section === "voice" && (
            <>
              <h2 className={styles.title}>Voice</h2>
              <VoiceSection className={styles.intro} />
            </>
          )}
          {section === "mcp" && (
            <>
              <div>
                <h2 className={styles.title}>AI apps</h2>
                <p className={styles.subtitle}>Claude Code, Cursor and other MCP apps</p>
              </div>
              <ClaudeIntegration
                connections={connections}
                provider={agentProvider}
                suggestions={noteSuggestions}
                tab={aiAppsTab}
              />
            </>
          )}
          {section === "activity" && (
            <>
              <h2 className={styles.title}>Your activity</h2>
              <MyActivityLog connections={connections} />
            </>
          )}
          {section === "team" && (
            <>
              <h2 className={styles.title}>Team</h2>
              <TeamSection account={settings.team ?? null} onChange={onTeamChange} />
            </>
          )}
          {section === "about" && (
            <>
              <h2 className={styles.title}>About</h2>
              <p className={styles.about}>
                SayQL Community Edition {version}, a product of Azenvoc. Free and open source under
                the Apache License 2.0. The license, privacy notice and third-party notices are
                included with the app.
              </p>
              <div className={styles.aboutActions}>
                <Button icon={Keyboard} onClick={onShowShortcuts}>
                  Keyboard shortcuts
                </Button>
                <Button icon={Sparkles} onClick={onShowWhatsNew}>
                  What&apos;s new
                </Button>
                <Button icon={BookOpen} onClick={() => void openUrl(docsUrl("about"))}>
                  Docs
                </Button>
                <Button onClick={onShowSetup}>Show the setup guide again</Button>
              </div>
            </>
          )}
        </div>
      </main>
    </div>
  );
}
