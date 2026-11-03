// Claude Code hooks for this repo (.claude/settings.json), so AI sessions follow AGENTS.md as
// they work instead of finding out in CI.
//
//   node scripts/claude-hook.mjs format   # after Edit/Write: formats the file it changed
//   node scripts/claude-hook.mjs lint     # before the session stops: `pnpm lint`
//
// The hook input arrives as JSON on stdin. Exit code 2 sends stderr back to Claude to fix.
import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { join, relative, sep } from "node:path";

const root = join(import.meta.dirname, "..");
const input = JSON.parse(readFileSync(0, "utf8") || "{}");
const run = (cmd, args, cwd = root) =>
  execFileSync(cmd, args, { cwd, encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] });

function format() {
  const path = input.tool_input?.file_path;
  if (!path) return;
  const file = relative(root, path).split(sep).join("/");
  if (file.startsWith("..")) return;
  if (file.endsWith(".rs") && file.startsWith("src-tauri/")) {
    run(
      "rustfmt",
      ["--edition", "2024", relative(join(root, "src-tauri"), path)],
      join(root, "src-tauri"),
    );
  } else if (/\.(ts|tsx|css|mjs|json)$/.test(file)) {
    // .prettierignore decides what Prettier owns.
    run("pnpm", ["exec", "prettier", "--write", "--ignore-unknown", file]);
  }
}

function lint() {
  // A second stop after a failed check: let it end, CI will report.
  if (input.stop_hook_active) return;
  try {
    run("pnpm", ["--silent", "lint"]);
  } catch (e) {
    process.stderr.write(
      `pnpm lint failed; fix these before finishing (AGENTS.md):\n${e.stdout}${e.stderr}`,
    );
    process.exit(2);
  }
}

try {
  ({ format, lint })[process.argv[2]]?.();
} catch (e) {
  process.stderr.write(`${e.stdout ?? ""}${e.stderr ?? e.message}`);
  process.exit(2);
}
