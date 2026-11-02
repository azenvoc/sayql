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
