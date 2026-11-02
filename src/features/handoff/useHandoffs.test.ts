// @vitest-environment jsdom
import { renderHook, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { Handoff } from "../../api";
import { useHandoffs } from "./useHandoffs";

const takeHandoffs = vi.fn<() => Promise<Handoff[]>>();
vi.mock("../../api", () => ({
  commands: { takeHandoffs: () => takeHandoffs() },
  subscribeToHandoffs: () => Promise.resolve(() => {}),
}));

describe("useHandoffs", () => {
  it("waits until the window is ready before taking handoffs, so a script opened at launch is found", async () => {
    takeHandoffs.mockResolvedValue([
      { type: "openScript", scriptId: "s1", source: "Claude Code" } as Handoff,
    ]);
    const openScript = vi.fn(() => true);
    const notify = vi.fn();
    const actions = {
      connection: () => null,
      openConnectionDialog: vi.fn(),
      openTab: vi.fn(),
      openScript,
      openTeamSignIn: vi.fn(),
      finishTeamSso: vi.fn(),
      notify,
    };
    const { rerender } = renderHook(({ ready }) => useHandoffs(actions, ready), {
      initialProps: { ready: false },
    });
    await new Promise((r) => setTimeout(r, 10));
    expect(takeHandoffs).not.toHaveBeenCalled();

    rerender({ ready: true });
    await waitFor(() => expect(openScript).toHaveBeenCalledWith("s1"));
    expect(notify).not.toHaveBeenCalled();
  });
});
