# Changelog

All notable changes to SayQL are listed here. Downloads are on the
[releases page](https://github.com/dithzz/sayql-releases/releases).

## Unreleased

### Update dialog

- **See what's new before you update.** When there's a newer version, SayQL shows its version, date and release
  notes, the changes from this changelog. **Upgrade…** downloads and installs it with a progress bar, then
  restarts; **Details…** opens the release page.
- **Don't show for version X** keeps the dialog away until the next version. The status bar's update button
  still opens it.

## 0.5.5 (2026-11-03)

### Security fixes from a code audit

- **Never-send columns hold in more queries.** On MySQL 5.7 and MariaDB, `PROCEDURE ANALYSE()` could return the
  values of a never-send column; AI queries using it are now refused. So are signed or bracketed positions in
  `ORDER BY` and `GROUP BY` (`ORDER BY +3`) that could sort by a withheld column.
- **No sending data out from the database.** AI queries can't call functions that send data to the internet,
  such as Postgres's `http_*` functions and RDS's `aws_s3.*` and `aws_lambda.*`.
- **Less goes to the AI about never-send columns.** Their comments, enum values and CHECK constraints are left
  out of the schema the AI sees.
- SQL Server's `SETUSER`, `WRITETEXT` and `UPDATETEXT` count as writes.
- In agent mode, a note's failed check shows the AI only the safe form of the database's error.

### Fewer crashes

- **A broken PDF can't close SayQL.** PDFs are read in a separate process with time and size limits; a PDF
  SayQL can't read shows a message instead.
- Fixed crashes when a renamed table or column appeared in a note with letters such as `İ`, on a reply from a
  team server whose rows didn't match its columns, and on unusually deep SQL Server geometry or huge Postgres
  arrays.

### Audits

- On Linux, an audit's working folder is now private to you, in SayQL's cache, rather than in the shared
  `/tmp`. It's removed when the run ends, on every platform.
- Secret files are kept out of OpenCode audits whatever their letter case (`.ENV`, `ID_RSA`), and more of them
  are: `.envrc`, `.dev.vars`, `*.tfstate`, `*.tfvars`, `.pgpass`, `.my.cnf`, keystores and more.
- The OpenCode and ripgrep executables are checked against SHA-256s built into SayQL, downloads time out
  instead of hanging, and folder paths work with OpenCode on Windows.

### Reliability

- The app and SayQL's MCP server lock SayQL's saved files while changing them, so neither loses the other's
  changes (settings, notes, suggestions, sources).

## 0.5.4 (2026-11-02)

### Audits without Claude Code

- **SayQL's built-in OpenCode.** Without Claude Code, **Run audit** downloads OpenCode 1.19.5 (and the ripgrep
  it searches with, about 66 MB) once, checks both against pinned SHA-256s, and runs the audit with the AI you
  chose in Settings → AI provider: SayQL AI credits, your own Anthropic, OpenAI or Gemini key, or a local model.
  People with Claude Code keep using it; when both are there, **Audit with** picks.
- It runs read-only: only reading and searching the knowledge sources and SayQL's audit tools. No commands,
  file edits, web or subagents, no `.env` files or keys, and nothing from your own or the project's OpenCode
  setup. Searches skip secret files too, and a symbolic link out of the sources stops the run with its name.
  SayQL won't run it under an administrator's OpenCode policy.

## 0.5.3 (2026-11-02)

### Knowledge from the database itself

- **Comments and CHECK constraints.** SayQL reads what your database already says about its tables and
  columns (comments, and SQLite's `--` comments in `CREATE TABLE`) and its CHECK constraints, and shows them
  to the AI with the schema. Postgres, MySQL, MariaDB, SQL Server and SQLite. Nothing to turn on.
- The demo database now has comments, so new installs see this on day one.

### Knowledge sources

- **Code sources become knowledge sources.** Besides local folders and git repositories, a connection can
  now read **files** (Markdown, text, CSV, JSON, YAML, SQL, HTML, PDF and Word) and **web pages** (one page, or
  the pages in its folder on the same site). The audit reads them all, cites documents as `label:page@version`,
  and treats documents as possibly out of date: the code and the database win, and conflicts are shown.
- Web pages are fetched without cookies, follow robots.txt when crawling, and never reach addresses on your
  computer or private network.
- Up to 3 sources per connection are free. SayQL for teams keeps unlimited sources in sync for everyone.

### Shared notes on team connections

- On a connection from a team server that shares knowledge, the Ask box and Investigate use the team's
  approved notes. The navigator shows them read-only, and **Suggest a note…** sends a suggestion to the
  team's knowledge owners. Notes Investigate suggests go to them too.
- Older team servers keep using your local notes.

## 0.5.2 (2026-10-30)

### Notes that stay true

- **Notes know what they're about.** When you save a note about a database, SayQL finds the tables and columns
  it names and remembers what they were like. The note dialog shows them.
- **Schema changes are caught.** Before each question, at most once a minute, SayQL checks whether the
  database's schema changed (one small read-only query), and reads it again right away when a query fails
  because a table or column doesn't exist. A note whose table or column was dropped, renamed or changed is
  marked **may be out of date**, with the reason, and SayQL suggests the fix: the note rewritten with the new
  name after a rename, or its removal after a drop. Nothing changes until you accept.
- **Checks.** A note can carry a read-only query that returns true while the note holds. SayQL runs it when
  the note is saved, weekly when you open the database, when you click **Check notes**, and when Investigate
  asks. Only true or false is kept. A failing check marks the note out of date; a passing one shows **checked**.
- **The schema wins.** The Ask box and Investigate now get only the notes about the tables in the question,
  each marked checked or may be out of date, and are told to follow the schema when a note disagrees with it.
  AI apps over MCP see the same marks.
- **Investigate corrects notes.** It can run a note's check mid-run, and the notes it suggests (new ones, and
  fixes to saved ones, with checks) wait for you with the other suggestions.
