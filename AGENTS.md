# How code is written in SayQL

These rules apply to everyone who changes this repository: people, Claude Code, Codex, Cursor and
any other agent. They are not suggestions. CI checks each of them, and a pull request that fails a
check can't be merged. `CLAUDE.md` points here, `CONTRIBUTING.md` links here, and
[docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) has the full map of the code.

Before you push, run:

```bash
pnpm check
```

It runs everything CI runs, except the database integration tests and the other-platform builds.

## Changing a rule

A rule changes only in a pull request that changes this file and the check that enforces it
(`.oxlintrc.json`, `src-tauri/Cargo.toml` `[lints]`, `scripts/check-architecture.mjs`,
`.prettierrc.json`, `rustfmt.toml`) together. Never switch a check off, skip it, or work around it
in a feature pull request.

## Formatting

- **Frontend, scripts and JSON:** Prettier (`.prettierrc.json`, width 100). Run `pnpm format`.
- **Rust:** rustfmt with the 2024 style. Run `cargo fmt` in `src-tauri`.
- **Toolchains are pinned:** Rust in `src-tauri/rust-toolchain.toml`, Node in `.node-version`,
  pnpm in `package.json` `packageManager`. Upgrade them in their own pull request.

## Lints and the ratchet

- **TypeScript (oxlint, `.oxlintrc.json`):** no `any`, no `@ts-ignore`, `import type` for types,
  no `console`, no `var`, `const` where possible, no import cycles, every hook dependency listed.
- **Rust (`[lints]` in `src-tauri/Cargo.toml`):** no `unwrap`/`expect`/`panic!`/`todo!` outside
  tests, no indexing or slicing that can panic, no byte-slicing of text, no `println!`/`eprintln!`,
  no `unsafe`, and every `allow` gives a `reason`.
- **The ratchet (`lint-baseline.json`):** code written before a rule existed is counted per file
  and rule. `pnpm lint:ratchet` fails when a count goes up, and also when it goes down without
  the baseline being lowered (`pnpm lint:ratchet --update`). Counts never go back up.
- **Moving a file** renames its keys in the baseline. In a commit that only moves code, run
  `pnpm lint:ratchet --update --moved`: it accepts counts under new keys as long as no rule's
  total across all files rises, and a key the baseline already has still may not rise. On pull
  requests CI compares the baseline with main's the same way: a key main has may never rise, and
  a new key is allowed only while its rule's total doesn't rise above main's.
- **An exception** is written on the line, with its reason:
  `#[expect(clippy::indexing_slicing, reason = "i < len checked above")]`, or
  `// oxlint-disable-next-line no-console -- why`. A reviewer must agree with the reason.

## Frontend layers

Dependencies point inward: `app → features → components/ui, hooks, lib, api`.

- Only `src/api/` calls the backend (`invoke`); everything else goes through `api/commands.ts`.
- `components/ui`, `lib`, `hooks` and `api` never import from `features/` or `app/`;
  `components/` never imports from `api/` (it takes data as props).
- A feature uses another feature only through its `index.ts`, or through a pure helper module
  listed in `PURE_HELPERS` in `scripts/check-architecture.mjs`. Features never import from `app/`.
- State lives in a `useX` hook; components are presentational. Styles are CSS Modules next to the
  component, using only tokens from `styles/tokens.css` (`scripts/check-tokens.mjs`).
- Naming: components `PascalCase.tsx`, hooks `useCamelCase.ts`, helpers `camelCase.ts`, tests
  `*.test.ts(x)` next to the code.

## Backend map

The top-level modules of `src-tauri/src` are fixed (`TOP_LEVEL` in
`scripts/check-architecture.mjs`). New code goes under an existing module. Adding a top-level
module is a design change: it takes a pull request that updates this file, the map in
`docs/ARCHITECTURE.md` and the list.

Where new code goes:

| Code | Module |
|---|---|
| Tauri commands | `commands/<area>.rs`, thin: check input, call a module, map the result |
| Database access, decoding, introspection | `db/` (no Tauri types) |
| SQL lexing, read-only and never-send checks, personal-data detection, error redaction | `sql_lex`, `db/classify`, `never_send`, `pii`, `ai/redact`, `ai/agent/mask` |
| AI providers, prompts, agent mode | `ai/` |
| The audit and its runners, knowledge sources | `audit*`, `knowledge`, `repos` |
| Saved files and secrets | `store/` (secrets only through `store::secrets`) |
| Team server client | `team/` |

## Backend layers

- `#[tauri::command]` only in `commands/`. Commands hold no business logic.
- Every HTTP request uses `AppState`'s shared `reqwest::Client`. Only `state.rs` (the shared
  client) and `knowledge.rs` (the crawler, which pins DNS and checks every redirect) build their
  own client.
- JSON sent to the UI is camelCase (`#[serde(rename_all = "camelCase")]`). The exception is
  explained in docs/ARCHITECTURE.md.

## The AI query gate

SQL that runs for an AI (the MCP server, agent mode, and checks on notes) goes through
`never_send::check` and `db::is_read_only` before it runs, and through `ai::redact::safe_error`
when it fails. Only `mcp.rs`, `ai/agent/run.rs` and `notes.rs` call the check. A new place that
runs SQL for an AI calls one of them, never the database directly.

These rules are the product's promise ("Let agents into your database. Not your data."):

- Never weaken read-only safety, never-send columns, masking or error redaction. A change to any
  of them comes with tests that show the old refusals still refuse.
- Never send table rows to an AI except as the connection's "AI may see" setting allows.
- Secrets live only in the OS keychain. They are never logged, written to JSON, put in a URL,
  or sent to the UI.

## Errors and panics

- Release builds abort on any panic, so code never panics on data from outside: database values,
  files, HTTP responses, model output or user input. Use `.get()`, `.first()`, `checked_*`,
  `char_indices()` and `?`.
- Functions return `AppResult<T>`. Messages for the person are whole sentences that say what
  happened and what to do (`"Could not find the Downloads folder."`).
- Tauri commands never `unwrap` or `expect`.

## Comments and naming

- Doc comments (`///`, `/** */`) say what a thing is for and why, in plain sentences. Don't
  restate the code.
- A constant shared with the other side says where its twin is: `Must match X in src/api/types.ts`.
- Tests are named for the behaviour they check, as a sentence:
  `fn never_send_columns_are_refused_in_where()`.

## Tests

- Tests sit next to the code: `#[cfg(test)] mod tests` or `tests.rs` in Rust, `*.test.ts(x)` in
  the frontend.
- A bug fix comes with a test that fails without it. A safety change comes with tests for the
  bypasses it closes.

## Dependencies

- New npm packages are pinned to an exact version (`pnpm add -E`). New crates name a version and
  say in a comment in `Cargo.toml` why they are needed when it isn't obvious.
- `pnpm audit` and Dependabot alerts are triaged: either upgrade, or note why the advisory can't
  be reached.

## Pull requests

- One change per pull request. Formatting-only and rule changes go in their own pull request.
- Fill in the template. CI must be green, and the code owner reviews.
- Never commit secrets, real customer data, or marketing drafts.
