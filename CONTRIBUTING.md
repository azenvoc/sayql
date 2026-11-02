# Contributing to SayQL

Thanks for helping make SayQL better. Bug reports, ideas, docs fixes and code are all welcome.

## Before you start

- **Bugs and ideas:** open an [issue](https://github.com/dithzz/sayql/issues/new/choose). For questions and
  general feedback, use [Discussions](https://github.com/dithzz/sayql/discussions).
- **Security problems:** don't open a public issue. See [SECURITY.md](SECURITY.md).
- **Larger changes:** open an issue first so we can agree on the approach before you spend time on it.

## Set up

You need:

- [Rust](https://rustup.rs/) (stable)
- [Node.js](https://nodejs.org/) LTS and [pnpm](https://pnpm.io/) 10
- The Tauri system dependencies for your platform
  ([guide](https://v2.tauri.app/start/prerequisites/)):
  - **macOS:** Xcode Command Line Tools (`xcode-select --install`)
  - **Windows:** Microsoft C++ Build Tools and WebView2 (preinstalled on Windows 11)
  - **Linux (Debian/Ubuntu):** `libwebkit2gtk-4.1-dev libappindicator3-dev librsvg2-dev patchelf libssl-dev libdbus-1-dev`

Then:

```bash
pnpm install
pnpm tauri dev      # run the desktop app
pnpm test           # frontend and backend tests
```

To work on the UI only, run `pnpm dev` and open http://localhost:1420. A mock backend serves demo data,
so no database or AI key is needed.

[docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) explains the folder structure and code conventions.

### Integration tests against PostgreSQL, MySQL and SQL Server

`pnpm test` covers SQLite only. The PostgreSQL, MySQL and SQL Server tests in
`src-tauri/src/db/integration_tests.rs` are skipped unless you point them at a server. Pick a throwaway
password for local test containers (SQL Server needs 8+ characters with upper and lower case, a digit and a
symbol) and keep it out of the repository:

```bash
export SAYQL_TEST_DB_PASSWORD='<throwaway password>'
docker run -d --name sayql-test-pg -e POSTGRES_PASSWORD="$SAYQL_TEST_DB_PASSWORD" -e POSTGRES_DB=sayql -p 55416:5432 postgres:16
docker run -d --name sayql-test-mysql -e MYSQL_ROOT_PASSWORD="$SAYQL_TEST_DB_PASSWORD" -p 33806:3306 mysql:8
docker run -d --platform linux/amd64 --name sayql-test-mssql -e ACCEPT_EULA=Y -e MSSQL_SA_PASSWORD="$SAYQL_TEST_DB_PASSWORD" -p 14334:1433 mcr.microsoft.com/mssql/server:2022-latest
cd src-tauri
SAYQL_TEST_PG_URL="postgres://postgres:$SAYQL_TEST_DB_PASSWORD@localhost:55416/sayql" \
SAYQL_TEST_MYSQL_URL="mysql://root:$SAYQL_TEST_DB_PASSWORD@localhost:33806" \
SAYQL_TEST_MSSQL_URL="mssql://sa:$SAYQL_TEST_DB_PASSWORD@localhost:14334/master" \
cargo test --lib integration
```

The URLs take the form `scheme://user:password@host:port/database` (no percent-encoding). `SAYQL_TEST_MYSQL_URL`
also takes a MariaDB server. The tests create and drop their own schemas, databases and roles (all named
`sayql_*`), so use throwaway servers. With the same variables, `cargo test --lib tenant_fixture ai::agent` loads
the synthetic tenant database (`src-tauri/src/db/tenant_fixture.rs`, made-up rows only) into a `sayql_tenant`
database on each server and runs the masking and agent evaluation checks against it.

The agent evaluation can also run the 45-question set with a real model. It costs API credits, so it only runs
when you set a key, and writes a record per question (steps, tokens, time, everything sent) to
`src-tauri/target/agent-eval.json`:

```bash
SAYQL_EVAL_ANTHROPIC_KEY=sk-ant-... cargo test --lib agent_answers_the_question_set -- --nocapture
```

For a local model, set `SAYQL_EVAL_OPENAI_URL` (e.g. `http://localhost:11434/v1`) and `SAYQL_EVAL_MODEL` instead. MySQL needs a few seconds after `docker run` before it accepts connections.

To run them against every version SayQL supports (PostgreSQL 13 to 17, MySQL 5.7 to 8.4, MariaDB 10.6 to
11.4, SQL Server 2019 and 2022), each in its own throwaway container that is removed afterwards:

```bash
scripts/test-databases.sh            # all of them, or name some: scripts/test-databases.sh pg17 mariadb114
```

CI runs them on every pull request against three combinations (the `integration` job in
`.github/workflows/ci.yml`). [docs/DATABASE_COMPATIBILITY.md](docs/DATABASE_COMPATIBILITY.md) has the full
matrix and what each check covers.

### The MCP server in development

`cargo build --bin sayql-mcp` builds it to `src-tauri/target/debug/sayql-mcp`. Point an MCP client at that path
(for Claude Code: `claude mcp add --scope user sayql -- /path/to/sayql-mcp`).

## Making a change

1. Fork the repository and create a branch from `main`.
2. Keep the change focused: one fix or feature per pull request.
3. Follow [AGENTS.md](AGENTS.md): the code rules for everyone, people and AI agents alike. CI enforces
   every one of them (formatting, lints, the lint ratchet, architecture checks), and a pull request
   that fails one can't be merged. `pnpm hooks` turns on a pre-commit hook with the fast checks.
4. Add or update tests, and run `pnpm check` before you push. It runs what CI runs, except the
   database integration tests.
5. Open a pull request describing what changed and why.

## Ground rules for SayQL features

- **Never weaken read-only safety.** Anything that could change data must keep going through the write
  confirmation, and the MCP server must stay read-only.
- **Never send table rows to an AI provider** from the Ask box. Only questions, schema and SQL.
- **Secrets stay in the OS keychain**, never in files or logs.

## Docs and website

The website is `site/`. Docs pages are in `docs-site/pages/` and are built with
`python3 scripts/build_docs.py`.

## License

By contributing, you agree that your contributions are licensed under the [Apache License 2.0](LICENSE),
the same license as the project.

Everyone taking part is expected to follow the [Code of Conduct](CODE_OF_CONDUCT.md).