- A doubtful note shows a warning sign in the navigator and a banner. Open it to see why, the suggested fix,
  its check and its earlier versions, or choose **It's still true**.

### The audit reads all your code

- **Several code sources per connection:** local folders and git repositories on GitHub, GitLab, Bitbucket or
  any host. SayQL fetches each repository with your own git login (or a read-only token kept in your
  keychain) before the audit, and the audit reads all of them. Notes cite their evidence as
  `source:path@commit`.

### New logo

- **"Shape, not data":** a database whose schema (what the AI sees) sits above rows it doesn't. New app icons,
  and the logo across the app, sayql.dev and the docs.

## 0.4.12 (2026-10-30)

### Teams

- **Sign in with your company.** On a team server that offers it (team server 0.4.0), **Sign in to your team**
  has a **Sign in with your company** button: your browser opens your company's Google Workspace, Microsoft
  Entra ID or Okta sign-in page, then hands you back to SayQL, signed in. No invite code needed. SayQL acts on
  the link back only while it waits for a sign-in it started, and the server accepts it only with a secret
  this copy of SayQL generated (PKCE), so a link from anywhere else does nothing. These sign-ins last 7 days by
  default, then you sign in the same way again.
- When a team requires sign-in with the company, an invite code says so instead of "isn't valid".

### SayQL AI credits

- **See what's left.** With a credit key, Settings → AI provider shows the balance, and SayQL warns once per
  launch when it drops below $5, so it doesn't run out in the middle of your work. Team server 0.4.0 shows the
  team's balance on the admin page's AI tab, with the same warning. The balance comes from the credits service
  your AI requests already go to; nothing else is sent.

## 0.4.11 (2026-10-29)

### Team connections

- **The status bar's privacy indicator and the connection overview are right on team connections.** They name
  your team's AI as where the Ask box and Investigate send (when your team provides AI), say the admin's limit
  applies, and no longer count AI apps on this computer, which can't use team connections: AI agents reach those
  only through the team's agent gateway. The overview says so in place of the local AI-app counts.
- Team server 0.3.4 says Investigate too: its admin page and audit log no longer say "Agent mode" or
  "Agent (MCP)" but **Investigate (app)** and **AI agent (gateway)**.

## 0.4.10 (2026-10-29)

From a code audit of SayQL. Update if you use SQL Server, or let AI apps read your saved queries.

### Safety

- **Read-only on SQL Server could be bypassed.** SQL Server comments nest, and a `]` inside brackets is written
  `]]`; the read-only check ended both early, so a statement like `SELECT 1 /* /* */ '*/ DELETE FROM x; COMMIT --'`
  passed as read-only, and its `COMMIT` defeated the rollback. The check now reads SQL with the same lexer as
  never-send, which handles both (and Postgres' nested comments, and backticks outside MySQL and SQLite). This
  applied to the editor, Investigate and AI apps on SQL Server connections.
- **Saved queries shown to AI apps hid fewer values than they should.** At "Counts and summaries" and
  "Rows, masked", AI apps see saved queries with their values masked; MySQL `"…"` strings, backslash escapes,
  Postgres `$$…$$` strings and comments slipped through. Every string and comment is masked now.
- **MySQL `/*! … */`**, which MySQL runs as SQL, is read as SQL by never-send too.
- **Your Claude plan** passes the system prompt (with the schema) to Claude Code in a private file, not on the
  command line, where other programs could read it and Windows limits its length. Claude Code runs in a folder of
  your own, not the shared temp folder.

### Under the hood

- One log format for the AI apps' activity and Your activity.
- The first keychain read of a launch no longer holds up other work while macOS asks for permission.
- A linter (oxlint with React's hook rules) and a design-token check run in CI; their findings are fixed.
- Code moved where it belongs: personal-data detectors in their own module, the masker in three files, coding-agent
  process handling out of the audit command, the Get started logic out of the main window.

## 0.4.9 (2026-10-29)

### Agent mode is now Investigate

- **Agent mode is called Investigate**, so it isn't confused with the AI agents (Claude Code, Cursor, …) you let
  into your databases. The Ask box switch is **Quick / Investigate**, the command is `/investigate`, the Settings
  section is **Investigate**, and a connection's menu has **Investigate settings…**. `/agent` still works, and tabs
  saved with it open as before. Nothing about what it does or sends changed.

### See what can reach an AI

- **Privacy indicator in the status bar.** For the connection you're on: **No rows to AI**, **Counts to AI**,
  **Masked rows to AI** or **All rows to AI apps**, the most that Investigate (when on) or AI apps may get. The
  tooltip has a line each for the Ask box, Investigate, AI apps and never-send columns; click it to change them.
- **Connection overview.** Right-click a connection and choose **Overview…**: what it is (tables, columns, notes,
  PROD), what can reach an AI from it, how many requests AI apps made on it and from which apps (with refusals),
  when it was last audited (with **Run audit**), and your recent queries on it.

### Your activity

- **Settings → Your activity** lists the queries you ran: from the editor, the Ask box and Investigate, with the
  connection, the SQL, rows, time and, for an Ask box or Investigate query, the question. For Investigate it also
  says what went to the AI for each query. Filter by connection or by where it came from, open a query in a new tab,
  or clear the log. It stays on this computer, keeps the SQL and counts (never rows) and about the last 2 MB.

### Get started

- **A Get started checklist** under the Database Navigator: choose how to use AI, ask a first question, connect your
  own database, let an AI app in safely, and teach SayQL a database. Each step ticks itself off when it happens, and
  the next one is highlighted. Hide it with ×; it goes away by itself once everything is done.

## 0.4.8 (2026-10-29)

### Choose how to use AI, with pros and cons

- **The first-run setup now asks "How do you want to use AI?"** with four ways, each with what it's good at and
  what it costs you: **Your own API key** (Anthropic, OpenAI, Gemini and more), **Your Claude plan**, **SayQL AI
  credits** and **On this computer** (Ollama, LM Studio). Pick one to set it up; **Other ways** goes back. The AI
  providers docs page has the same comparison.
- **New: Your Claude plan.** Use the Claude Pro or Max plan you already have, with no API key: SayQL starts your
  installed, unmodified Claude Code (`claude -p`) for each request, signed in however you set it up, so requests
  count against your plan's limits. It runs with no tools, no MCP servers, no hooks and no saved session, in an
  empty folder, so Claude only answers in text. It works for the Ask box, Explain, Fix, Improve and agent mode
  (tools travel as text, as with local models). SayQL checks Claude Code's version and sign-in status (Settings →
  AI provider shows "signed in with your Claude Max plan") and never reads your Claude credentials. SQL appears a
  second or two later than with an API key, because Claude Code starts for each request.

### The audit: Run audit, on your own Claude Code sign-in

- **"Audit with Claude Code" is now "Run audit"**, everywhere it appears (Teach SayQL this database, Settings → AI
  apps → Access, the Suggestions tab), and its dialog is **Run an audit**. What it does hasn't changed: it runs your
  installed Claude Code, read-only, in your project folder, and the dialog and tooltip say so.
- **The audit runs on your own Claude Code sign-in; paying for it with SayQL AI credits is off for now.** The
  **Pay with** choice is gone from the dialog. A saved credits choice from 0.4.7 now runs on your own sign-in (or
  however your Claude Code is set up): SayQL gives Claude Code no API key, token or endpoint, and never reads your
  Claude credentials. **Open in Terminal** is always available again. SayQL AI credits still work for the Ask box and
  agent mode.

### Safer around production

- **Nothing connects on its own.** Tabs you had open come back when SayQL starts, but they wait, dimmed, with
  **Not connected** and a **Connect** button (also in the navigator and the status bar). A connection opens when you
  click it, click Connect, run a query or ask. With no tabs open SayQL no longer opens the first connection either.
  AI apps over MCP are unaffected: `sayql-mcp` opens the connections they ask for, as before. Team connections
  follow the same rule.
- **Production tag.** A connection's settings have **This is a production database** (off for existing
  connections). A tagged connection shows a red **PROD** badge in the navigator, on its tabs, in the top bar and in
  the status bar. It's a label only.

