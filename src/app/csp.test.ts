import { describe, expect, it } from "vitest";
import conf from "../../src-tauri/tauri.conf.json";

/** Parses "a x y; b z" into { a: ["x", "y"], b: ["z"] }. */
function directives(csp: string): Record<string, string[]> {
  return Object.fromEntries(
    csp
      .split(";")
      .map((d) => d.trim().split(/\s+/))
      .filter(([name]) => name)
      .map(([name, ...sources]) => [name, sources]),
  );
}

describe("content security policy", () => {
  const { csp, devCsp } = conf.app.security;

  it("is set, and the release policy allows no remote or inline scripts", () => {
    expect(csp).toBeTruthy();
    const d = directives(csp);
    expect(d["default-src"]).toEqual(["'self'"]);
    expect(d["script-src"]).toEqual(["'self'"]);
    expect(d["object-src"]).toEqual(["'none'"]);
  });

  it("only talks to Tauri IPC, and loads Monaco's worker from the app", () => {
    const d = directives(csp);
    expect(d["connect-src"]).toEqual(["ipc:", "http://ipc.localhost"]);
    expect(d["worker-src"]).toContain("'self'");
  });

  it("never allows a remote host in any directive", () => {
    for (const policy of [csp, devCsp]) {
      for (const [name, sources] of Object.entries(directives(policy))) {
        for (const s of sources) {
          const remote =
            /^(https?|wss?):/.test(s) && !/^(https?|wss?):\/\/(ipc\.)?localhost(:\d+)?$/.test(s);
          expect(remote, `${name} ${s}`).toBe(false);
          expect(s, name).not.toBe("*");
        }
      }
    }
  });
});
