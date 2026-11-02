// @vitest-environment jsdom
import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, expect, it } from "vitest";
import { Markdown, parseBlocks } from "./Markdown";

afterEach(cleanup);

it("splits an AI summary into blocks", () => {
  const text =
    "## Title\n\nOne line\ncontinues.\n\n- a\n- b\n  more b\n\n1. first\n2. second\n\n> quoted\n\n```\nSELECT 1;\n```\n\n---\nEnd";
  expect(parseBlocks(text)).toEqual([
    { kind: "h", level: 2, text: "Title" },
    { kind: "p", text: "One line continues." },
    { kind: "ul", items: ["a", "b more b"] },
    { kind: "ol", items: ["first", "second"] },
    { kind: "quote", text: "quoted" },
    { kind: "code", text: "SELECT 1;" },
    { kind: "hr" },
    { kind: "p", text: "End" },
  ]);
});

it("styles inline Markdown without running HTML or unsafe links", () => {
  render(
    <Markdown
      text={"**bold** and *em* and `a*b*c` [x](javascript:alert(1)) <b>raw</b>"}
      onLink={() => {}}
    />,
  );
  expect(screen.getByText("bold").tagName).toBe("STRONG");
  expect(screen.getByText("em").tagName).toBe("EM");
  expect(screen.getByText("a*b*c").tagName).toBe("CODE");
  expect(screen.queryByRole("link")).toBeNull();
  expect(document.querySelector("b")).toBeNull();
});