### Clear words for what the AI gets back

- **One vocabulary.** Agent mode and AI apps now use the same choices and descriptions: **Nothing (names and row
  counts only)**, **Counts and summaries**, **Rows, masked** (agent mode: up to 20 per query) and **All rows** (AI
  apps only). Each default is marked: Nothing for agent mode, Counts and summaries for AI apps. Stored settings and
  defaults are unchanged.
- **"Turn on agent mode" no longer offers "Off" first.** The choices sit under **What the AI may see of query
  results**, and the dialog also says what AI apps get on the same connection.
- **Never-send columns say where they apply**: agent mode and AI apps over MCP, in the agent settings and in
  Settings → AI apps. The Ask box sends no values from your tables, so it doesn't need them.
- **Accurate AI provider wording.** The AI provider page said the question and names were sent "never the data in
  your tables", which isn't so once agent mode may send results. It now lists what the Ask box sends (question,
  names, notes, the SQL you're working on, a failed query's error with values masked) and says data from your
  tables goes only with agent mode turned on and allowed for a connection. The note shows for local providers too.
  PRIVACY.md, the README and the privacy docs now list notes among what the Ask box sends, and mention the AI
  credits exception where they said SayQL has no server.
- Settings → AI apps → Activity says it covers AI apps only, not agent mode or your own queries.

### Setup and help

- **The setup guide offers SayQL AI credits** as a provider (Claude without an Anthropic account; ask
  hello@azenvoc.com for a credit key), and ends with **Let AI agents in safely**: shortcuts to connect an AI app
  and to teach SayQL your database.
- **Settings → AI apps → Access starts with Connect an app**, open and highlighted until an app has connected,
  one line after that.
