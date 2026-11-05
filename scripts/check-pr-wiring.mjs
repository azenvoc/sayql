// The `wiring` check: a pull request's description has the template's Wiring section, and each
// item in it is ticked or marked "n/a:" with a reason (see "Wiring a feature in" in AGENTS.md).
//
//   PR_BODY="$(gh pr view 12 --json body --jq .body)" node scripts/check-pr-wiring.mjs
import { pathToFileURL } from "node:url";

/** The items of the Wiring section, one per line, and the problems with them. */
export function checkWiring(body) {
  const lines = (body ?? "").replace(/<!--[\s\S]*?-->/g, "").split(/\r?\n/);
  const start = lines.findIndex((l) => /^##\s+Wiring\s*$/i.test(l.trim()));
  if (start === -1)
    return ['The description has no "## Wiring" section. Copy it from the pull request template.'];
  const items = [];
  for (const line of lines.slice(start + 1)) {
    if (/^#{1,2}\s/.test(line.trim())) break;
    if (/^\s*[-*]\s/.test(line)) items.push(line.trim().replace(/^[-*]\s+/, ""));
  }
  if (items.length === 0)
    return ["The Wiring section lists nothing. Copy its items from the pull request template."];
  const problems = [];
  for (const item of items) {
    if (/^\[[xX]\]\s+\S/.test(item)) continue;
    const na = item.match(/^n\/a:\s*(.*)$/i);
    if (na) {
      // The reason follows the item's name: at least a few words of it.
      if (na[1].split(/\s+/).filter(Boolean).length < 4)
        problems.push(`"${item}": say why it doesn't apply.`);
      continue;
    }
    problems.push(`"${item}": tick it, or replace "[ ]" with "n/a:" and say why.`);
  }
  return problems;
}

if (import.meta.url === pathToFileURL(process.argv[1]).href) {
  const problems = checkWiring(process.env.PR_BODY);
  if (problems.length > 0) {
    console.error("Wiring (see AGENTS.md, Wiring a feature in):");
    for (const p of problems) console.error(`  - ${p}`);
    process.exit(1);
  }
  console.log("Wiring: every item is ticked or marked n/a with a reason.");
}
