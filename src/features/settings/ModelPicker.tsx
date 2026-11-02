import { Check, ChevronDown, Settings as SettingsIcon } from "lucide-react";
import { commands, type AiProvider, type Settings, type TeamAiStatus } from "../../api";
import { ContextMenu, useContextMenu, type MenuEntry } from "../../components/ui";
import { errorText } from "../../lib/errors";
import { modelLabel } from "./useSettings";
import styles from "./ModelPicker.module.css";

export interface ModelPickerProps {
  settings: Settings;
  /** Quick mode sets the model that writes SQL as you type; agent mode the one that runs the agent. */
  purpose: "typing" | "agent";
  onChange: (s: Settings) => void;
  onError: (message: string) => void;
  onOpenSettings: () => void;
  /** On a team connection with team AI: the team's models, set by the admin (no switching here). */
  team?: TeamAiStatus | null;
}

/** The models a provider offers here: Claude's list, or the ones chosen for it in Settings. */
function modelsOf(settings: Settings, p: AiProvider): string[] {
  if (p.api !== "openai") return settings.models.map((m) => m.id);
  return [...new Set([p.typingModel, p.expertModel, p.agentModel].filter(Boolean))];
}

/** The model in use for `purpose`, as an id. */
export function currentModel(settings: Settings, purpose: "typing" | "agent"): string {
  if (purpose === "typing") return settings.typingModel;
  return settings.agentModel || settings.expertModel;
}

/** A small button in the Ask box that switches the model, and the provider, without opening Settings. */
export function ModelPicker({
  settings,
  purpose,
  onChange,
  onError,
  onOpenSettings,
  team,
}: ModelPickerProps) {
  const menu = useContextMenu();
  if (team) return <TeamModels team={team} purpose={purpose} />;
  return (
    <OwnModels
      settings={settings}
      purpose={purpose}
      onChange={onChange}
      onError={onError}
      onOpenSettings={onOpenSettings}
      menu={menu}
    />
  );
}

/** The team's models: the one in use, and the admin's list, which can't be changed here. */
function TeamModels({ team, purpose }: { team: TeamAiStatus; purpose: "typing" | "agent" }) {
  const menu = useContextMenu();
  const current =
    (purpose === "typing" ? team.fastModel || team.defaultModel : team.defaultModel) ?? "";
  const items: MenuEntry[] = [
    { label: "Set by your team admin", disabled: true, onSelect: () => {} },
    "separator",
    ...(team.models.length ? team.models : [current]).map((id): MenuEntry => ({
      label: id,
      icon: id === current ? Check : undefined,
      disabled: true,
      onSelect: () => {},
    })),
  ];
  return (
    <>
      <button
        type="button"
        className={styles.picker}
        title="Your team's AI, set by your team admin. AI requests on this connection go through your team's SayQL server."
        onClick={(e) => menu.open(e, items)}
      >
        {current} · Team
        <ChevronDown size={12} />
      </button>
      {menu.menu && <ContextMenu at={menu.menu.at} items={menu.menu.items} onClose={menu.close} />}
    </>
  );
}

function OwnModels({
  settings,
  purpose,
  onChange,
  onError,
  onOpenSettings,
  menu,
}: Omit<ModelPickerProps, "team"> & { menu: ReturnType<typeof useContextMenu> }) {
  const current = currentModel(settings, purpose);
  const ready = settings.providers.filter((p) => p.ready);

  const choose = (provider: string, model: string) =>
    commands.chooseModel(provider, model, purpose).then(onChange, (e) => onError(errorText(e)));

  const entries = (p: AiProvider): MenuEntry[] =>
    modelsOf(settings, p).map((id) => ({
      label: modelLabel(settings, id, p.id),
      icon: p.id === settings.provider && id === current ? Check : undefined,
      onSelect: () => void choose(p.id, id),
    }));

  const items: MenuEntry[] = [
    ...(ready.length === 1
      ? entries(ready[0])
      : ready.map((p): MenuEntry => ({
          label: p.id === settings.provider ? `${p.label} (in use)` : p.label,
          submenu: entries(p),
        }))),
    "separator",
    { label: "More models and providers…", icon: SettingsIcon, onSelect: onOpenSettings },
  ];

  if (!settings.aiReady) return null;
  return (
    <>
      <button
        type="button"
        className={styles.picker}
        title={
          purpose === "agent"
            ? "The model that investigates. Click to switch."
            : "The model that writes SQL as you type. Click to switch."
        }
        onClick={(e) => menu.open(e, items)}
      >
        {modelLabel(settings, current)}
        <ChevronDown size={12} />
      </button>
      {menu.menu && <ContextMenu at={menu.menu.at} items={menu.menu.items} onClose={menu.close} />}
    </>
  );
}
