# Database compatibility

What SayQL was tested against, what each check covers, what was fixed, and what isn't supported. The
tests behind it are in `src-tauri/src/db/integration_tests.rs` and `src-tauri/src/mcp/tests.rs`.
`scripts/test-databases.sh` reruns them against every version below, and CI runs three combinations on every
pull request (the `integration` job). Last full run: 2026-10-22, against real servers in Docker.

**Legend:** ✅ passes · 🔧 was broken, fixed in this audit · ⚠️ known limitation (see below) · — not applicable

## The matrix

| Check | PG 13 | PG 15 | PG 16 | PG 17 | MySQL 5.7 | MySQL 8.0 | MySQL 8.4 | MariaDB 10.6 | MariaDB 10.11 | MariaDB 11.4 | SQL Server 2019 | SQL Server 2022 | SQLite |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| Connect, password auth (MariaDB: incl. ed25519) | ✅ md5 | ✅ md5 | ✅ scram | ✅ scram | 🔧 | ✅ | ✅ | 🔧 | 🔧 | 🔧 | ✅ | ✅ | — |
| SSL off / required | ✅ ¹ | ✅ ¹ | ✅ | ✅ | 🔧 | ✅ | ✅ | ✅ ¹ | ✅ ¹ | ✅ | ✅ | ✅ | — |
| Clear errors: wrong password, host, port, database | 🔧 | 🔧 | 🔧 | 🔧 | 🔧 | 🔧 | 🔧 | 🔧 | 🔧 | 🔧 | ✅ | ✅ | ✅ |
| List databases, default database | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | — |
| Schema: schemas, views, keys, enums, odd names | 🔧 | 🔧 | 🔧 | 🔧 | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| Schema as a read-only login | 🔧 | 🔧 | 🔧 | 🔧 | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | — |
| Materialized views, partitioned tables | 🔧 | 🔧 | 🔧 | 🔧 | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | — |
| 500-table schema, row estimates | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| Data types in results | ✅ | ✅ | ✅ | ✅ | 🔧 | 🔧 | 🔧 | 🔧 | 🔧 | 🔧 | 🔧 | 🔧 | ✅ |
| CTEs, window functions, scripts, 5,000-row cap | ✅ | ✅ | ✅ | ✅ | 🔧 ² | 🔧 | 🔧 | 🔧 | 🔧 | 🔧 | ✅ | ✅ | 🔧 |
| Cancel a running query (stops on the server) | 🔧 | 🔧 | 🔧 | 🔧 | 🔧 | 🔧 | 🔧 | 🔧 | 🔧 | 🔧 | 🔧 | 🔧 | 🔧 |
| Read-only: writes refused (app and MCP) | 🔧 | 🔧 | 🔧 | 🔧 | 🔧 | 🔧 | 🔧 | 🔧 | 🔧 | 🔧 | ✅ | ✅ | ✅ |
| Read-only: the database itself refuses | ✅ | ✅ | ✅ | ✅ | ⚠️ | ⚠️ | ⚠️ | ⚠️ | ⚠️ | ⚠️ | ⚠️ | ⚠️ | ✅ |
| Confirmed write commits, leaves no session state | 🔧 | 🔧 | 🔧 | 🔧 | 🔧 | 🔧 | 🔧 | 🔧 | 🔧 | 🔧 | 🔧 | 🔧 | ✅ |
| Schema text for the AI (quoting) | 🔧 | 🔧 | 🔧 | 🔧 | 🔧 | 🔧 | 🔧 | 🔧 | 🔧 | 🔧 | 🔧 | 🔧 | 🔧 |
| MCP tools, paging, `export_csv` | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | 🔧 |
| Error values masked before the AI | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | 🔧 | 🔧 | 🔧 | 🔧 | 🔧 | ✅ |

¹ The Docker images of these versions don't enable TLS, so "Require SSL" correctly fails, with a message
saying to turn it off. PostgreSQL 16 and 17 were run with TLS on.
² MySQL 5.7 has no `WITH` or window functions; SayQL passes the server's error through. 🔧 here: scripts
are split the MySQL way (`#` comments, `\'` escapes), and on SQLite a multi-statement call shows only the
first result set.

