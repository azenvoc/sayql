// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { AiProvider, Settings } from "../../api";
import { commands } from "../../api/commands";
import { OnboardingDialog, type OnboardingDialogProps } from "./OnboardingDialog";

vi.mock("../../api/commands", () => ({
  commands: { listModels: vi.fn(), saveSettings: vi.fn(), checkClaudePlan: vi.fn() },
}));

const provider = (id: string, label: string, extra: Partial<AiProvider> = {}): AiProvider =>
  ({
    id,
    label,
    note: `${label} note`,
    api: "openai",
    key: "required",
    keyPlaceholder: "",
    local: false,
    defaultBaseUrl: "x",
    baseUrl: "",
    typingModel: "",
    expertModel: "",
    agentModel: "",
    hasKey: false,
    ready: false,
    ...extra,
  }) as AiProvider;

const settings = (aiReady: boolean) =>
  ({
    provider: "anthropic",
    aiReady,
    workspaceId: "",
    providers: [
      provider("anthropic", "Anthropic Claude", { api: "anthropic" }),
      provider("sayql", "SayQL AI credits", {
        api: "anthropic",
        note: "Claude without an Anthropic account.",
      }),
      provider("openai", "OpenAI"),
      provider("gemini", "Google Gemini"),
      provider("ollama", "Ollama", { key: "none", local: true }),
      provider("claude-plan", "Your Claude plan", {
        api: "plan",
        key: "none",
        note: "Uses your installed Claude Code.",
        typingModel: "claude-haiku-5-5",
      }),
      provider("openrouter", "OpenRouter"),
    ],
  }) as unknown as Settings;

function renderStep(
  step: OnboardingDialogProps["step"],
  over: Partial<OnboardingDialogProps> = {},
) {
  const p: OnboardingDialogProps = {
    settings: settings(true),
    step,
    onStep: vi.fn(),
    onSettingsSaved: vi.fn(),
    demoAvailable: true,
    onUseDemo: vi.fn(),
    onConnectOwn: vi.fn(),
    examples: ["Top 5 customers"],
    onTry: vi.fn(),
    onOpenSettings: vi.fn(),
    onOpenAiApps: vi.fn(),
    onTeach: vi.fn(),
    onFinish: vi.fn(),
    ...over,
  };
  render(<OnboardingDialog {...p} />);
  return p;
}

describe("first-run setup", () => {
  afterEach(cleanup);

  it("offers four ways to use AI, each with pros and cons", () => {
    renderStep("ai", { settings: settings(false) });
    for (const way of [
      "Your own API key",
      "Your Claude plan",
      "SayQL AI credits",
      "On this computer",
    ]) {
      const card = screen.getByRole("button", { name: new RegExp(way) });
      expect(card.querySelectorAll("li").length).toBeGreaterThanOrEqual(2);
    }
    expect(screen.getByRole("button", { name: /Your Claude plan/ }).textContent).toContain(
      "Counts against your plan's limits",
    );
  });

  it("offers SayQL AI credits, with where to get a key", () => {
    renderStep("ai", { settings: settings(false) });
    fireEvent.click(screen.getByRole("button", { name: /SayQL AI credits/ }));
    expect(screen.getByText("Credit key")).toBeTruthy();
    expect(screen.getAllByText(/hello@azenvoc\.com/).length).toBeGreaterThan(0);
  });

  it("checks Claude Code before using the Claude plan", async () => {
    vi.mocked(commands.checkClaudePlan).mockRejectedValueOnce("Claude Code isn't signed in.");
    renderStep("ai", { settings: settings(false) });
    fireEvent.click(screen.getByRole("button", { name: /Your Claude plan/ }));
    expect(await screen.findByText(/isn't signed in/)).toBeTruthy();
    expect((screen.getByRole("button", { name: "Continue" }) as HTMLButtonElement).disabled).toBe(
      true,
    );

    vi.mocked(commands.checkClaudePlan).mockResolvedValueOnce({
      version: "2.1.308",
      authMethod: "claude.ai",
      plan: "max",
    });
    fireEvent.click(screen.getByRole("button", { name: "Check again" }));
    expect(await screen.findByText(/signed in with your Claude Max plan/)).toBeTruthy();
    expect((screen.getByRole("button", { name: "Continue" }) as HTMLButtonElement).disabled).toBe(
      false,
    );
  });

  it("ends with a short, optional way into AI agents and governance", () => {
    const p = renderStep("try");
    expect(screen.getByText("Let AI agents in safely")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Connect an AI app" }));
    expect(p.onOpenAiApps).toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "Teach SayQL" }));
    expect(p.onTeach).toHaveBeenCalled();
    // Skippable: the usual way out is still there.
    expect(screen.getByRole("button", { name: "I'll type my own" })).toBeTruthy();
  });

  it("shows the panel with AI off too, and no Teach button without a database", () => {
    renderStep("try", { settings: settings(false), onTeach: undefined });
    expect(screen.getByRole("button", { name: "Connect an AI app" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Teach SayQL" })).toBeNull();
  });
});
