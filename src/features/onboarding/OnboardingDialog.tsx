import { useState, type ReactNode } from "react";
import {
  ArrowLeft,
  ArrowRight,
  Bot,
  Check,
  Database,
  HardDrive,
  MessageSquareText,
  Minus,
  Plug,
  Plus,
  ShieldCheck,
  Sparkles,
  Wand2,
} from "lucide-react";
import { commands, type AiProvider, type Settings } from "../../api";
import {
  Alert,
  BrandMark,
  Button,
  Dialog,
  Field,
  Logo,
  Select,
  TextInput,
  type LogoId,
} from "../../components/ui";
import { cx } from "../../lib/cx";
import { ClaudePlanCheck, providerLogo } from "../settings";
import { errorText } from "../../lib/errors";
import styles from "./OnboardingDialog.module.css";

export type OnboardingStep = "welcome" | "ai" | "database" | "try";
const STEPS: OnboardingStep[] = ["welcome", "ai", "database", "try"];

/** SayQL AI credits (src-tauri/src/ai/providers.rs `SAYQL_CREDITS`): Claude through SayQL with a prepaid key. */
const SAYQL_CREDITS = "sayql";
/** API-key providers shown as cards; every other one is in the "More providers" list. */
const FEATURED = ["anthropic", "openai", "gemini"];

export interface OnboardingDialogProps {
  settings: Settings;
  step: OnboardingStep;
  onStep: (step: OnboardingStep) => void;
  onSettingsSaved: (s: Settings) => void;
  /** False when the person deleted the demo connection. */
  demoAvailable: boolean;
  onUseDemo: () => void;
  onConnectOwn: () => void;
  /** Starter questions for the database picked in the previous step. */
  examples: string[];
  /** Opens a tab that asks `question` and runs the answer; ends the setup. */
  onTry: (question: string) => void;
  onOpenSettings: () => void;
  /** Ends the setup and opens Settings → AI apps → Access, to connect Claude Code, Cursor or another app. */
  onOpenAiApps: () => void;
  /** Ends the setup and opens "Teach SayQL your database"; absent when there's no database to teach. */
  onTeach?: () => void;
  /** Ends the setup (finished, skipped or closed). */
  onFinish: () => void;
}

/** First-run setup: what SayQL is, how to use AI, pick a database, ask a first question. */
export function OnboardingDialog(p: OnboardingDialogProps) {
  const index = STEPS.indexOf(p.step);
  const title = {
    welcome: "Welcome to SayQL",
    ai: "How do you want to use AI?",
    database: "Pick a database",
    try: "Ask your first question",
  }[p.step];

  return (
    <Dialog title={title} onClose={p.onFinish} actions={null}>
      <div className={styles.progress} aria-label={`Step ${index + 1} of ${STEPS.length}`}>
        {STEPS.map((s, i) => (
          <span key={s} className={cx(styles.dot, i <= index && styles.dotDone)} />
        ))}
      </div>
      {p.step === "welcome" && <Welcome {...p} />}
      {p.step === "ai" && <ChooseAi {...p} />}
      {p.step === "database" && <ChooseDatabase {...p} />}
      {p.step === "try" && <TryIt {...p} />}
    </Dialog>
  );
}

function Footer({ children }: { children: ReactNode }) {
  return <div className={styles.footer}>{children}</div>;
}

function Welcome(p: OnboardingDialogProps) {
  const points = [
    {
      icon: MessageSquareText,
      text: "Type what you need in plain English. SayQL writes the SQL as you type.",
    },
    {
      icon: ShieldCheck,
      text: "Queries run read-only. Anything that could change data asks you first.",
    },
    {
      icon: HardDrive,
      text: "The Ask box never sends rows from your tables to the AI: only your question, table and column names, and your notes.",
    },
  ];
  return (
    <>
      <div className={styles.hero}>
        <BrandMark size={52} />
        <p className={styles.lede}>
          Ask your database in plain English. Setup takes about a minute.
        </p>
      </div>
      <ul className={styles.points}>
        {points.map(({ icon: Icon, text }) => (
          <li key={text}>
            <span className={styles.pointIcon}>
              <Icon size={15} />
            </span>
            {text}
          </li>
        ))}
      </ul>
      <Footer>
        <Button variant="ghost" onClick={p.onFinish}>
          Set up later
        </Button>
        <Button variant="primary" icon={ArrowRight} onClick={() => p.onStep("ai")}>
          Get started
        </Button>
      </Footer>
    </>
  );
}

