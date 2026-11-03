import type { LogoId } from "../../components/ui";

/** Each AI provider's logo, by the provider id in src-tauri/src/ai/providers.rs. */
const PROVIDER_LOGOS: Record<string, LogoId> = {
  anthropic: "anthropic",
  "claude-plan": "claude",
  sayql: "sayql",
  openai: "openai",
  gemini: "googlegemini",
  openrouter: "openrouter",
  mistral: "mistralai",
  groq: "groq",
  deepseek: "deepseek",
  ollama: "ollama",
  lmstudio: "lmstudio",
  custom: "compatible",
};

/** A provider's logo; a provider SayQL doesn't know is shown as OpenAI-compatible. */
export function providerLogo(provider: string): LogoId {
  return PROVIDER_LOGOS[provider] ?? "compatible";
}

/** Model families, by words in the model's id, for the logo of the company that made it. */
const MODEL_LOGOS: [RegExp, LogoId][] = [
  [/claude|haiku|sonnet|opus|fable/, "claude"],
  [/gpt|chatgpt|(^|\/)o\d|openai/, "openai"],
  [/gemini|gemma/, "googlegemini"],
  [/mistral|mixtral|codestral|ministral|magistral|devstral|pixtral/, "mistralai"],
  [/deepseek/, "deepseek"],
  [/llama/, "meta"],
  [/qwen|qwq/, "qwen"],
];

/**
 * The logo for a model: its maker's when the id names one (`anthropic/claude-…` through
 * OpenRouter, `qwen2.5-coder` in Ollama), otherwise the provider's.
 */
export function modelLogo(model: string, provider: string): LogoId {
  const id = model.toLowerCase();
  return MODEL_LOGOS.find(([re]) => re.test(id))?.[1] ?? providerLogo(provider);
}
