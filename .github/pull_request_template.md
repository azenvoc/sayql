## What and why

<!-- What does this change, and why? Link the issue it fixes, e.g. "Fixes #12". -->

## How I tested it

<!-- What you ran or tried. `pnpm check` must pass. -->

## Checklist

- [ ] Follows [AGENTS.md](../AGENTS.md); `pnpm check` passes
- [ ] No new lint exceptions, or each one is `#[expect(..., reason = "...")]` / `-- reason` and explained here
- [ ] Tests added or updated (a bug fix has a test that fails without it)
- [ ] Read-only safety, never-send columns, masking, keychain storage and "no table rows to the AI" are unaffected, or the change has tests showing the old refusals still refuse
- [ ] Docs updated if behaviour changed (`docs-site/pages`, `docs/ARCHITECTURE.md`)

## Wiring

<!-- See "Wiring a feature in" in AGENTS.md. Tick each item, or replace "[ ]" with "n/a:" and say why. -->

- [ ] Team connections (`team:`)
- [ ] The Ask box, Investigate and MCP (`sayql-mcp`)
- [ ] Safety: the AI query gate, never-send, what AI apps may see, masking, read-only
- [ ] Saved data: the store's update helpers, serde defaults for older files
- [ ] Knowledge: notes, checks, audits, knowledge sources
- [ ] The team server (`azenvoc/sayql-server`): its own pull request, or why not
- [ ] The UI preview (`src/dev/mockBackend.ts`)
- [ ] Docs, CHANGELOG, ARCHITECTURE; README and landing page claims
- [ ] Tests where it meets existing features