- **Keyboard shortcuts:** press **?** outside a text field for the list (also in Settings → About and, on macOS,
  Help → Keyboard Shortcuts). New: **⌘W** closes the tab and **⌘,** opens Settings. On macOS SayQL now has its
  own menu bar so ⌘W no longer closes the window (that's **⇧⌘W**).
- **What's new** shows once after an update, and from Settings → About or Help → What's New.
- Each Settings section links to its page on sayql.dev/docs; About links to the docs.

### Fixes

- Finishing the setup with an example question no longer leaves two identical "Demo store" tabs.
- "Building on your last query" no longer shows right after the first answer; it shows once you type a new
  question.

### Development

- The browser preview's `?firstrun` flag gives a real first run: only the demo connection, AI not set up, no notes,
  suggestions or AI-app activity, and its own empty storage on every load.

## 0.4.7 (2026-10-29)

### Fixes

- **No false "isn't signed in" warning in the audit.** A first refused request is often Claude Code refreshing its sign-in, after which the run goes on; the audit now says something only if the sign-in keeps being refused.

### Audit with Claude Code: pay with SayQL AI credits

- **No Claude plan needed.** The audit dialog has **Pay with: My Claude Code sign-in** (the default) **| SayQL AI credits**, next to Model, remembered. With credits, Claude Code's requests go through SayQL's credits service to Anthropic and are charged to your credit balance (about $0.40 for a typical audit on Sonnet, $0.80 on Opus). SayQL keeps only token counts, never your code or the answers. Without a credit key saved, the option is greyed out and links to Settings → AI provider.
- **The key stays out of sight.** It goes to Claude Code in its environment only (`ANTHROPIC_AUTH_TOKEN`, which Claude Code uses ahead of a claude.ai sign-in), never on the command line or in a file, and is masked in anything the dialog shows. Variables that could point the run elsewhere (another API key, a cloud provider) are removed for the run, nonessential traffic is off, and every model it may ask for is pinned to Sonnet 5.5, Opus 5.5 or (background tasks) Haiku 5.5. Open in Terminal is off while credits are chosen.
- **Clear errors**: a refused credit key (stopped at once instead of after Claude Code's minutes of retries), credit run out or too low for the next step, a model credits don't cover, the service unavailable.
- **Credits service (sayql.dev/api/ai)**: forwards Claude Code's `anthropic-beta` header (needed for the request fields it pairs with), passes `retry-after` and `x-should-retry` back, holds the 5-minute cache-write price unless a request asks for the 1-hour cache, and says when a balance is too low for one request rather than run out. Deploy it before releasing this app version.

## 0.4.6 (2026-10-29)

### Connect your coding agent, whichever it is

- **Pick your coding agent.** **Teach SayQL this database** and **Settings → AI apps → Access → Connect an app**
  now ask for **Your coding agent**: Claude Code, Cursor, VS Code (GitHub Copilot), Codex CLI, Gemini CLI, GitHub
  Copilot CLI, opencode, Windsurf / Devin Desktop, Zed, Cline, Roo Code, Continue, Amp, JetBrains AI Assistant,
  Claude Desktop, or any other MCP app. Each shows its own setup with the path to `sayql-mcp` filled in and quoted
  for your shell: the command to run, **Open in Cursor** or **Open in VS Code** (their own install links, which ask
  before adding SayQL), or the config to copy and the file it goes in.
- **The right way to run the audit in it.** Claude Code keeps **Audit with Claude Code** and `/mcp__sayql__audit`.
  VS Code shows `/mcp.sayql.audit`, opencode `/sayql:audit` and Gemini CLI `/audit`; Cursor and Continue say to
  pick SayQL's audit from the `/` menu. Agents without documented MCP prompts get **Copy audit prompt** to paste
  into their chat. Only Claude Code is started by SayQL, since it's the only one SayQL can keep read-only.
- **A sensible first pick.** SayQL starts with the agent you picked last time, else the installed agent that last
  connected over MCP, else one it finds installed (it checks for the app or command, and runs nothing), else the app
  that last connected, else Claude Code. Step 1 says when the agent you picked is the one connected.

### Fixes from a real "Audit with Claude Code" run

- **Suggested notes can hold codes and file names again.** The check for values from rows refused schema facts
  the audit asks for: `1=Pending, 2=Paid`, `status IN (2,3)`, `divide by 100` and
  `migrations/2026_03_01_merge_orders_legacy.sql`. Small numbers and enum codes, capitalised enum and status
  words, SQL, file paths with dates or digits in them, short commit hashes, dates and units now pass. Still
  refused: emails, phone, card, ID and account (IBAN) numbers, IP addresses, UUIDs and other long ids or keys,
  standalone numbers of six or more digits or with thousands separators, money amounts with a currency symbol,
  and people's names (two capitalised words in prose that aren't enum, schema or product words, or a common first
  name and a surname). The same check now covers notes you write, agent mode's suggestions and `suggest_note`,
  and the refusal says what's fine and what isn't.
- **The audit runs on Sonnet by default.** **Model** in the audit dialog picks Sonnet (the default, cheaper),
  Opus or your Claude Code default, and is remembered. It still uses your own Claude plan.
- **The run panel shows a suggestion once SayQL has taken it.** A refused one shows only as refused, with the
  start of its text, instead of first as suggested.
- **The audit's closing summary is formatted:** headings, lists, bold and code show as such.
- `list_scripts` is denied to the audit run by name, like the other tools it doesn't need, and a test checks
  that every SayQL tool is either allowed or denied.
- The PATH SayQL gives Claude Code lists each folder once.

### Audit: teach SayQL your database

- **The `audit` prompt.** SayQL's MCP server now offers a prompt, `audit` (`/mcp__sayql__audit` in Claude Code,
  with the connection's name or id and optionally a database). Run in your project, it has the coding agent read
  the schema, its usage figures and the saved notes, search your code for where each table and column is used,
  and suggest notes with evidence: retired tables, what status codes and flags mean, relationships that aren't
  foreign keys, units and time zones. It keeps to the 10 to 20 most useful notes and needs no rows. Fetching it
  is blocked and approved like the tools, and logged.
- **Fixing outdated notes.** `describe_schema` now lists saved notes with a short id (`[n_1a2b3c4d]`), and
  `suggest_note` takes `replaces` (the note it corrects) and `remove: true` (to delete it; the text says why).
  Suggestions show **Replaces:** or **Remove note:** with the old text, and **Replace note** or **Remove note**
  applies them. **Dismiss** leaves the note as it is.
- **Last audit.** **Settings → AI apps → Access** shows when an app last fetched the audit prompt for each
  connection, says **Run an audit** when it's never been run or not in 30 days, and has **Copy prompt** for apps
  without MCP prompts.
- **Audit with Claude Code.** One button runs the audit for you: SayQL starts Claude Code in the project folder
  you choose (remembered per connection), read-only, with the audit for that connection, shows its progress as it
  reads and searches, and ends with how many notes it suggested and **Review suggestions**. Read-only is enforced
  with Claude Code's own options: only file read, search and list tools, no shell commands, edits or web, and
  only SayQL's MCP server, given to that run without changing your Claude Code setup. It uses your own Claude Code
  sign-in and subscription (2.1.259 or later); **Cancel** stops it, and **Open in Terminal** runs the same thing in
  your terminal. It's in **Teach SayQL this database**, next to **Copy prompt** in **Settings → AI apps → Access**
  and on the empty **Suggestions** tab. Codex, Gemini and Cursor CLIs aren't offered: SayQL can't keep them
  read-only from the command line yet, so use **Copy audit prompt** with them.
- **Teach SayQL this database.** Until a connection has been audited, the navigator offers the three steps
  (connect an app, run the audit, approve the notes) with the command for that connection; it can be hidden for
  good. The same steps show on the **Suggestions** tab while none are waiting.

## 0.4.5 (2026-10-28)

### Security: AI apps (MCP) and agent mode

An end-to-end test of what AI apps can get through SayQL found ways around its limits. All are fixed:

- **Database errors no longer carry values.** Messages such as `unrecognized configuration parameter "…"` or an
  XML or JSON parse error could quote a value from your rows, even where AI apps may see nothing. Below **All
  rows** (and at All rows for a query on a table with never-send columns), AI apps and agent mode now get a fixed
  description chosen from the database's error code (a syntax error and where, a column that doesn't exist, a
  value that couldn't be converted, …), naming only what is in their own SQL. The activity log keeps only that
  form. When you ask the AI to fix your own query, quoted text in the error is kept only if it is in the query.
- **Never-send columns can't be read in another form.** Whole rows as JSON or text (`row_to_json(c)`, `c::text`,
  `jsonb_each_text`), computed JSON keys, `U&"…"` names, column alias lists, `*` inside a subquery or CTE, and set
  operations are refused on tables with never-send columns. On PostgreSQL, SayQL checks the query's plan
  (`EXPLAIN`, not run) so it also follows views and renames; a never-send column passed straight to the result is
  withheld. `export_csv` and agent mode use the same check. Entries written `table.column` now apply to that
  table only; a bare `column` applies to every table. `describe_schema` marks never-send columns.
