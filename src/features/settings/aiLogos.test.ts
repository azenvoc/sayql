import { describe, expect, it } from "vitest";
import { modelLogo, providerLogo } from "./aiLogos";

describe("AI logos", () => {
  it("gives every provider SayQL offers a logo, and unknown ones the OpenAI-compatible icon", () => {
    expect(providerLogo("anthropic")).toBe("anthropic");
    expect(providerLogo("claude-plan")).toBe("claude");
    expect(providerLogo("sayql")).toBe("sayql");
    expect(providerLogo("gemini")).toBe("googlegemini");
    expect(providerLogo("custom")).toBe("compatible");
    expect(providerLogo("someday")).toBe("compatible");
  });

  it("shows a model's maker when its id names one, otherwise the provider", () => {
    expect(modelLogo("anthropic/claude-sonnet-5-5", "openrouter")).toBe("claude");
    expect(modelLogo("openai/gpt-5-mini", "openrouter")).toBe("openai");
    expect(modelLogo("o4-mini", "openai")).toBe("openai");
    expect(modelLogo("models/gemini-3-flash", "gemini")).toBe("googlegemini");
    expect(modelLogo("qwen2.5-coder:14b", "ollama")).toBe("qwen");
    expect(modelLogo("llama-4-scout", "groq")).toBe("meta");
    expect(modelLogo("my-finetune", "lmstudio")).toBe("lmstudio");
  });
});