**Not tested:** Azure SQL Edge (see below), Azure SQL itself (needs a cloud account; SayQL follows its
gateway redirects), PostGIS types (the plain `postgres` images don't include it), Windows/Entra sign-in.

## What each check covers

- **Connect:** PostgreSQL `md5` (13, 15) and `scram-sha-256` (16, 17). MySQL `caching_sha2_password` and
  `mysql_native_password` (8.4 started with `--mysql-native-password=ON`). SSL off and required. Wrong
  password, host, port and database. A user that can only `SELECT`.
- **Schema:** two schemas with a foreign key between them, views, materialized views, composite primary keys,
  enums (including same-named enums in different schemas), and mixed-case, accented, space-containing and
  reserved-word names. Also partitioned tables, and 500 tables of 11 columns. Reading the 500 took 1.2 s on
  PostgreSQL, 0.2 s on MySQL and 0.4 s on SQL Server (emulated).
- **Types:**
  - json/jsonb, uuid, arrays (nested, with NULLs), numeric at full precision, money;
  - date, time, timestamp with and without time zone, interval, infinity;
  - bytea/blob/varbinary, bit, boolean, point and geometry;
  - MySQL zero dates, SQL Server datetimeoffset, uniqueidentifier, nvarchar, xml and sql_variant;
  - NULLs, 200,000-character text, emoji.
- **Read-only:** about 50 statements per dialect, run two ways: through the app's read-only path, and
  straight inside the read-only transaction with SayQL's check skipped, to see what the database stops on
  its own. They include:
  - INSERT, UPDATE, DELETE, MERGE, DDL, TRUNCATE;
  - `SELECT … INTO`, `INTO OUTFILE`, COPY, CALL/EXEC of writing procedures;
  - writing functions, `nextval`, LOCK, temp tables, SET, `EXPLAIN ANALYZE` of a write;
  - `COMMIT` inside a script, dynamic SQL;
  - writes hidden in each dialect's comments and strings.

  Every write was refused or sent to confirmation, and data never changed. MCP `run_query` and `export_csv`
  refuse them outright.

## Fixed in this audit

1. **MySQL 5.7 couldn't connect at all.** With SSL off, the TLS handshake failed (5.7 offers no cipher suite
   rustls accepts) and SayQL stopped. It now falls back to an unencrypted connection, as MySQL's own
   `PREFERRED` mode does.
2. **"Pool timed out" instead of the real reason** when a port was closed or a host unreachable, after 10
   seconds. It now says `Could not reach host:port: Connection refused` at once.
3. **PostgreSQL read-only logins saw no primary keys.** `information_schema` hides constraints from roles with
   only SELECT. Materialized views were missing too. Partitions were listed one by one, and on PostgreSQL 13
   the partitioned table claimed ~0 rows.
4. **The AI got names it couldn't use.** `FirstName` (PostgreSQL), `first name` and `order` were written
   unquoted in the schema text, so generated queries failed. They're now quoted per dialect.
5. **"Fix with AI" was blind on SQL Server.** The masking that hides values before an error goes to the AI
   hid the whole SQL Server message. PostgreSQL errors ended in "at line 414", a line in Postgres' source
   code, and lacked DETAIL and HINT.
6. **Lasting effects ran in read-only mode.** These passed the check, and a read-only transaction allows them:
   - creating and dropping PostgreSQL replication slots;
   - `pg_stat_reset`;
   - session advisory locks and MySQL `GET_LOCK`.

   They now need confirmation. `SHOW CREATE TABLE` no longer does.
7. **Confirmed statements leaked session state.** After a confirmed `USE`, about one query in four ran in the
   other database, and `SET`, locks and session settings stayed on one pooled connection. Each confirmed
   statement now gets its own connection.
8. **SQL Server showed no rows for confirmed statements** like `INSERT … OUTPUT` or `DECLARE …; SELECT`.
9. **MySQL zero dates showed as NULL**, geometry as `<geometry>`, and SQL Server's unnamed columns had
   blank headers. SQL Server `geometry` showed as bytes; it now reads as `STAsText()` writes it.
10. **A running query couldn't be stopped.** It now can, from a **Cancel** button next to **Run**.
    SayQL asks the server to stop it:
    - `pg_cancel_backend` on PostgreSQL;
    - `KILL QUERY` on MySQL and MariaDB;
    - a protocol attention on SQL Server;
    - an interrupt on SQLite.

    Tests check the statement is gone from the server, not just abandoned. MCP queries stop on their own
    after 2 minutes, and exports after 10, since an AI app can't cancel them.
11. **The editor split MySQL scripts wrongly** at a `;` inside a `#` comment or a `'it\'s'` string, and the
    Postgres `$tag1$` quote. Scripts are now split the way the connection's database reads them.