- **Counts and summaries:** a renamed personal column (`date_of_birth AS d`) no longer gets a min and max, and
  min/max are now given only for a table's own number and date columns and counts, sums, averages, minimums and
  maximums of them, not for computed expressions (which could carry text, a few bytes at a time).
- **Masked rows:** text whose source SayQL can't trace (column alias lists, `VALUES`, `LATERAL`, `(row).field`,
  `unnest`, JSON access, `UNION`) is masked.
- **Unreadable settings fail closed.** If `agent.json` or `mcp-apps.json` can't be read, every MCP request is
  refused until SayQL fixes it; the MCP server no longer moves the app's files aside (which reset every
  connection's AI settings) or treats a damaged block list as "nothing blocked".
- AI apps and agent mode can no longer call functions that read the server's files or reach outside the
  database (`pg_read_file`, `pg_ls_dir`, large objects, `dblink`, `load_file`, `OPENROWSET`, …), whatever the
  saved login may do.
- `list_scripts` shows as much SQL as the connection allows: all of it at All rows, with string literals masked
  at Counts and Masked, names only at Off.
- `add_connection` refuses a URL or `user:password` in `host` or `user`.
- The activity log also lists schema reads, listings and hand-offs (with the SQL an app opened for you), never
  stores error text with values or suggested note text, and logs suggestions for unknown connections.
- Hand-offs from an app SayQL doesn't recognise show the name it reported, marked *(unverified)*; the display
  title an app sends is ignored. Blocking goes by that reported name and is documented as a convenience, not a
  security boundary.
- Suggested notes that hold a long number, an amount or what looks like a person's name are refused, as the
  refusal message already said.
- The docs now say plainly what the levels do: they keep rows from reaching an AI app by accident, but a count is
  still a fact about your rows; never send is the firm line.

### Settings → AI apps

- **Settings → AI apps, in tabs:** the page formerly called **AI apps (MCP)** is now **AI apps** (for Claude Code,
  Cursor and other MCP apps), with three tabs:
  - **Access:** each app that has connected, allowed or blocked, with when it was last seen; a compact table of
    your connections with what AI apps get back (each level explained once, above the table) and the never-send
    columns as chips, with **Edit** (the same list agent mode uses; editing it here doesn't turn agent mode on or
    off); and **Connect an app**, the setup for Claude Code and Claude Desktop, open until an app has connected.
  - **Activity:** the log at full width, filtered by app, connection or refused queries only. Click a row for its
    full SQL and details. Still the SQL and row counts, never rows; **Clear the log** asks first, as before.
  - **Suggestions:** notes AI apps suggested, with **Accept** and **Dismiss**, and how many are waiting on the tab.
- **Only allow apps I've approved:** a new setting at the top of **Access → Apps**. When it's on, an app you
  haven't approved is refused every tool (and told to ask you to approve it), logged as refused (*not approved*)
  and listed as **Waiting for approval** with **Approve** and **Block**. Turning it on approves the apps already
  listed that aren't blocked, so the ones you use keep working. Blocked still wins over approved. Apps are
  recognised by the name they report, so this keeps out unknown apps but isn't a security boundary against
  software that lies about its name. Off by default; existing settings carry over unchanged.
- **Suggested notes waiting** are counted on **AI apps** in the Settings sidebar and on the settings (gear) button,
  which then opens straight to **AI apps → Suggestions**.
- **Never-send columns are picked, not typed:** a searchable list of your tables and columns, with likely
  personal or secret columns suggested and **Add all suggested**. Picked columns show as chips;
  `table.column` now applies to that table only, and a bare column name to every table.


## 0.4.4 (2026-10-28)

