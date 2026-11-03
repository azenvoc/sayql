import { useState } from "react";
import type { Settings } from "../../api";
import type { LogoId } from "../../components/ui";
import { modelLogo } from "./aiLogos";

/** Short display names of the two models, e.g. "Haiku 5.5". */
export interface ModelNames {
  typing: string;
  expert: string;
  /** The logo of the model that writes SQL as you type. */
  logo: LogoId;
}

/** A model's short display name, e.g. "Haiku 5.5" or "gpt-5-mini", for `provider` (default: the current one). */
export function modelLabel(settings: Settings, id: string, provider = settings.provider): string {
  return shortName({ ...settings, provider }, id);
}

function shortName(settings: Settings, id: string): string {
  if (settings.provider === "anthropic") {
    const label = settings.models.find((m) => m.id === id)?.label ?? id;
    return label.split(" — ")[0].replace(/^Claude /, "");
  }
  // "openai/gpt-x" (OpenRouter) and "models/gemini-x" (Gemini) read better without the prefix.
  return id.slice(id.lastIndexOf("/") + 1);
}

/** App settings, initialised from the bootstrap payload and replaced when the dialog saves. */
export function useSettings(initial: Settings) {
  const [settings, setSettings] = useState(initial);
  const modelNames: ModelNames = {
    typing: shortName(settings, settings.typingModel),
    expert: shortName(settings, settings.expertModel),
    logo: modelLogo(settings.typingModel, settings.provider),
  };
  return { settings, setSettings, modelNames };
}
