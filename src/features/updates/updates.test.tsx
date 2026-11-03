// @vitest-environment jsdom
import {
  act,
  cleanup,
  fireEvent,
  render,
  renderHook,
  screen,
  waitFor,
} from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { UpdateInfo } from "../../api";
import { UpdateDialog, releaseUrl } from "./UpdateDialog";
import { useUpdate } from "./useUpdate";

const checkUpdate = vi.fn<() => Promise<UpdateInfo | null>>();
vi.mock("../../api", async (orig) => ({
  ...(await orig<typeof import("../../api")>()),
  commands: { checkUpdate: () => checkUpdate(), installUpdate: () => Promise.resolve(null) },
}));

const UPDATE: UpdateInfo = {
  version: "0.6.0",
  currentVersion: "0.5.5",
  date: "2026-11-10",
  notes: "### New\n\n- **Logos.** Every database has its logo.",
};

afterEach(() => {
  cleanup();
  checkUpdate.mockReset();
  localStorage.clear();
});

describe("update dialog", () => {
  it("opens on its own when the startup check finds a newer version", async () => {
    checkUpdate.mockResolvedValue(UPDATE);
    const { result } = renderHook(() => useUpdate());
    await waitFor(() => expect(result.current.open).toBe(true));
    expect(result.current.skipped).toBe(false);
  });

  it("stays closed for a version the person chose not to see, and opens for the next one", async () => {
    checkUpdate.mockResolvedValue(UPDATE);
    const first = renderHook(() => useUpdate());
    await waitFor(() => expect(first.result.current.update).not.toBeNull());
    act(() => first.result.current.setSkipped(true));
    expect(first.result.current.skipped).toBe(true);
    first.unmount();

    const again = renderHook(() => useUpdate());
    await waitFor(() => expect(again.result.current.update).not.toBeNull());
    expect(again.result.current.open).toBe(false);
    again.unmount();

    checkUpdate.mockResolvedValue({ ...UPDATE, version: "0.6.1" });
    const newer = renderHook(() => useUpdate());
    await waitFor(() => expect(newer.result.current.open).toBe(true));
  });

  it("shows both versions, the release notes and the skip choice", async () => {
    checkUpdate.mockResolvedValue(UPDATE);
    const { result } = renderHook(() => useUpdate());
    await waitFor(() => expect(result.current.update).not.toBeNull());
    render(<UpdateDialog updates={result.current} />);
    expect(screen.getByText("0.5.5")).toBeTruthy();
    expect(screen.getByText("(2026-11-10)")).toBeTruthy();
    expect(screen.getByRole("region", { name: "Release notes" }).textContent).toContain(
      "Every database has its logo.",
    );
    fireEvent.click(screen.getByLabelText("Don't show for version 0.6.0"));
    await waitFor(() => expect(result.current.skipped).toBe(true));
  });

  it("links Details… to the version's release page", () => {
    expect(releaseUrl("0.6.0")).toBe(
      "https://github.com/azenvoc/sayql-releases/releases/tag/v0.6.0",
    );
  });
});
