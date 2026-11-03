// The `wiring` check on pull request descriptions (scripts/check-pr-wiring.mjs).
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { expect, it } from "vitest";
import { checkWiring } from "./check-pr-wiring.mjs";

const template = readFileSync(
  join(import.meta.dirname, "..", ".github", "pull_request_template.md"),
  "utf8",
);

it("refuses the template as it is, with every item still open", () => {
  expect(checkWiring(template)).toHaveLength(9);
});

it("accepts every item ticked or marked n/a with a reason", () => {
  const body = template
    .replace(
      "- [ ] Team connections (`team:`)",
      "- n/a: Team connections, the change is only in the local editor",
    )
    .replace(/- \[ \]/g, "- [x]");
  expect(checkWiring(body)).toEqual([]);
});

it("refuses an n/a without a reason", () => {
  const body = template
    .replace("- [ ] Team connections (`team:`)", "- n/a: Team connections")
    .replace(/- \[ \]/g, "- [x]");
  expect(checkWiring(body)).toEqual(['"n/a: Team connections": say why it doesn\'t apply.']);
});

it("refuses a description without the Wiring section", () => {
  expect(checkWiring("## What and why\n\nA fix.")[0]).toMatch(/no "## Wiring" section/);
  expect(checkWiring(undefined)[0]).toMatch(/no "## Wiring" section/);
});

it("ignores the items of the other sections and the template's comments", () => {
  const body =
    "## Checklist\n\n- [ ] Follows AGENTS.md\n\n## Wiring\n\n<!-- - [ ] not an item -->\n- [x] Team connections\n";
  expect(checkWiring(body)).toEqual([]);
});
