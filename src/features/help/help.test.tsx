// @vitest-environment jsdom
import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { version } from "../../../package.json";
import { ShortcutSheet, WhatsNewDialog } from "./HelpDialogs";
import { SECTION_DOCS } from "./docs";
import { isTypingTarget, keyLabel } from "./shortcuts";
import { WHATS_NEW, WHATS_NEW_SEEN_KEY, shouldShowWhatsNew, whatsNewOnStart } from "./whatsNew";

afterEach(cleanup);

describe("shortcut sheet", () => {
  it("lists the app's shortcuts, ⌘W and ⌘, included", () => {
    render(<ShortcutSheet onClose={() => {}} />);
    for (const group of ["Anywhere", "Ask box", "SQL editor", "Navigator"])
      expect(screen.getByRole("region", { name: group })).toBeTruthy();
    expect(screen.getByText("Close the tab")).toBeTruthy();
    expect(screen.getByText("Open Settings")).toBeTruthy();
    expect(screen.getByText(/Show these shortcuts/)).toBeTruthy();
  });

  it("shows ⌘ on macOS and Ctrl elsewhere", () => {
    expect(keyLabel("Mod", true)).toBe("⌘");
    expect(keyLabel("Mod", false)).toBe("Ctrl");
    expect(keyLabel("?", true)).toBe("?");
  });

  it("leaves ? to text fields and the SQL editor", () => {
    const input = document.createElement("input");
    const area = document.createElement("textarea");
    const editor = document.createElement("div");
    editor.className = "monaco-editor";
    const inner = document.createElement("span");
    editor.appendChild(inner);
    const plain = document.createElement("div");
    document.body.append(input, area, editor, plain);
    expect(isTypingTarget(input)).toBe(true);
    expect(isTypingTarget(area)).toBe(true);
    expect(isTypingTarget(inner)).toBe(true);
    expect(isTypingTarget(plain)).toBe(false);
    expect(isTypingTarget(document.body)).toBe(false);
  });
});

describe("What's new", () => {
  beforeEach(() => localStorage.clear());

  it("shows once per version, after an update", () => {
    const v = WHATS_NEW[0].version;
    expect(shouldShowWhatsNew(v, "0.4.7", true)).toBe(true);
    expect(shouldShowWhatsNew(v, v, true)).toBe(false);
    // Updated from a version that didn't remember anything yet.
    expect(shouldShowWhatsNew(v, null, true)).toBe(true);
    // Someone new: the setup, not What's new.
    expect(shouldShowWhatsNew(v, null, false)).toBe(false);
    // A version with nothing written for it shows nothing.
    expect(shouldShowWhatsNew("9.9.9", "0.4.7", true)).toBe(false);
  });

  it("remembers the version once seen, and starts someone new at their version", () => {
    const v = WHATS_NEW[0].version;
    localStorage.setItem(WHATS_NEW_SEEN_KEY, JSON.stringify("0.4.7"));
    const first = whatsNewOnStart(v, true);
    expect(first.show).toBe(true);
    first.markSeen();
    expect(whatsNewOnStart(v, true).show).toBe(false);

    localStorage.clear();
    expect(whatsNewOnStart(v, false).show).toBe(false);
    expect(JSON.parse(localStorage.getItem(WHATS_NEW_SEEN_KEY)!)).toBe(v);
  });

  it("shows the entry for the version, or the latest one", () => {
    render(<WhatsNewDialog version={version} onClose={() => {}} />);
    expect(screen.getByText(/What's new in SayQL \d+\.\d+\.\d+/)).toBeTruthy();
    expect(screen.getByText(WHATS_NEW[0].items[0].title + ".")).toBeTruthy();
  });
});

describe("docs links", () => {
  it("point at pages and headings the docs site has", () => {
    const pages = import.meta.glob("../../../docs-site/pages/*.html", {
      query: "?raw",
      import: "default",
      eager: true,
    }) as Record<string, string>;
    const slugs = new Map<string, string>();
    expect(Object.keys(pages).length).toBeGreaterThan(10);
    for (const raw of Object.values(pages)) {
      const slug = /slug:\s*([^|]*?)\s*\|/.exec(raw)?.[1] ?? "";
      slugs.set(slug, raw);
    }
    for (const target of Object.values(SECTION_DOCS)) {
      const [path, anchor] = target.split("#");
      const slug = path.replace(/\/$/, "");
      expect(slugs.has(slug), `docs page ${slug}`).toBe(true);
      if (anchor) {
        const headings = [...slugs.get(slug)!.matchAll(/<h[23]>(.*?)<\/h[23]>/g)].map((m) =>
          (new DOMParser().parseFromString(m[1], "text/html").body.textContent ?? "")
            .toLowerCase()
            .replace(/[^a-z0-9]+/g, "-")
            .replace(/^-|-$/g, ""),
        );
        expect(headings, `#${anchor} on ${slug}`).toContain(anchor);
      }
    }
  });
});
