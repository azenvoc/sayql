import { useState } from "react";
import { commands, type AgentSettings, type Settings } from "../../api";
import { Alert, Button, Checkbox, Field, FieldRow, Select, TextInput } from "../../components/ui";
import { errorText } from "../../lib/errors";
import styles from "./AgentAccessDialog.module.css";
import { AGENT_DEFAULT, AGENT_LEVEL, LEVELS } from "./resultLevels";

const LIMITS: { key: Exclude<keyof AgentSettings, "skipPlan">; label: string; hint: string }[] = [
  {
    key: "maxSteps",
    label: "Steps per run",
    hint: "Model turns; at the limit the AI must answer with what it has.",
  },
  { key: "maxQueries", label: "Queries per run", hint: "Further queries are refused." },
  {
    key: "querySeconds",
    label: "Seconds per query",
    hint: "Then the query is stopped on the server.",
  },
  { key: "runSeconds", label: "Seconds per run", hint: "Waiting for you doesn't count." },
  {
    key: "maxTokens",
    label: "Tokens per run",
    hint: "Prompt tokens at the full rate plus output tokens.",
  },
];

/** Settings → Agent mode: the model that runs it and the limits per run. */
export function AgentSettingsSection({
  settings,
  onSaved,
}: {
  settings: Settings;
  onSaved: (s: Settings) => void;
}) {
  const [model, setModel] = useState(settings.agentModel);
  const [limits, setLimits] = useState<AgentSettings>(settings.agent);
  const [error, setError] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);
  const claude = settings.provider === "anthropic";

  const save = () => {
    setError(null);
    commands.saveAgentSettings(model, limits).then(
      (s) => {
        setSaved(true);
        onSaved(s);
      },
      (e) => setError(errorText(e)),
    );
  };

  return (
    <div className={styles.body}>
      <p className={styles.lead}>
        Investigate answers a question in several read-only steps and ends with a cited answer. Turn
        it on per connection: switch the Ask box to <strong>Investigate</strong> (<kbd>⌘⇧A</kbd>{" "}
        there) or type <code>/investigate</code>, or right-click a connection and choose Investigate
        settings. It never runs writes. What it may see of query results is set per connection, and
        starts at <strong>{LEVELS[AGENT_LEVEL[AGENT_DEFAULT]].label}</strong>.
      </p>
      <Field
        label="Model"
        hint="Accuracy matters more than speed here. Fast models aren't recommended for investigating."
      >
        {(id) =>
          claude ? (
            <Select
              id={id}
              value={model}
              onChange={setModel}
              options={[
                { value: "", label: "Same as the expert model" },
                ...settings.models.map((m) => ({ value: m.id, label: m.label })),
              ]}
            />
          ) : (
            <TextInput
              id={id}
              value={model}
              placeholder="Same as the expert model"
              onChange={(e) => setModel(e.target.value)}
            />
          )
        }
      </Field>
      <FieldRow>
        {LIMITS.slice(0, 3).map((l) => (
          <Field key={l.key} label={l.label} hint={l.hint} grow>
            {(id) => (
              <TextInput
                id={id}
                type="number"
                value={limits[l.key]}
                onChange={(e) => setLimits({ ...limits, [l.key]: Number(e.target.value) })}
              />
            )}
          </Field>
        ))}
      </FieldRow>
      <FieldRow>
        {LIMITS.slice(3).map((l) => (
          <Field key={l.key} label={l.label} hint={l.hint} grow>
            {(id) => (
              <TextInput
                id={id}
                type="number"
                value={limits[l.key]}
                onChange={(e) => setLimits({ ...limits, [l.key]: Number(e.target.value) })}
              />
            )}
          </Field>
        ))}
      </FieldRow>
      <Checkbox
        label="Skip the plan: start right away instead of showing the plan first"
        checked={limits.skipPlan}
        onChange={(skipPlan) => setLimits({ ...limits, skipPlan })}
      />
      {error && <Alert tone="danger">{error}</Alert>}
      <div>
        <Button variant="primary" onClick={save}>
          Save
        </Button>
        {saved && <span className={styles.detail}> Saved</span>}
      </div>
    </div>
  );
}