- **Table usage in `describe_schema`:** AI apps using SayQL over MCP now see, after each table, how much it is
  used according to the database's own statistics: reads and writes (PostgreSQL `pg_stat_user_tables`, SQL Server
  index usage, MySQL `performance_schema`), the last write, and the period the counters cover ("since
  2026-01-01", "since the server started on …"), so they can tell a live table from an abandoned one. Counts and
  dates only, never row values. One extra query per describe, with a 5-second limit; if the statistics can't be
  read, one line says so. SQLite keeps no such statistics.
- **AI apps suggest notes, you accept them:** a new MCP tool, `suggest_note`, lets Claude Code, Cursor and other
  apps suggest a database note (a schema-level fact such as "orders_legacy is unused; use orders_v2") with the
  evidence they checked. Nothing is saved until you accept it: suggestions wait under **Notes → Suggested by AI
  apps** in the navigator and in **Settings → AI apps (MCP) → Suggested notes**, with the app's name, the note and
  the evidence, and **Accept** / **Dismiss**. Accepted notes work like your own: the Ask box, agent mode and
  every AI app see them. Suggestions that look like values from the data are refused, and each one is in the
  agent activity log.
- Over MCP, notes now follow the connection's default database when `database` is omitted, as in the app, so
  notes saved in SayQL on a server connection reach `describe_schema`.

## 0.4.3 (2026-10-27)

- **SayQL AI credits:** a new AI provider in **Settings → AI provider** for Claude without an Anthropic account.
  Paste a prepaid credit key from SayQL (it starts with `sqlc_`); SayQL checks it before saving. Requests go
  through SayQL's service to Anthropic and are charged to your credit; SayQL keeps only token counts, never
  your questions or the answers. When the credit runs out, SayQL says so.
- **Choose what AI apps may see:** Claude, Cursor and other apps using SayQL over MCP now get counts and
  summaries of query results by default, not your rows. In **Settings → AI apps (MCP)**, choose per connection:
  nothing, counts and summaries, rows with personal data masked, or all rows. CSV export works only where you
  allow all rows, and below that, values quoted in database errors are masked. **If an AI app relied on seeing
  rows,** set that connection to **All rows**.
- **Agent activity:** **Settings → AI apps (MCP)** now lists what each AI app ran through SayQL: when, which app,
  the connection, the SQL, and whether it ran or was refused and why. SQL and counts only, never rows; kept on
  your computer, with **Clear the log**.
- **Block an AI app:** each app SayQL has seen has a **Block** button; a blocked app is refused everything until
  you allow it again.

## 0.4.2 (2026-10-27)

- **Easier to join your team:** paste the whole invite link into **Settings → Team → Sign in** and it fills in the
  server and the code. Links of the new `https://sayql.dev/join#…` kind work too.
- **A way in from the navigator:** while you're not signed in to a team, the Database Navigator shows
  **Have a team invite? Sign in**. Hide it with its × if you don't need it.
- A team server on its free trial now shows as **Free trial** in **Settings → Team**.

## 0.4.1 (2026-10-27)

The first release with SayQL for teams. (There is no 0.4.0 release: its tag was made too early.)

- **Sign in to your team (SayQL Team, preview):** in **Settings → Team**, sign in to your company's SayQL server
  with an invite code, or open a `sayql://team-join` invite link, which fills in the dialog for you to confirm.
  The team's connections appear in the navigator under **Team**: read-only, with no host or password on your
  computer, and every query goes through the team server, which records it (and the question it answers) in its
  audit log. The team admin's rules win there: agent mode can't see more than they allow, and their never-send
  columns can't be removed. Without signing in, SayQL never contacts a team server.
- **Team AI:** when your team admin sets up a company AI key on the team server, the Ask box, fixes,
  Improve and agent mode on team connections use it through the server (streaming as usual), with the team's
  models shown as set by your admin. Settings → Team shows this month's usage against the limits; at a limit,
  SayQL says when it resets. Your own connections keep your own AI provider and key.

## 0.3.0 (2026-10-23)

- **Agent mode:** type `/agent` and a question that needs looking around first ("who owns this tenant?", "why did
  signups drop last week?"). The agent shows a plan you can change, works through it in read-only steps, asks
  you when a question has two meanings, and answers with every sentence linked to the queries behind it.
  Numbers it can't find in any result are marked "not verified", and joins that would double count are flagged.
  It never runs writes: a change comes back as SQL marked "not run". Stop ends the run and its query.
- **Off until you turn it on, per connection.** You choose what the AI may see of results there: nothing, counts
  and summaries, or up to 20 rows per query with likely personal data masked (and filled back in on your
  screen). Ask before each send shows exactly what would go, with Send and Skip; never-send columns are refused.
- **Database notes:** facts the agent learns ("owner = memberships.role = 'owner'") can be saved per database.
  The Ask box, the agent and MCP all use them; edit them in the navigator.
- **Quick or Agent, and the model, right in the Ask box.** A switch under the Ask box (or ⌘⇧A) picks
  Quick (SQL as you type) or Agent, so nobody has to remember `/agent`; turning on Agent for a connection the
  first time asks what it may see. Next to it, the model name switches the model, or the provider among the ones
  already set up, without opening Settings. In agent mode, an empty Ask box suggests a few questions to start with.
- **Auto mode for the agent:** switch **Ask first** to **Auto** next to the model name (or press "Go, and don't
  ask again") and runs no longer stop for the plan or before each send. Questions with two meanings still ask,
  and what the AI may see stays as the connection's setting.
- **Saved scripts follow the open database,** with **All** one click away.
- **Right-click a tab** to close it, the others, those to the right or left, the saved ones or all, or duplicate it.
  Closing several with unsaved SQL asks once, and can keep the unsaved ones open.
- **MCP and agent mode work together:** MCP apps see your database notes, and never-send columns are refused
  over MCP too (their values are withheld from `SELECT *`).
- An agent step opened after SayQL restarts shows its rows again: rows aren't kept with the tab, so the step's
  read-only query runs again (and says so) instead of showing 0 rows.
- SQL an AI app opens in SayQL over MCP is no longer fixed automatically by SayQL's AI when it fails, so it
  never spends your AI credits unasked. **Fix with AI** on the error still does it when you choose.
- MCP's read-only helpers moved to a module agent mode shares, so both keep the same safety rules.

## 0.2.9 (2026-10-23)

- **Supabase and other connection poolers work:** PostgreSQL behind a transaction pooler (Supabase on port 6543,
  PgBouncer) failed with "prepared statement already exists". Every query now runs in its own transaction on one
  server connection, and settings a confirmed statement changes don't leak to the next person on that connection.
  A wrong password behind a pooler now says so.
- **Several questions:** when one result answers everything naturally, you get one query; when the answers have
  different shapes (a count and a list, unrelated tables), you get one statement each, with its own results tab.
- **"All" means all:** "show all users" no longer adds `LIMIT 100`; the app's own 5,000-row cap still applies.
- **No chatter in the SQL:** the AI writes at most a one-line assumption above a statement, never its reasoning.

## 0.2.8 (2026-10-23)

- **Paste a connection string** into the New connection dialog to fill in every field, including the password.
  Reads URLs, JDBC and key=value strings for PostgreSQL, MySQL, SQL Server/Azure SQL and SQLite. The string isn't
  kept; the password goes to the keychain only when you save.

## 0.2.7 (2026-10-22)
- **MCP: opening a saved script launches SayQL and opens it.** Before, if SayQL was closed, it opened but said
  the script "isn't saved here".
- **SQL Server `geography`, `geometry`, `hierarchyid` and `xml` columns** are described without a length (the AI
  saw `geography(max)`, which isn't valid SQL Server).
- **MCP sees schema changes:** `describe_schema` reads the schema each time, so tables and columns added
  while your AI app is open show up without restarting it.
- **MySQL 5.7 connects:** with "Require SSL" off, SayQL now falls back to an unencrypted connection when
  the server's encryption can't be used, as MySQL's own clients and psql do. Before, MySQL 5.7 servers (and
  others offering only older TLS ciphers) couldn't be reached at all.
- **Clearer connection errors:** a wrong port or a stopped server now says "Could not reach host:port:
  Connection refused" right away, instead of "pool timed out" after 10 seconds. SSL problems say so, and how
  to connect without SSL.
- **PostgreSQL schema:** logins that can only `SELECT` (the setup we recommend) now see primary keys, which
  were missing for them. Materialized views are listed. Partitioned tables are listed once, with a row
  estimate that covers all their partitions, instead of once per partition.
- **AI and names that need quotes:** the schema SayQL sends to the AI (and MCP's `describe_schema`) now
  writes table and column names the way your database needs them: `"FirstName"` in PostgreSQL,
  `` `first name` `` in MySQL, `[Order Lines]` in SQL Server. Before, the AI copied such names unquoted and the
  query failed.
- **Readable database errors:** PostgreSQL errors no longer end in "at line 414" (a line in Postgres' own
  source code, not your query) and now include the DETAIL and HINT lines, e.g. which key already exists.
  SQL Server errors show the message, then the error number and the line of your query, instead of
  `Token error: '…' on server … executing …`.
- **Fixing SQL Server queries with AI works again:** the wrapped SQL Server message looked like one quoted
  value, so masking values before sending the error to the AI hid the whole message, column names
  included. Only the values are masked now, on every database.
- **Read-only mode covers effects a rollback keeps:** PostgreSQL functions that a read-only transaction
  allows but that outlast it now ask for confirmation, like any other write: creating or dropping
  replication slots, resetting statistics, and session advisory locks. So does MySQL's `GET_LOCK`.
- **`SHOW CREATE TABLE` runs in read-only mode** on MySQL and MariaDB; it no longer asks for confirmation.
- **Confirmed statements don't leave settings behind:** a `SET`, `USE`, lock or temporary table from a
  statement you confirmed no longer sticks to one of SayQL's pooled connections, where it changed the
  results of later queries at random (one query in four would run in the other database after a `USE`).
  Each confirmed statement now runs on a connection of its own.
- **SQL Server shows the rows a confirmed statement returns,** such as `INSERT ... OUTPUT inserted.*` or
  `DECLARE ...; SELECT ...`. Before, only "0 rows affected" was shown.
- **MySQL zero dates show as stored:** `0000-00-00` and dates with a zero month or day (`2024-00-15`) used
  to show as NULL. They now show as MySQL writes them.
- **MySQL geometry shows as WKT:** `POINT(1.5 -2)`, `POLYGON(...)` and the rest, instead of `<geometry>`.
- **SQL Server columns without a name** show as "(No column name)", as in SQL Server's own tools.
- **Cancel a running query:** a **Cancel** button next to **Run** stops a slow query on the database
  server itself (PostgreSQL, MySQL, MariaDB, SQL Server, SQLite), not just in SayQL. Queries from AI apps
  over MCP stop on their own after 2 minutes (exports after 10).
- **Scripts split the way your database reads them:** a `;` inside a MySQL `#` comment or `'it\'s'` string,
  or a Postgres `$tag1$` string, no longer splits a statement in two.
- **SQL Server `geometry` shows as WKT,** as `STAsText()` writes it, instead of bytes.
- **SQLite scripts over MCP** return the first result set, instead of every statement's rows under the
  first one's columns.
- **MySQL 5.7 with "Require SSL" connects.** PostgreSQL and MySQL connections now use your system's TLS
  (rustls couldn't negotiate with MySQL 5.7).
- **MariaDB `ed25519` accounts sign in,** and MariaDB JSON columns show as JSON instead of text.
- **SQL Server `money` is exact** (shown as `12.3400`, as in SSMS) instead of a float that lost digits above
  about 900 billion. **`geography` and `hierarchyid` show as text** (`POINT (-122.3 47.6)`, `/1/2/`), as SQL
  Server writes them.

## 0.2.6 (2026-10-22)
- **Deleting a connection** also removes its password from the keychain item older versions saved it in.
- **API keys:** release builds no longer fall back to the `ANTHROPIC_API_KEY` environment variable; save the
  key in Settings.
- **Deleting a connection shows why when it fails** (e.g. keychain access denied); the connection and its tabs stay.

## 0.2.5 (2026-10-22)

- **Voice input:** click the mic in the Ask box, or press ⌘⇧M (Ctrl+Shift+M), and say your question. It
  becomes the question as if you'd typed it, keeping any `/command`; press ↵ to run. Speech is turned into
  text on your computer by Whisper, and audio is never saved or sent anywhere. The first time, the speech
  model (82 MB) downloads once from Hugging Face; remove or redownload it in Settings → Voice.
- **Safer saved connections:** if a saved file (connections, settings, scripts or skills) is damaged,
  SayQL no longer treats it as empty and erases it on the next save. It moves the file aside as
  `<name>.corrupt-<time>` and says where. Saves are now written in one step, so a crash mid-save can't
  leave a half-written file.
- **MCP: hand work to SayQL.** Two new tools for AI apps. `add_connection` opens the New connection dialog
  filled in; you type the password in SayQL. `open_query` opens SQL in a new tab for you to review and run.
  Passwords and writes never go through MCP, and nothing runs or saves on its own.
- **MCP: saved scripts.** `list_scripts` lists them with their SQL; `open_script` opens one in SayQL, unrun.
- **MCP: more rows.** `run_query` pages with `offset`, and `export_csv` saves up to 100,000 rows of a
  read-only query to a new CSV in Downloads without sending them to the AI app.
- **Privacy:** when SayQL fixes a failed query, values from your rows that the database quotes in its error
  (a duplicate email, a value that failed to convert) are masked before the error goes to your AI provider.
- **Stricter read-only check:** SayQL now reads each statement the way your database does (MySQL `#` comments,
  `/*! */` and backslash escapes; Postgres `E'...'` and `$$` strings), so a write can't hide inside them.
  `INTO OUTFILE`, `INTO DUMPFILE`, `NEXT VALUE FOR` and side-effect functions such as `nextval`, `dblink` and
  `pg_terminate_backend` now ask before running, and MCP refuses them.
- **Content Security Policy:** the app window now loads scripts only from SayQL itself and talks only to SayQL's
  own backend, so injected content can't load remote code or send data elsewhere.

## 0.2.5 (2026-10-22)
- **Security:** updated the SQL Server driver, which fixes a high-severity issue in a TLS library it used.
- **Under the hood:** newer database drivers (sqlx 0.9, tiberius 0.13), editor (Monaco 0.57) and icons.
- **Editor:** suggests your tables and columns as you type, plus find and replace (⌘F, ⌥⌘F) and comment
  toggling (⌘/).

## 0.2.4 (2026-10-22)

- **MCP server:** it has come with SayQL since 0.1.0 (the docs and website said "coming soon" by mistake).
  Set it up in Settings → AI apps (MCP). On Linux, Settings now warns that the AppImage's path changes on
  every launch; use the `.deb` or `.rpm` for MCP.

0.2.3 was not released: its build failed.

## 0.2.2 (2026-10-22)

- **One-line Ask box:** the tables you picked sit at the right end of the question, and the box stays one line
  until you type more.

## 0.2.1 (2026-10-22)

- **New logo and app icon:** "Say" with "QL" in a blue chip.

## 0.2.0 (2026-10-22)

- **Slash commands** in the Ask box: type `/` for `/followup`, `/new`, `/explain`, `/fix` and `/improve`.
- **Skills:** save an instruction once, like `/pii-safe` = "Never select email, phone or address columns", and add it
  to any question. Text only, stored on your computer, managed in Settings.
- **Settings is a full-window page** with a sidebar: AI provider, Skills, AI apps (MCP) and About.
- **Run scripts:** several statements run one after another, each with its own results tab. Writes still ask
  first, and the run stops at the first error.
- **Run selection:** select part of the SQL and press Run or ⌘↵ to run only that. A selection that cuts a
  statement in half runs the whole statement instead, so a `DELETE` never runs without its `WHERE`.
- **Setup guide** on first launch: pick your AI (the key is checked before saving), pick the demo or your own
  database, and run a first question. Reopen it from Settings.
- **Claude API keys are checked** in the setup guide before they're saved, so a typo shows up straight away.
- **Send feedback** from the status bar: a pre-filled public GitHub issue. Only the SayQL
  version and operating system are added.
- **Open source** under the Apache License 2.0, with contributor docs, a security policy and CI on pull requests.
- **Docs** at [sayql.dev/docs](https://sayql.dev/docs/): install, AI providers, databases, MCP,
  shortcuts and troubleshooting.

## 0.1.0 (2026-10-21)

The first release of SayQL Community Edition.

- **Ask in plain English:** SQL streams into the editor as you type. Simple requests are answered instantly on your
  machine, follow-ups edit the query you have, failed queries are fixed once automatically, and **Improve**
  reviews a query on request.
- **Any AI provider:** Claude, OpenAI, Google Gemini, OpenRouter, Mistral, Groq, DeepSeek, Ollama, LM Studio or any
  OpenAI-compatible API, with a fast model for drafts and a stronger one for fixes.
- **Databases:** PostgreSQL, MySQL / MariaDB, SQLite, SQL Server and Azure SQL, with every database on a server
  listed in the navigator and a built-in demo store.
- **Read-only by default:** statements are checked, reads run read-only in the database, and anything that could
  change data asks first.
- **Workbench:** tabs, saved scripts, and picking tables as AI context.
- **Secrets in the OS keychain**, in a single item, so macOS asks at most once after an update.
- **Automatic, signed updates** from the status bar.
