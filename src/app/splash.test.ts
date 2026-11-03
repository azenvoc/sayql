import { describe, expect, it } from "vitest";
import conf from "../../src-tauri/tauri.conf.json";
import splash from "../../splash.html?raw";

describe("splash screen", () => {
  const windows = conf.app.windows;
  const main = windows.find((w) => w.label === "main");
  const card = windows.find((w) => w.label === "splash");

  it("covers the main window, which starts hidden and is shown when its first screen is ready", () => {
    expect(main?.visible).toBe(false);
    expect(card?.url).toBe("splash.html");
    expect(card?.decorations).toBe(false);
    expect(card?.skipTaskbar).toBe(true);
  });

  it("gets no permissions: only the main window has a capability", async () => {
    const capability = await import("../../src-tauri/capabilities/default.json");
    expect(capability.windows).toEqual(["main"]);
  });

  it("runs no scripts and loads nothing, so it draws at once under the release CSP", () => {
    expect(splash).not.toMatch(/<script|<link|<img|<iframe|\son[a-z]+=/i);
    expect(splash).not.toMatch(/url\(|@import|https?:\/\//i);
  });

  it("shows the app's version, filled in at build time", () => {
    expect(splash).toContain("Community Edition");
    expect(splash).toContain("Version %SAYQL_VERSION%");
  });
});