/** The ways to use AI, each with what it's good and bad at. Providers are grouped under them. */
type Way = "key" | "plan" | "credits" | "local";
const WAYS: {
  id: Way;
  title: string;
  tag?: string;
  local?: boolean;
  /** The logos of the providers it covers. */
  logos: LogoId[];
  summary: string;
  pros: string[];
  cons: string[];
}[] = [
  {
    id: "key",
    title: "Your own API key",
    logos: ["anthropic", "openai", "googlegemini", "mistralai", "deepseek"],
    summary: "Anthropic, OpenAI, Gemini and more.",
    pros: [
      "Pay the provider directly, only for what you use",
      "Every model, and the fastest answers",
    ],
    cons: ["Needs an account and an API key with billing"],
  },
  {
    id: "plan",
    title: "Your Claude plan",
    logos: ["claude"],
    tag: "No API key",
    summary: "The Pro or Max plan you already have, through your installed Claude Code.",
    pros: ["No key and no extra bill", "Claude's models, on your own sign-in"],
    cons: [
      "Needs Claude Code installed and signed in",
      "Slower: Claude Code starts for each request",
      "Counts against your plan's limits",
    ],
  },
  {
    id: "credits",
    title: "SayQL AI credits",
    logos: ["sayql"],
    tag: "No Anthropic account",
    summary: "Prepaid Claude from SayQL.",
    pros: ["No other account to set up", "Prepaid, so no surprise bills"],
    cons: [
      "Requests go through SayQL to Anthropic (never stored)",
      "Credits are paid; ask hello@azenvoc.com",
    ],
  },
  {
    id: "local",
    title: "On this computer",
    logos: ["ollama", "lmstudio"],
    local: true,
    summary: "Ollama or LM Studio.",
    pros: ["Nothing leaves your computer", "Free"],
    cons: [
      "Needs a capable computer and a downloaded model",
      "Less accurate SQL than the large models",
    ],
  },
];

function wayOf(p: AiProvider): Way {
  if (p.api === "plan") return "plan";
  if (p.id === SAYQL_CREDITS) return "credits";
  return p.local ? "local" : "key";
}

function ChooseAi(p: OnboardingDialogProps) {
  const { settings } = p;
  const current = settings.providers.find((x) => x.id === settings.provider);
  const [way, setWay] = useState<Way | null>(settings.aiReady && current ? wayOf(current) : null);
  if (!way) return <PickWay {...p} onPick={setWay} />;
  return <SetUpWay {...p} way={way} onBack={() => setWay(null)} key={way} />;
}

function PickWay(p: OnboardingDialogProps & { onPick: (w: Way) => void }) {
  const available = WAYS.filter((w) => p.settings.providers.some((x) => wayOf(x) === w.id));
  return (
    <>
      <p className={styles.intro}>
        SayQL writes SQL with the AI you choose. You can change this any time in Settings.
      </p>
      <div className={styles.ways}>
        {available.map((w) => (
          <button key={w.id} type="button" className={styles.way} onClick={() => p.onPick(w.id)}>
            <span className={styles.wayLogos}>
              {w.logos.map((id) => (
                <Logo key={id} id={id} size={20} />
              ))}
            </span>
            <span className={styles.cardTitle}>
              {w.title}
              {w.tag && <span className={styles.tag}>{w.tag}</span>}
              {w.local && <span className={cx(styles.tag, styles.tagLocal)}>Private</span>}
            </span>
            <span className={styles.cardNote}>{w.summary}</span>
            <ul className={styles.proCon}>
              {w.pros.map((t) => (
                <li key={t} className={styles.pro}>
                  <Plus size={12} aria-label="Good:" /> {t}
                </li>
              ))}
              {w.cons.map((t) => (
                <li key={t} className={styles.con}>
                  <Minus size={12} aria-label="But:" /> {t}
                </li>
              ))}
            </ul>
          </button>
        ))}
      </div>
      <Footer>
        <Button variant="ghost" onClick={() => p.onStep("database")}>
          Skip for now
        </Button>
      </Footer>
    </>
  );
}

