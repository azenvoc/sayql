# Architecture

SayQL is a Tauri 2 desktop app: a React + TypeScript UI (`src/`) on top of a Rust backend (`src-tauri/`). The UI never talks to a database or an AI provider directly; it calls typed Tauri commands.

## Frontend layout (`src/`)

```
src/
  main.tsx              entry: global styles, dev-only mock backend, <App/>
  app/                  the window shell; composes features, owns no business logic
    App.tsx             ThemeProvider + bootstrap gate
    Workbench.tsx       layout: TopBar, Navigator | tabs, StatusBar, dialogs; the navigator follows the active tab
    QueryTab.tsx        one editor tab: Ask bar + SQL editor over results, bound to one database
    TopBar.tsx, StatusBar.tsx, SqlWorkspace.tsx, useBootstrap.ts
  api/                  the only place that calls Tauri
    commands.ts         typed invoke() wrappers, one per Rust command
    events.ts           AI streaming event subscription
    types.ts            shapes mirrored from the Rust serde structs
  components/ui/        reusable, feature-agnostic primitives (Button, Dialog, Panel, SplitPane, TabBar, TreeRow, Form,
                        ContextMenu, BrandMark…)
  features/             one folder per product area; each exports its public API from index.ts
    agent/              agent mode: run state (agentRun.ts reducer, useAgentRun), the Agent view, the
                        per-connection opt-in dialog and access store, note editing, Settings → Agent mode
    ask/                plain-English input, instant fast path, AI generation hook
    connections/        connection dialog, kinds metadata, useConnections
    editor/             Monaco setup (offline, themed) and SqlEditor
    feedback/           feedback dialog: GitHub issue and private form links pre-filled with version and OS
    navigator/          Database Navigator tree and its right-click menus
    onboarding/         first-run setup guide and starter questions
    query/              useQueryRunner, write confirmation
    results/            virtualized DataGrid, ResultsPanel, CSV/format helpers
    schema/             dialect-aware SQL helpers (quoting, qualified names, SQL templates)
    scripts/            saved scripts, save and close-tab dialogs
    settings/           settings dialog (AI provider, models, MCP) and state
    skills/             saved Ask-box skills (a /slug that adds an instruction to the prompt): list and editor
    tabs/               open editor tabs (persisted), each bound to a (connection, database) target
    theme/              ThemeProvider (system/light/dark) and toggle
    updates/            update check and install from the status bar
  hooks/                generic hooks (useLatest, usePersistentState)
  lib/                  generic utilities (cx, storage, errors) and sqlStatements.ts (splits SQL text into
                        statements outside strings and comments, for running a selection or a script)
  styles/               tokens.css (design tokens, light + dark) and base.css
  dev/                  development-only code (mock backend for browser preview)
```

### Rules

- **Dependencies point inward:** `app → features → components/ui, hooks, lib, api`. Features may use another feature's public `index.ts` (or a pure helper module such as `schema/sql.ts`), never its internals. `components/ui` never imports from `features` or `api`.
- **State lives in hooks, views stay presentational.** Each feature exposes a `useX` hook for its state and side effects; components receive data and callbacks as props.
- **One way to style:** CSS Modules next to the component (`Foo.tsx` + `Foo.module.css`), using only variables from `styles/tokens.css`. No hex colors or pixel font sizes outside `tokens.css` (the Monaco theme in `features/editor/monacoSetup.ts` is the single documented exception, because Monaco needs literal colors).
- **Theming:** `<html data-theme="light|dark">` switches the color tokens. New colors are added to both theme blocks in `tokens.css`.
- **Icons:** `lucide-react` only, 13–16px.
- **Naming:** components `PascalCase.tsx`, hooks `useCamelCase.ts`, helpers `camelCase.ts`, tests next to the code as `*.test.ts`.

## Backend layout (`src-tauri/src/`)

