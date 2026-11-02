// The lint ratchet: every clippy and oxlint warning counts as an error, except the ones
// lint-baseline.json already records, per file and rule. Those counts may only go down:
// a change that adds a warning fails, and so does one that removes a warning without lowering
// the baseline, so the next change can't quietly spend it.
//
//   node scripts/lint-ratchet.mjs           # check (CI: the `clippy` and `consistency` jobs)
//   node scripts/lint-ratchet.mjs --update  # after fixing warnings: write the lower counts
//   node scripts/lint-ratchet.mjs --only=clippy|oxlint
//   node scripts/lint-ratchet.mjs --init    # only when a pull request adds a lint rule
//
// `--update` refuses to raise a count: fix the new warning, or put
// `#[expect(lint, reason = "...")]` (Rust) / `// oxlint-disable-next-line rule -- reason` (TS)
// on the line if it is truly right there.
import { execFileSync } from "node:child_process";
import { readFileSync, writeFileSync } from "node:fs";
import { join, relative } from "node:path";

const root = join(import.meta.dirname, "..");
const baselinePath = join(root, "lint-baseline.json");
const args = process.argv.slice(2);
// `--init` records the counts as they are; only for adding a new lint rule, in its own commit.
const init = args.includes("--init");
const update = init || args.includes("--update");
const only = args.find((a) => a.startsWith("--only="))?.slice("--only=".length);

/** `file::rule` → count, from clippy's JSON messages (each site once, across targets). */
function clippy() {
  const out = execFileSync(
    "cargo",
    ["clippy", "--all-targets", "--message-format=json", "--quiet"],
    { cwd: join(root, "src-tauri"), encoding: "utf8", maxBuffer: 1 << 30, stdio: ["ignore", "pipe", "inherit"] },
  );
  const seen = new Set();
  const counts = {};
  let errors = 0;
  for (const line of out.split("\n")) {
    if (!line.startsWith("{")) continue;
    const m = JSON.parse(line);
    if (m.reason !== "compiler-message") continue;
    const msg = m.message;
    if (msg.level === "error") errors++;
    if (msg.level !== "warning" || !msg.code) continue;
    const span = msg.spans.find((s) => s.is_primary) ?? msg.spans[0];
    if (!span) continue;
    const file = relative(root, join(root, "src-tauri", span.file_name));
    const site = `${file}:${span.line_start}:${span.column_start}:${msg.code.code}`;
    if (seen.has(site)) continue;
    seen.add(site);
    const key = `${file}::${msg.code.code}`;
    counts[key] = (counts[key] ?? 0) + 1;
  }
  if (errors) {
    console.error(`clippy: ${errors} error(s); run \`cargo clippy --all-targets\` in src-tauri.`);
    process.exit(1);
  }
  return counts;
}

/** `file::rule` → count, from oxlint's JSON report (errors already fail `pnpm lint`). */
function oxlint() {
  let out;
  try {
    out = execFileSync("pnpm", ["exec", "oxlint", "src", "--format", "json"], {
      cwd: root,
      encoding: "utf8",
      maxBuffer: 1 << 30,
    });
  } catch (e) {
    out = e.stdout; // oxlint exits non-zero on errors; `pnpm lint` reports those.
  }
  const counts = {};
  for (const d of JSON.parse(out).diagnostics) {
    if (d.severity !== "warning") continue;
    const key = `${d.filename}::${d.code}`;
    counts[key] = (counts[key] ?? 0) + 1;
  }
  return counts;
}

const tools = { clippy, oxlint };
const baseline = JSON.parse(readFileSync(baselinePath, "utf8"));
let failed = false;

// `--against=<main's lint-baseline.json>` (CI, on pull requests): the baseline itself may only
// shrink. A count may rise only for a rule main's baseline doesn't have yet (a newly added rule).
const against = args.find((a) => a.startsWith("--against="))?.slice("--against=".length);
if (against) {
  const main = JSON.parse(readFileSync(against, "utf8"));
  const raised = [];
  for (const tool of Object.keys(tools)) {
    const old = main[tool] ?? {};
    const rules = new Set(Object.keys(old).map((k) => k.split("::").pop()));
    for (const [key, n] of Object.entries(baseline[tool] ?? {})) {
      if (rules.has(key.split("::").pop()) && n > (old[key] ?? 0)) {
        raised.push(`  ${tool} ${key}: ${n} (main has ${old[key] ?? 0})`);
      }
    }
  }
  if (raised.length) {
    console.error(`lint-baseline.json raises counts main already tracks. Fix the warnings instead:\n${raised.join("\n")}`);
    process.exit(1);
  }
  console.log("lint-baseline.json: no count raised against main.");
  process.exit(0);
}

for (const [tool, run] of Object.entries(tools)) {
  if (only && only !== tool) continue;
  const now = run();
  const before = baseline[tool] ?? {};
  const more = [];
  const fewer = [];
  for (const key of new Set([...Object.keys(now), ...Object.keys(before)])) {
    const n = now[key] ?? 0;
    const b = before[key] ?? 0;
    if (n > b) more.push(`  ${key}: ${n} (allowed ${b})`);
    if (n < b) fewer.push(`  ${key}: ${n} (baseline ${b})`);
  }
  const total = Object.values(now).reduce((a, b) => a + b, 0);
  if (more.length && !init) {
    failed = true;
    console.error(`${tool}: new warnings. Fix them; see AGENTS.md for the rules.\n${more.join("\n")}`);
  } else if (update) {
    baseline[tool] = Object.fromEntries(Object.entries(now).sort(([a], [b]) => a.localeCompare(b)));
    console.log(`${tool}: baseline written (${total} warnings left to fix).`);
  } else if (fewer.length) {
    failed = true;
    console.error(
      `${tool}: fewer warnings than the baseline. Good: run \`pnpm lint:ratchet --update\` and commit lint-baseline.json.\n${fewer.join("\n")}`,
    );
  } else {
    console.log(`${tool}: no new warnings (${total} left to fix in the baseline).`);
  }
}

if (update && !failed) writeFileSync(baselinePath, `${JSON.stringify(baseline, null, 2)}\n`);
process.exit(failed ? 1 : 0);
