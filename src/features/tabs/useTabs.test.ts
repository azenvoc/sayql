// @vitest-environment jsdom
import { act, renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";
import { hasUnsavedChanges, isPristine, useTabs } from "./useTabs";

const target = (connectionId: string) => ({ connectionId, database: null });

describe("useTabs", () => {
  beforeEach(() => localStorage.clear());

  it("opens tabs and activates the newest", () => {
    const { result } = renderHook(() => useTabs());
    expect(result.current.active).toBeNull();
    let second = "";
    act(() => {
      result.current.open(target("a"), { sql: "SELECT 1;" });
      second = result.current.open(target("b"));
    });
    expect(result.current.tabs.map((t) => t.target.connectionId)).toEqual(["a", "b"]);
    expect(result.current.active?.id).toBe(second);
    expect(result.current.tabs[0].sql).toBe("SELECT 1;");
  });

  it("closing the active tab activates its left neighbour", () => {
    const { result } = renderHook(() => useTabs());
    const ids: string[] = [];
    act(() => {
      ids.push(
        result.current.open(target("a")),
        result.current.open(target("b")),
        result.current.open(target("c")),
      );
    });
    act(() => result.current.activate(ids[1]));
    act(() => result.current.close(ids[1]));
    expect(result.current.active?.id).toBe(ids[0]);
    act(() => result.current.close(ids[0]));
    expect(result.current.active?.id).toBe(ids[2]);
  });

  it("closeWhere closes every match and keeps an unaffected active tab", () => {
    const { result } = renderHook(() => useTabs());
    let keep = "";
    act(() => {
      result.current.open(target("a"));
      keep = result.current.open(target("b"));
      result.current.open(target("a"));
    });
    act(() => result.current.activate(keep));
    act(() => result.current.closeWhere((t) => t.target.connectionId === "a"));
    expect(result.current.tabs.map((t) => t.id)).toEqual([keep]);
    expect(result.current.active?.id).toBe(keep);
  });

  it("updates a tab and remembers tabs across mounts", () => {
    const first = renderHook(() => useTabs());
    let id = "";
    act(() => {
      id = first.result.current.open(target("a"));
    });
    act(() => first.result.current.update(id, { sql: "SELECT 2;" }));
    first.unmount();
    const { result } = renderHook(() => useTabs());
    expect(result.current.tabs).toHaveLength(1);
    expect(result.current.tabs[0]).toMatchObject({ id, sql: "SELECT 2;" });
  });

  it("hasUnsavedChanges ignores empty and saved SQL", () => {
    const tab = { id: "x", target: target("a"), title: "", question: "" };
    expect(hasUnsavedChanges({ ...tab, sql: "  " })).toBe(false);
    expect(hasUnsavedChanges({ ...tab, sql: "SELECT 1;", savedSql: "SELECT 1;" })).toBe(false);
    expect(hasUnsavedChanges({ ...tab, sql: "SELECT 1;" })).toBe(true);
  });
});

describe("isPristine", () => {
  const tab = { id: "t", target: target("demo"), title: "", question: "", sql: "" };
  it("is true only for a tab nothing has happened in, which the setup's first question reuses", () => {
    expect(isPristine(tab)).toBe(true);
    expect(isPristine({ ...tab, sql: "SELECT 1" })).toBe(false);
    expect(isPristine({ ...tab, question: "top customers" })).toBe(false);
    expect(isPristine({ ...tab, title: "orders" })).toBe(false);
    expect(isPristine({ ...tab, lastRun: { question: "", sql: "SELECT 1" } })).toBe(false);
    expect(isPristine({ ...tab, scriptId: "s1" })).toBe(false);
  });
});
