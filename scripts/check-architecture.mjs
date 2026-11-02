// Fails when code is put where AGENTS.md says it doesn't go: which layer may import which,
// where Tauri commands, HTTP clients and the AI query checks live, and which top-level Rust
// modules exist. Each rule names the AGENTS.md section that explains it.
//
//   node scripts/check-architecture.mjs        # part of `pnpm lint`
//
// To change a rule, change AGENTS.md and this file in the same pull request.
import { readFileSync, readdirSync } from "node:fs";
import { join, relative, sep } from "node:path";

const root = join(import.meta.dirname, "..");
const rel = (p) => relative(root, p).split(sep).join("/");

function files(dir, ext) {
  return readdirSync(dir, { withFileTypes: true }).flatMap((e) => {
    const path = join(dir, e.name);
    if (e.isDirectory()) return files(path, ext);
    return ext.test(e.name) ? [path] : [];
  });
}

/** Rust source without its `#[cfg(test)] mod … { … }` blocks (tests may do more). */
function withoutTests(text) {
  const lines = text.split("\n");
  const out = [];
  for (let i = 0; i < lines.length; i++) {
    if (
      lines[i].trim() === "#[cfg(test)]" &&
      /^\s*(pub(\(crate\))? )?mod \w+\s*\{/.test(lines[i + 1] ?? "")
    ) {
      let depth = 0;
      for (i = i + 1; i < lines.length; i++) {
        for (const c of lines[i]) depth += c === "{" ? 1 : c === "}" ? -1 : 0;
        if (depth === 0) break;
      }
      continue;
    }
    out.push(lines[i]);
  }
  return out.join("\n");
}

const problems = [];
const fail = (file, rule, section) => problems.push(`${file}: ${rule} (AGENTS.md: ${section})`);

// --- Frontend layers -------------------------------------------------------------------------
// Feature modules other features may import directly: pure helpers, no state or components.
const PURE_HELPERS = new Set(["schema/sql", "team/teamText", "settings/sections"]);

const ts = files(join(root, "src"), /\.(ts|tsx)$/);
for (const path of ts) {
  const file = rel(path);
  const text = readFileSync(path, "utf8");
  const imports = [...text.matchAll(/(?:from|import)\s*\(?\s*"([^"]+)"/g)].map((m) => m[1]);
  const reaches = (dir) =>
    imports.some((s) => s.startsWith(".") && new RegExp(`(^|/)${dir}(/|$)`).test(s));

  if (!file.startsWith("src/api/") && imports.includes("@tauri-apps/api/core")) {
    fail(file, "calls the backend outside src/api (use api/commands.ts)", "Frontend layers");
  }
  if (
    /^src\/(components|lib|hooks|api|styles)\//.test(file) &&
    (reaches("features") || reaches("app"))
  ) {
    fail(file, "a shared layer imports from features/ or app/", "Frontend layers");
  }
  if (file.startsWith("src/features/") && reaches("app")) {
    fail(file, "a feature imports from app/", "Frontend layers");
  }
  if (file.startsWith("src/components/") && reaches("api")) {
    fail(file, "a UI component imports from api/ (take data as props)", "Frontend layers");
  }
  // Another feature only through its index.ts, or a pure helper module listed here.
  const feature = file.match(/^src\/features\/([^/]+)\//)?.[1];
  const internal = feature
    ? /^\.\.\/([a-z]+)\/(\w+)$/ // from a feature: ../other/module
    : file.startsWith("src/app/")
      ? /^\.\.\/features\/([a-z]+)\/(\w+)$/ // from app/: ../features/x/module
      : null;
  const isTest = /\.test\.tsx?$/.test(file);
  for (const s of internal && !isTest ? imports : []) {
    const m = s.match(internal);
    if (!m || m[1] === feature || PURE_HELPERS.has(`${m[1]}/${m[2]}`)) continue;
    fail(
      file,
      `imports ${m[1]}/${m[2]}, another feature's internals (use its index.ts)`,
      "Frontend layers",
    );
  }
}

// --- Types shared with the backend -----------------------------------------------------------
// src/api/bindings.ts, generated from the Rust types (`pnpm gen:types`), is the only source of
// the backend's types and the only caller of `invoke`. src/api/types.ts renames its types for
// the UI and declares only these frontend-only helpers: a backend shape written by hand drifts.
const API_TYPE_HELPERS = new Set([
  "ConnectionConfig",
  "Target",
  "Dialect",
  "Commands",
  "Camel",
  "CommandName",
  "Wrapper",
  "CommandArgs",
  "CommandResult",
]);
const API_TYPES = "src/api/types.ts";
for (const m of readFileSync(join(root, API_TYPES), "utf8").matchAll(
  /^(?:export\s+)?(?:declare\s+)?(?:interface|type)\s+(\w+)/gm,
)) {
  if (!API_TYPE_HELPERS.has(m[1])) {
    fail(
      API_TYPES,
      `declares the type ${m[1]} (re-export it from ./bindings, or list a frontend-only helper in API_TYPE_HELPERS)`,
      "Types shared with the backend",
    );
  }
}
for (const path of ts) {
  const file = rel(path);
  const text = readFileSync(path, "utf8");
  const api = file.startsWith("src/api/");
  if (api && file !== "src/api/bindings.ts" && text.includes('"@tauri-apps/api/core"')) {
    fail(
      file,
      "calls invoke itself (use the generated commands in ./bindings)",
      "Types shared with the backend",
    );
  }
  if (!api && /(?:from|import)\s*\(?\s*"[^"]*\/bindings"/.test(text)) {
    fail(
      file,
      "imports the generated bindings (use api/types.ts and api/commands.ts)",
      "Types shared with the backend",
    );
  }
}

// --- Backend ---------------------------------------------------------------------------------
const rustRoot = join(root, "src-tauri", "src");

// New top-level modules are a design decision: add them here and to AGENTS.md's map.
const TOP_LEVEL = new Set([
  "ai",
  "app_menu",
  "audit",
  "bin",
  "commands",
  "db",
  "error",
  "handoff",
  "lib",
  "main",
  "mcp",
  "notes",
  "safety",
  "schema",
  "state",
  "store",
  "team",
  "voice",
]);
for (const e of readdirSync(rustRoot, { withFileTypes: true })) {
  const name = e.name.replace(/\.rs$/, "");
  if (!TOP_LEVEL.has(name)) {
    fail(
      `src-tauri/src/${e.name}`,
      "a new top-level module (put it under an existing one)",
      "Backend map",
    );
  }
}

// Files outside tests that may build their own HTTP client. Everything else uses
// AppState's shared client, so timeouts and redirect rules apply everywhere.
const OWN_HTTP_CLIENT = new Set([
  "src-tauri/src/state.rs", // the shared client
  "src-tauri/src/audit/knowledge.rs", // the crawler: pinned DNS, no redirects followed
]);
// The only place that checks SQL for an AI (never-send and forbidden functions): the AI query
// gate. The MCP server, agent mode and note checks call it; a new path that runs SQL for an AI
// must too.
const AI_QUERY_GATES = new Set(["src-tauri/src/safety/gate.rs"]);

for (const path of files(rustRoot, /\.rs$/)) {
  const file = rel(path);
  if (/(^|\/)(tests|[a-z_]+_tests)\.rs$/.test(file) || file.includes("/eval/")) continue;
  const text = withoutTests(readFileSync(path, "utf8"));

  if (text.includes("#[tauri::command]") && !file.startsWith("src-tauri/src/commands/")) {
    fail(file, "a Tauri command outside commands/", "Backend layers");
  }
  if (/reqwest::Client::(new|builder)\(/.test(text) && !OWN_HTTP_CLIENT.has(file)) {
    fail(file, "builds its own HTTP client (use AppState's)", "Backend layers");
  }
  if (
    /never_send::check\(/.test(text) &&
    file !== "src-tauri/src/safety/never_send.rs" &&
    !AI_QUERY_GATES.has(file)
  ) {
    fail(file, "a new place that checks SQL for an AI", "The AI query gate");
  }
  if (/\.unwrap\(\)|\.expect\(/.test(text) && file.startsWith("src-tauri/src/commands/")) {
    fail(file, "unwrap/expect in a Tauri command (return an error)", "Errors and panics");
  }
}

if (problems.length) {
  console.error(`Architecture rules broken:\n${problems.join("\n")}`);
  process.exit(1);
}
console.log(`Architecture: ${ts.length} frontend files and the backend follow AGENTS.md.`);