12. **The browser preview's copy of the read-only check had drifted** from the app's. It's now a port of
    the same rules, with the same test cases.
13. **On SQLite, a call with several statements returned all their rows under the first one's columns.**
    It now returns the first result set, as the other databases do.
14. **MySQL 5.7 with "Require SSL" couldn't connect.** 5.7 only negotiates DHE or RSA key exchange, which
    rustls doesn't implement. SayQL's PostgreSQL and MySQL connections now use the platform's TLS:
    Security.framework on macOS, SChannel on Windows, OpenSSL on Linux. The tests require an encrypted
    session whenever the server offers TLS.
15. **MariaDB `ed25519` accounts couldn't sign in.** sqlx has no `client_ed25519` plugin; SayQL now uses a
    patched copy of sqlx-mysql that adds it (see `src-tauri/vendor/README.md`).
16. **MariaDB JSON showed as text.** MariaDB stores JSON as LONGTEXT and marks it only in its extended type
    info, which the patched sqlx-mysql now asks for. JSON columns and `JSON_OBJECT()` results read as JSON,
    and plain text columns stay text.
17. **SQL Server `money` lost digits above about 900 billion**, because tiberius decoded it as a float. A
    patched tiberius decodes it as an exact 4-place decimal, as SSMS shows it (`12.3400`).
18. **SQL Server `geography` and `hierarchyid` showed as bytes.** The driver only says "UDT", so SayQL now
    asks SQL Server what each column is (`sp_describe_first_result_set`, which doesn't run the query). It
    then has the server write the values as `STAsText()` or `ToString()` do. That's exact for every shape,
    with geography's longitude first, as SQL Server writes it. If the server can't describe a statement,
    `geometry` with SRID 0 is still decoded locally and the rest shows as bytes.

## Known limitations

- **No automatic timeout in the app.** Queries run until they finish or you click **Cancel**. To cap them
  for a shared login, set a server-side limit: `statement_timeout` (PostgreSQL), `max_execution_time`
  (MySQL) or the Resource Governor (SQL Server).
- **MySQL and MariaDB don't stop DDL in a read-only transaction.** DDL commits the transaction and runs.
  SayQL's own check refuses every DDL statement in read-only mode, and prepared statements prevent smuggling
  a second statement in. But the database isn't a second line of defense here, so use a login with only
  `SELECT` (below).
- **SQL Server has no read-only transaction.** Reads run in a transaction that is always rolled back, and
  SayQL refuses anything that could write. A procedure that commits defeats the rollback, so `EXEC` always
  needs confirmation. Use a `db_datareader` login.
- **Privileged logins can read server files** with `pg_read_file` or `LOAD_FILE`. These are reads, so
  read-only mode allows them. Don't connect as a superuser.
- **Azure SQL Edge** crashes on startup under Docker on Apple silicon, and Microsoft has retired it, so it
  wasn't used as an Azure SQL stand-in. Azure SQL behaves like SQL Server 2022 for everything SayQL does,
  plus the gateway redirect SayQL follows.

## Setup: a read-only login for SayQL

SayQL's read-only mode is a safety net. A login that can only read makes the database enforce it too. All
the checks above passed with logins like these.

**PostgreSQL**
```sql
CREATE ROLE sayql_reader LOGIN PASSWORD '…';
GRANT CONNECT ON DATABASE app TO sayql_reader;
GRANT USAGE ON SCHEMA public TO sayql_reader;
GRANT SELECT ON ALL TABLES IN SCHEMA public TO sayql_reader;
ALTER DEFAULT PRIVILEGES IN SCHEMA public GRANT SELECT ON TABLES TO sayql_reader;
ALTER ROLE sayql_reader SET statement_timeout = '60s';
```

**MySQL / MariaDB**
```sql
CREATE USER 'sayql_reader'@'%' IDENTIFIED BY '…';
GRANT SELECT, SHOW VIEW ON app.* TO 'sayql_reader'@'%';
```

**SQL Server / Azure SQL** (in the database)
```sql
CREATE LOGIN sayql_reader WITH PASSWORD = '…';   -- on Azure SQL: CREATE USER sayql_reader WITH PASSWORD = '…'
CREATE USER sayql_reader FOR LOGIN sayql_reader;
ALTER ROLE db_datareader ADD MEMBER sayql_reader;
```

**SQLite:** SayQL already opens the file read-only for reads. To be sure, make the file read-only in your
file system.