```
src-tauri/src/
  lib.rs                plugin + state + command registration only
  mcp.rs                sayql-mcp: MCP server over stdio (read-only tools), reusing state/db/schema
  bin/sayql-mcp.rs      its entry point
  safety/               the product's promise in one place: what keeps data from an AI that may not see it
    gate.rs             the AI query gate: every query run for an AI (MCP, agent mode, note checks) is checked and
                        run here (read-only, never-send, time limit, redacted errors), with per-caller options
    sql_lex.rs          the SQL lexer every check shares (strings, quoted names and comments per dialect)
    classify.rs         is_read_only (also db::is_read_only): per-dialect lexer; reading first keyword and no
                        write keyword anywhere
    never_send.rs       never-send columns: refuses SQL that could read one (names, the Postgres plan, shapes)
    readonly.rs         read-only access shared by MCP and agent mode: schema search (describe), queries stopped
                        on the server after a time limit (run_limited), row caps, the write refusal
    redact.rs           database error messages in a fixed form for an AI that may not see values (safe_error)
    pii.rs              likely personal data in text, for the masker and note checks
    mask/               masks likely personal data in results as per-run tokens (‹email-3›), puts real values back
                        into SQL as dialect literals, and builds the "counts and summaries only" view
  notes.rs              database notes: checks, review after a schema change, suggested fixes
  audit/                the audit: an AI checks a database's notes against code, files and web pages
    mod.rs              the audit prompt and its sources
    run.rs              a run from its checks to its end: which agents can run, who pays, the run folder and
                        instructions, starting Claude Code or the built-in OpenCode, progress events, Terminal
    sources.rs          a connection's knowledge sources: adding, checking and reading them again, and bringing
                        them up to date before a run
    runner.rs, runner/  runs the audit in a coding agent (Claude Code, Codex, ...), read-only; run_dir.rs: the
                        private run folder
    opencode.rs, opencode/  the built-in OpenCode audit: pinned download, deny-first config, private HOME,
                        secret files kept out (secrets.rs)
    process.rs          child processes: process groups, stop and kill
    coding_agents.rs    which coding agents are installed
    knowledge.rs        knowledge sources: file text (PDF, DOCX, HTML) and the web crawler
    knowledge/pdf.rs    PDF text, read by SayQL's own executable in a child process (`--extract-pdf`,
                        wired in main.rs) with a time, output and (Linux) memory limit
    repos.rs            git and local code sources copied for the audit
  team/                 the team server client (mod.rs): sign-in, queries, shared notes, team AI (ai.rs),
                        shared knowledge (knowledge.rs)
  app_menu.rs           the native menu
  handoff.rs            sayql:// links from sayql-mcp to the app (prefilled connection, SQL tab): validation, one-time tokens
  error.rs              AppError / AppResult: the single error type; serializes to a message for the UI
  state.rs              AppState: store, open connections, the running AI generation
  commands/             Tauri commands, one file per UI area (thin: validate, call modules, map results)
    app.rs              app_info: version, OS and architecture, for pre-filling feedback
    connections.rs      init, save/delete/test/open connection, refresh schema
    query.rs            run_query (write confirmation gate)
    agent.rs            start/stop/reply_agent (agent://event), per-connection agent access, database notes
    generation.rs       generate_sql, cancel_generation
    scripts.rs          list/save/delete saved scripts
    settings.rs         save_settings, model choice, MCP setup and AI apps, activity logs (logic in ai/settings.rs)
    audit.rs            coding agents, knowledge sources (file and folder pickers), audit runs (audit://event);
                        the logic is in audit/run.rs and audit/sources.rs
    handoff.rs          receives sayql:// links, queues valid handoffs for the window (take_handoffs)
    voice.rs            voice_start/stop/cancel, model download and removal (events: voice://level, voice://progress)
    skills.rs           list/save/delete skills; validate (name rules, no clash with built-in commands)
    updates.rs          check_update, install_update (tauri-plugin-updater; never in development builds)
  db/                   database access, no Tauri types
    mod.rs              Kind, ConnConfig, DbPool, QueryResult, public API
    connect.rs          pools per database kind
    databases.rs        the databases on a server, and connecting to one of them
    mssql_pool.rs       SQL Server connection (tiberius), Azure redirects, host,port parsing
    execute.rs          run_query: read-only transactions, row cap, column headers
    decode/             driver values → JSON for display (postgres binary wire format, mysql, sqlite, mssql)
    introspect/         schema reading per database kind, shared assembly in mod.rs
    demo.rs, demo.sql   the bundled demo database
    tests.rs            SQLite end-to-end tests
    integration_tests.rs  PostgreSQL/MySQL/SQL Server tests (opt-in via env vars, see CONTRIBUTING.md)
    tenant_fixture.rs   tests only: a synthetic multi-tenant health database with planted answers and personal data,
                        written for each dialect (SQLite always, the servers when their env vars are set)
  ai/                   English → SQL over any AI provider, no knowledge of Tauri commands
    mod.rs              stream_sql (picks the client per Endpoint) + event payloads
    providers.rs        the provider catalog: Claude plus OpenAI-compatible presets
    anthropic.rs        Claude Messages API, prompt caching, API error messages
    openai.rs           OpenAI-compatible Chat Completions (OpenAI, Gemini, Ollama, …), model lists
    models.rs           Claude's models and per-model request options
    prompt.rs           the English → SQL prompt, shared by every provider
    sse.rs              server-sent-event parser
    settings.rs         Settings → AI provider: the settings view sent to the UI (never a key), checking and saving
                        a provider's choices, model switching, listing a provider's models
    agent/              agent mode: answers a question in several read-only steps with a cited answer
      run.rs            the loop: turns, tools run through the read-only gate, result setting and Ask before
                        send, step/query/time/token limits, Stop, events to the tab
      model.rs          the provider-neutral conversation (text, tool calls, results) and a scripted mock
      tools.rs          the tools the model may call (plan, search, describe, profile, query, explain, ask, finish)
      prompt.rs         instructions, the "why" playbook, the schema and notes block
      checks.rs         checks in code: joins that repeat rows before a sum, numbers no step backs, big scans
      text_protocol.rs  one fenced JSON action per reply, for models without tool calling
      eval/             tests only: the evaluation question set (questions.json, demo store and tenant fixture);
                        `Reference` checks the set, `agent.rs` runs it with a real model (opt-in, API key)
  schema/               schema model (mod.rs) and compact prompt rendering (render.rs)
  store/                settings/connections JSON (mod.rs), keychain secrets (secrets.rs), and agent mode's
                        per-connection access, run limits and database notes (agent.rs)
  voice/                voice input, all on this computer, no Tauri types
    model.rs            the Whisper model: pinned URL and SHA-256, download, path in app data
    audio.rs            microphone capture with cpal, mono 16 kHz resampling
    transcribe.rs       whisper.cpp (whisper-rs) speech to text
```

