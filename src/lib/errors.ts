/** Human-readable text for anything thrown by a Tauri command or the browser. */
export function errorText(e: unknown): string {
  if (typeof e === "string") return e;
  if (e instanceof Error) return e.message;
  if (isTeamRefusal(e)) return e.message;
  return JSON.stringify(e);
}

/** A team server refused the request by its rules (read-only, never send, licence, sign-in): not a database error. */
export function isTeamRefusal(e: unknown): e is { message: string; teamRefused: true } {
  return (
    typeof e === "object" &&
    e !== null &&
    (e as { teamRefused?: unknown }).teamRefused === true &&
    typeof (e as { message?: unknown }).message === "string"
  );
}
