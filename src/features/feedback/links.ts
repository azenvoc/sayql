import type { AppInfo } from "../../api";

const REPO = "https://github.com/dithzz/sayql";

/**
 * The private feedback form (Tally, Google Forms, …). Empty hides "Send privately".
 * The form should accept `version` and `os` as URL parameters (hidden fields in Tally).
 */
export const FEEDBACK_FORM_URL = "";

export type FeedbackKind = "feedback" | "bug" | "idea";

/** Long text makes URLs some browsers refuse; the person can add more on the page. */
const MAX_TEXT = 1800;

/** The label used by the issue templates' operating-system dropdown. */
export function osLabel(info: AppInfo): string {
  if (info.os === "macos")
    return info.arch === "aarch64" ? "macOS (Apple silicon)" : "macOS (Intel)";
  if (info.os === "windows") return "Windows";
  if (info.os === "linux") return "Linux";
  return info.os;
}

/** A new GitHub issue with the matching template, pre-filled from the person's text. */
export function githubIssueUrl(kind: FeedbackKind, text: string, info: AppInfo): string {
  const body = text.trim().slice(0, MAX_TEXT);
  const params = new URLSearchParams();
  if (kind === "bug") {
    params.set("template", "bug_report.yml");
    if (body) params.set("what", body);
    params.set("version", info.version);
    params.set("os", osLabel(info));
  } else if (kind === "idea") {
    params.set("template", "feature_request.yml");
    if (body) params.set("problem", body);
  } else {
    params.set("template", "feedback.yml");
    if (body) params.set("feedback", body);
    params.set("version", info.version);
    params.set("os", osLabel(info));
  }
  return `${REPO}/issues/new?${params}`;
}

/** The private form with the version and OS filled in. */
export function privateFormUrl(base: string, kind: FeedbackKind, info: AppInfo): string {
  const url = new URL(base);
  url.searchParams.set("kind", kind);
  url.searchParams.set("version", info.version);
  url.searchParams.set("os", osLabel(info));
  return url.toString();
}