### Rules

- **Commands are thin.** Business logic lives in `db/`, `ai/`, `safety/`, `schema/`, `store/` and `voice/`, which never import Tauri command types; only `commands/` and `state.rs` know about Tauri state.
- **Errors:** functions return `AppResult<T>`. Use `?` on sqlx/io/keychain/JSON errors (converted by `From`), and `"message".into()` for messages written for the user. A message that mentions "Settings" makes the UI offer an Open Settings button.
- **Secrets** only go through `store::secrets`; they are never written to JSON or sent to the UI.
- **Tests** sit next to the code (`#[cfg(test)] mod tests`).
- **JSON field names are camelCase** (`#[serde(rename_all = "camelCase")]`), with one exception: the schema model
  (`schema::{Schema, Table, Column, ForeignKey}`) keeps its Rust snake_case names (`foreign_keys`, `data_type`,
  `primary_key`, `ref_table`, …). It is only serialized, never read back or written to disk; MCP clients and the
  AI get it as rendered text (`schema::render`), not JSON, so the UI is its only JSON consumer. Its TypeScript
  mirror in `src/api/types.ts` and the code reading it (navigator, editor completions, SQL templates, fast path,
  starter questions, mock backend) all use the snake_case names, so renaming is a coordinated change across
  both sides with no benefit beyond consistency.

## Testing

`pnpm test` runs the Vitest suite (pure frontend logic) and `cargo test --lib` (SQL classification, SSE parsing, API error messages, settings validation, and an end-to-end run against a generated SQLite demo database). The PostgreSQL/MySQL/SQL Server integration tests run when their env vars are set (see CONTRIBUTING.md).

## Previewing the UI without the backend

`pnpm dev` and open http://localhost:1420 in a browser: `src/dev/mockBackend.ts` fakes the Rust commands with demo data, including streamed SQL. It is only loaded in development outside Tauri.

## Outside the app

```
site/                 the website (landing page, built docs), deployed to Vercel
docs-site/            docs source: pages/*.html and the template the docs build uses
scripts/              build_docs.py (docs) and third_party_notices.py (credits shipped with the app)
  video/              records the browser preview and renders the explainer videos (see its README)
media/                the rendered explainer videos and poster, their cut lists and voiceover
.github/              CI, the release workflow, issue and PR templates, README images
```
