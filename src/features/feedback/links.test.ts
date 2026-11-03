import { describe, expect, it } from "vitest";
import { githubIssueUrl, osLabel, privateFormUrl } from "./links";

const mac = { version: "0.1.1", os: "macos", arch: "aarch64" };

describe("feedback links", () => {
  it("names the OS the way the issue templates' dropdown does", () => {
    expect(osLabel(mac)).toBe("macOS (Apple silicon)");
    expect(osLabel({ ...mac, arch: "x86_64" })).toBe("macOS (Intel)");
    expect(osLabel({ ...mac, os: "windows" })).toBe("Windows");
    expect(osLabel({ ...mac, os: "linux" })).toBe("Linux");
  });

  it("pre-fills the feedback template with the text, version and OS only", () => {
    const url = new URL(githubIssueUrl("feedback", "  Loved the follow-ups  ", mac));
    expect(url.origin + url.pathname).toBe("https://github.com/azenvoc/sayql/issues/new");
    expect(Object.fromEntries(url.searchParams)).toEqual({
      template: "feedback.yml",
      feedback: "Loved the follow-ups",
      version: "0.1.1",
      os: "macOS (Apple silicon)",
    });
  });

  it("uses the bug and feature templates' own fields", () => {
    const bug = new URL(githubIssueUrl("bug", "Crash on connect", mac)).searchParams;
    expect([bug.get("template"), bug.get("what")]).toEqual(["bug_report.yml", "Crash on connect"]);
    const idea = new URL(githubIssueUrl("idea", "", mac)).searchParams;
    expect([idea.get("template"), idea.has("problem")]).toEqual(["feature_request.yml", false]);
  });

  it("keeps long text short enough for a URL", () => {
    const url = new URL(githubIssueUrl("feedback", "x".repeat(5000), mac));
    expect(url.searchParams.get("feedback")).toHaveLength(1800);
  });

  it("adds the version and OS to the private form", () => {
    const url = new URL(privateFormUrl("https://tally.so/r/abc?ref=app", "bug", mac));
    expect(Object.fromEntries(url.searchParams)).toEqual({
      ref: "app",
      kind: "bug",
      version: "0.1.1",
      os: "macOS (Apple silicon)",
    });
  });
});
