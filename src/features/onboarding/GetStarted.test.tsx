// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { GetStarted, type ChecklistItem } from "./GetStarted";

const item = (id: string, done: boolean): ChecklistItem => ({
  id,
  title: `Step ${id}`,
  done,
  action: `Do ${id}`,
  onAction: vi.fn(),
});

describe("Get started", () => {
  afterEach(cleanup);

  it("counts what's done and offers a button for each step left", () => {
    const items = [item("a", true), item("b", false), item("c", false)];
    render(<GetStarted items={items} onHide={vi.fn()} />);
    expect(screen.getByText("1 of 3")).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Do a" })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Do b" }));
    expect(items[1].onAction).toHaveBeenCalled();
  });

  it("can be hidden, and goes away once everything is done", () => {
    const onHide = vi.fn();
    const { rerender } = render(<GetStarted items={[item("a", false)]} onHide={onHide} />);
    fireEvent.click(screen.getByRole("button", { name: "Hide Get started" }));
    expect(onHide).toHaveBeenCalled();
    rerender(<GetStarted items={[item("a", true)]} onHide={onHide} />);
    expect(screen.queryByLabelText("Get started")).toBeNull();
  });
});
