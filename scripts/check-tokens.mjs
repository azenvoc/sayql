// Fails when a stylesheet or component uses a design token that src/styles/tokens.css doesn't
// define. CSS falls back silently on a missing token, so a typo wouldn't show up otherwise.
//
//   node scripts/check-tokens.mjs        # part of `pnpm lint`
import { readFileSync, readdirSync } from "node:fs";
import { join, relative } from "node:path";

const root = join(import.meta.dirname, "..");
const src = join(root, "src");

function files(dir) {
  return readdirSync(dir, { withFileTypes: true }).flatMap((e) => {
    const path = join(dir, e.name);
    if (e.isDirectory()) return files(path);
    return /\.(css|tsx?)$/.test(e.name) ? [path] : [];
  });
}

const tokens = readFileSync(join(src, "styles", "tokens.css"), "utf8");
const defined = new Set([...tokens.matchAll(/(--[a-z0-9-]+)\s*:/g)].map((m) => m[1]));
const missing = files(src).flatMap((path) =>
  [...readFileSync(path, "utf8").matchAll(/var\((--[a-z0-9-]+)/g)]
    .filter(([, name]) => !defined.has(name))
    .map(([, name]) => `${relative(root, path)}: ${name}`),
);

if (missing.length) {
  console.error(
    `Design tokens used but not defined in src/styles/tokens.css:\n${missing.join("\n")}`,
  );
  process.exit(1);
}
console.log(`Design tokens: ${defined.size} defined, every use matches.`);
