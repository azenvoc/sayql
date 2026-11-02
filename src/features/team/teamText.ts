import type { TeamAiStatus, TeamAiUsage, TeamServerInfo } from "../../api";

/** "SayQL for teams" on the website. */
export const TEAMS_URL = "https://sayql.dev/#teams";

/** The plan and licence as one line, e.g. "Team plan for Acme · 3 of 10 seats · until 2027-04-30". */
export function licenceText(s: TeamServerInfo): string {
  if (!s.licensed) return "No valid licence: queries can't run until your team admin adds one";
  const plan = s.plan ? s.plan[0].toUpperCase() + s.plan.slice(1) : "Team";
  // A server without a key runs its built-in trial, licensed to "Free trial".
  const parts = [
    s.licensee === "Free trial"
      ? "Free trial"
      : `${plan} plan${s.licensee ? ` for ${s.licensee}` : ""}`,
  ];
  if (s.users != null && s.userLimit != null) parts.push(`${s.users} of ${s.userLimit} seats`);
  if (s.expires) parts.push(`until ${s.expires}`);
  return parts.join(" · ");
}

/** "120k", "2M": token counts at a glance. */
export function tokens(n: number): string {
  if (n >= 1_000_000) return `${+(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${Math.round(n / 1_000)}k`;
  return String(n);
}

/** "120k of 2M tokens", or "120k tokens" with no limit. */
export function usageText(u: TeamAiUsage | null): string {
  if (!u) return "Unknown";
  return u.limit == null
    ? `${tokens(u.tokens)} tokens (no limit)`
    : `${tokens(u.tokens)} of ${tokens(u.limit)} tokens`;
}

/** "Anthropic Claude" or "an OpenAI-compatible service": the team's AI, for people. */
export function teamAiName(ai: TeamAiStatus): string {
  return ai.provider === "anthropic" ? "Anthropic Claude" : "an OpenAI-compatible AI service";
}

/** Team AI applies (on, with a model to use). */
export function teamAiOn(ai: TeamAiStatus | null | undefined): ai is TeamAiStatus {
  return !!ai?.enabled && !!ai.provider && !!ai.defaultModel;
}

/**
 * A whole invite link pasted where the server goes: `https://sayql.dev/join#server=…&code=…` or
 * `sayql://team-join?server=…&code=…`. Null for anything else.
 */
export function parseInviteLink(text: string): { server: string; code: string } | null {
  const t = text.trim();
  const query = t.startsWith("sayql://team-join?")
    ? t.slice("sayql://team-join?".length)
    : /^https?:\/\/[^#]*\/join\/?#/.test(t)
      ? t.slice(t.indexOf("#") + 1)
      : null;
  if (query == null) return null;
  const p = new URLSearchParams(query);
  const server = p.get("server")?.trim();
  const code = p.get("code")?.trim();
  return server && code ? { server, code } : null;
}

/** The host of a server address, for "sign in to sayql.acme.internal". */
export function serverHost(server: string): string {
  try {
    return new URL(server.includes("://") ? server : `https://${server}`).host || server;
  } catch {
    return server;
  }
}

/** Where agent mode's sends go on a team connection with team AI, as the agent settings say it. */
export function teamAiDestination(ai: TeamAiStatus): { label: string; local: boolean } {
  return {
    label: `your team's AI (${teamAiName(ai)}, through your company's SayQL server)`,
    local: false,
  };
}