function SetUpWay(p: OnboardingDialogProps & { way: Way; onBack: () => void }) {
  const { settings, way } = p;
  const options = settings.providers.filter((x) => wayOf(x) === way);
  const [providerId, setProviderId] = useState(
    options.some((x) => x.id === settings.provider)
      ? settings.provider
      : (options[0]?.id ?? settings.provider),
  );
  const [apiKey, setApiKey] = useState("");
  const [baseUrl, setBaseUrl] = useState("");
  const [models, setModels] = useState<string[] | null>(null);
  const [model, setModel] = useState("");
  const [planOk, setPlanOk] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const provider = options.find((x) => x.id === providerId) ?? options[0];
  const info = WAYS.find((w) => w.id === way)!;
  const featured =
    way === "key"
      ? (FEATURED.map((id) => options.find((x) => x.id === id)).filter(Boolean) as AiProvider[])
      : options;
  const more = way === "key" ? options.filter((x) => !FEATURED.includes(x.id)) : [];
  // Claude's models are fixed (its API, credits, the plan); others pick from what the service lists.
  const fixedModels = provider.api !== "openai";

  const choose = (id: string) => {
    setProviderId(id);
    setApiKey("");
    setBaseUrl("");
    setModels(null);
    setModel("");
    setError(null);
  };

  /** Checks the key (and address), then saves; other providers first pick a model from the list. */
  const next = async () => {
    setBusy(true);
    setError(null);
    try {
      if (provider.api !== "plan" && (fixedModels || !models)) {
        const ids = await commands.listModels(provider.id, baseUrl, apiKey.trim() || null);
        if (!fixedModels) {
          // Pick (or type, when the list is empty) the model first; Continue then saves.
          setModels(ids);
          setModel(provider.typingModel || ids[0] || "");
          return;
        }
      }
      const saved = await commands.saveSettings({
        provider: provider.id,
        typingModel: fixedModels ? provider.typingModel : model.trim(),
        expertModel: fixedModels ? provider.expertModel : "",
        baseUrl,
        apiKey: apiKey.trim() || null,
        workspaceId: settings.workspaceId,
      });
      p.onSettingsSaved(saved);
      p.onStep("database");
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  };

  const needsKey = provider.key === "required" && !provider.hasKey && !apiKey.trim();
  const needsUrl = provider.api === "openai" && !provider.defaultBaseUrl && !baseUrl.trim();
  const needsModel = !!models && !model.trim();
  const blocked = needsKey || needsUrl || needsModel || (provider.api === "plan" && !planOk);
  const action =
    models || provider.api === "plan"
      ? "Continue"
      : provider.api === "anthropic"
        ? "Check key and continue"
        : "Connect";

  return (
    <>
      <p className={styles.intro}>
        <b>{info.title}.</b>{" "}
        {way === "key"
          ? "Pick the provider your key is from."
          : way === "local" && featured.length > 1
            ? "Pick the app you run models in."
            : provider.note}
      </p>
      {featured.length > 1 && (
        <div className={styles.cards} role="radiogroup" aria-label="AI provider">
          {featured.map((x) => (
            <button
              key={x.id}
              type="button"
              role="radio"
              aria-checked={x.id === provider.id}
              className={cx(styles.card, x.id === provider.id && styles.cardOn)}
              onClick={() => choose(x.id)}
            >
              <span className={styles.cardTitle}>
                <Logo id={providerLogo(x.id)} size={16} />
                {x.label}
                {x.id === "anthropic" && <span className={styles.tag}>Recommended</span>}
              </span>
              <span className={styles.cardNote}>{x.note}</span>
            </button>
          ))}
        </div>
      )}
      {more.length > 0 && (
        <Field label="More providers">
          {(id) => (
            <Select
              id={id}
              value={FEATURED.includes(provider.id) ? "" : provider.id}
              onChange={(v) => v && choose(v)}
              options={[
                {
                  value: "",
                  label: [
                    ...more
                      .filter((x) => !/^other\b/i.test(x.label))
                      .slice(0, 5)
                      .map((x) => x.label),
                    "other…",
                  ].join(", "),
                },
                ...more.map((x) => ({ value: x.id, label: x.label })),
              ]}
            />
          )}
        </Field>
      )}

      {provider.api === "plan" && <ClaudePlanCheck onChecked={setPlanOk} />}
      {provider.key !== "none" && (
        <Field
          label={provider.id === SAYQL_CREDITS ? "Credit key" : `${provider.label} API key`}
          optional={provider.key === "optional"}
          hint={
            provider.id === SAYQL_CREDITS
              ? "A prepaid credit key from SayQL: ask hello@azenvoc.com. Stored in your system keychain, never in a file."
              : "Stored in your system keychain, never in a file."
          }
        >
          {(id) => (
            <TextInput
              id={id}
              type="password"
              mono
              autoFocus
              value={apiKey}
              placeholder={
                provider.hasKey
                  ? "Saved in Keychain — paste a new key to replace it"
                  : provider.keyPlaceholder || "Paste your API key"
              }
              onChange={(e) => {
                setApiKey(e.target.value);
                setModels(null);
              }}
            />
          )}
        </Field>
      )}
      {provider.api === "openai" && (!provider.defaultBaseUrl || provider.local) && (
        <Field label="Base URL" optional={!!provider.defaultBaseUrl}>
          {(id) => (
            <TextInput
              id={id}
              mono
              value={baseUrl}
              placeholder={provider.defaultBaseUrl || "https://your-server.example.com/v1"}
              onChange={(e) => {
                setBaseUrl(e.target.value);
                setModels(null);
              }}
            />
          )}
        </Field>
      )}
      {models && (
        <Field
          label="Model"
          hint="A fast model keeps SQL appearing as you type. You can add a stronger one for fixes in Settings."
        >
          {(id) =>
            models.length ? (
              <Select
                id={id}
                value={model}
                onChange={setModel}
                options={models.map((m) => ({ value: m, label: m }))}
              />
            ) : (
              <TextInput
                id={id}
                mono
                value={model}
                placeholder="Model id"
                onChange={(e) => setModel(e.target.value)}
              />
            )
          }
        </Field>
      )}
      {error && <Alert tone="danger">{error}</Alert>}

      <Footer>
        <Button variant="ghost" icon={ArrowLeft} onClick={p.onBack}>
          Other ways
        </Button>
        <Button
          variant="primary"
          icon={models || provider.api === "plan" ? Check : Plug}
          loading={busy}
          disabled={blocked}
          onClick={next}
        >
          {action}
        </Button>
      </Footer>
    </>
  );
}

function ChooseDatabase(p: OnboardingDialogProps) {
  return (
    <>
      <p className={styles.intro}>
        Start with the built-in demo, or connect a database of your own. You can add more later.
      </p>
      <div className={styles.choices}>
        {p.demoAvailable && (
          <button type="button" className={styles.choice} onClick={p.onUseDemo}>
            <span className={styles.choiceIcon}>
              <Sparkles size={18} />
            </span>
            <span>
              <b>Try the demo database</b>
              <span>A small shop with customers, orders and products. Nothing to set up.</span>
            </span>
            <ArrowRight size={16} className={styles.choiceArrow} />
          </button>
        )}
        <button type="button" className={styles.choice} onClick={p.onConnectOwn}>
          <span className={styles.choiceIcon}>
            <Database size={18} />
          </span>
          <span>
            <b>Connect my own database</b>
            <span>PostgreSQL, MySQL, SQLite or SQL Server (including Azure SQL).</span>
          </span>
          <ArrowRight size={16} className={styles.choiceArrow} />
        </button>
      </div>
      <Footer>
        <Button variant="ghost" onClick={() => p.onStep("ai")}>
          Back
        </Button>
        <Button variant="ghost" onClick={p.onFinish}>
          Skip for now
        </Button>
      </Footer>
    </>
  );
}

/**
 * The end of the setup: AI agents (Claude Code, Cursor, …) can use SayQL too, under rules set
 * per connection. Short, and every button is optional.
 */
function AgentsPanel(p: OnboardingDialogProps) {
  return (
    <section className={styles.agents} aria-labelledby="setup-agents">
      <span className={styles.choiceIcon}>
        <Bot size={16} />
      </span>
      <div>
        <h3 id="setup-agents" className={styles.agentsTitle}>
          Let AI agents in safely
        </h3>
        <p className={styles.agentsText}>
          Connect Claude Code, Cursor or another AI app to SayQL, choose what they may see per
          connection (counts and summaries by default, not your rows), and teach SayQL your database
          with an audit of your code.
        </p>
        <div className={styles.agentsActions}>
          <Button size="sm" icon={Plug} onClick={p.onOpenAiApps}>
            Connect an AI app
          </Button>
          {p.onTeach && (
            <Button size="sm" variant="ghost" icon={Wand2} onClick={p.onTeach}>
              Teach SayQL
            </Button>
          )}
        </div>
      </div>
    </section>
  );
}

function TryIt(p: OnboardingDialogProps) {
  if (!p.settings.aiReady) {
    return (
      <>
        <Alert tone="warning">
          AI is off, so SayQL can't write SQL yet. You can still browse tables and run your own SQL.
        </Alert>
        <p className={styles.intro}>
          Add an AI provider any time in Settings to turn plain English into SQL.
        </p>
        <AgentsPanel {...p} />
        <Footer>
          <Button
            variant="ghost"
            onClick={() => {
              p.onFinish();
              p.onOpenSettings();
            }}
          >
            Open Settings
          </Button>
          <Button variant="primary" onClick={p.onFinish}>
            Start using SayQL
          </Button>
        </Footer>
      </>
    );
  }
  return (
    <>
      <p className={styles.intro}>
        Pick one to see SayQL write the SQL and run it. Or close this and type your own question in
        the Ask box.
      </p>
      <div className={styles.choices}>
        {p.examples.map((q) => (
          <button
            key={q}
            type="button"
            className={cx(styles.choice, styles.example)}
            onClick={() => p.onTry(q)}
          >
            <span className={styles.choiceIcon}>
              <MessageSquareText size={16} />
            </span>
            <span>
              <b>{q}</b>
            </span>
            <ArrowRight size={16} className={styles.choiceArrow} />
          </button>
        ))}
        {p.examples.length === 0 && (
          <p className={styles.intro}>
            Type a question in the Ask box above the editor to get started.
          </p>
        )}
      </div>
      <AgentsPanel {...p} />
      <Footer>
        <Button variant="primary" onClick={p.onFinish}>
          I'll type my own
        </Button>
      </Footer>
    </>
  );
}
