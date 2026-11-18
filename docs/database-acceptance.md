# Database Manager MVP acceptance verification

This document records the Database Manager MVP acceptance run for issue #191.
It follows section 14 of the Database Manager plan and the project's existing
MVP acceptance method in [acceptance.md](acceptance.md).

## Candidate and environment

- Date: 2026-11-18
- Candidate: `feat/db-191-acceptance`, based on `dev` after #190
- Application version: `0.1.0`
- Host: Fedora Linux 44 x86_64, kernel `7.3.3-200.fc44.x86_64`
- Desktop: X11 / GNOME session, WebKitGTK `2.54.2`
- Toolchain: Node.js `24.21.0`, Rust `1.99.1`, Tauri `2.12.5`
- Automation: `tauri-driver` with `WebKitWebDriver`, driving the packaged
  application binary and the real Tauri command surface
- SSH fixture: disposable Ubuntu 22.04 container running OpenSSH `8.9p1` on
  `127.0.0.1:2222`
- MySQL fixture: disposable MySQL Community Server `8.4.13` container on
  `127.0.0.1:3307`
- PostgreSQL fixture: disposable PostgreSQL `18.6` container on
  `127.0.0.1:5433`
- Tunnel path: every database profile named the host gateway
  (`169.254.1.2`) reachable from the SSH fixture. The desktop connected only to
  the loopback listener opened by ServerDesk; no database port was reachable
  directly from the desktop during the run.

Only the Linux host was available. Windows and macOS were not run.

### Artifacts

| Artifact                        | Size         | SHA-256                                                            |
| ------------------------------- | ------------ | ------------------------------------------------------------------ |
| `ServerDesk_0.1.0_amd64.deb`    | 9.77 MiB     | `06523ae4e347366c54287936b6e6413714e433250d0de4cf8ad367ae7ae80ef4` |
| `ServerDesk-0.1.0-1.x86_64.rpm` | 9.78 MiB     | `760cbeb7b2c8329e4e5360dffeb2b09a024f26c0dd20ce4530fb083f30bce7e5` |
| Packaged `serverdesk` binary    | 21,697,928 B | `8a9dcbe392e63aba7d5bc1584e4ca752cab5f15f330c06feac9e0d45bd72db5c` |

The RPM was installed in a disposable Fedora 44 container as `server-desk
0.1.0-1.x86_64`. `serverdesk --version` printed `ServerDesk 0.1.0`, and a
`SERVERDESK_SMOKE_TEST=1` launch under Xvfb created a clean window, migrated
the SQLite schema to version 6, and exited with code 0. The SQLite file was
mode `0600`. The installed `/usr/bin/serverdesk` has the same SHA-256
(`8a9dcbe3...5bd72db5c`) as the packaged binary that completed the full UI
workflow below.

The full UI workflow below ran on the packaged binary extracted from that RPM
on the host. The system-installed container run was the launch/smoke test; the
full workflow was not repeated from inside the container. See limitations.

## Result summary

| Area                                              | Result                          |
| ------------------------------------------------- | ------------------------------- |
| Database Manager plan section 14 checklist        | 12 pass, 0 fail                 |
| Automated acceptance checks in the packaged build | 51 pass, 0 fail                 |
| MySQL live integration suite                      | 13 pass, 0 fail                 |
| PostgreSQL live integration suite                 | 13 pass, 0 fail                 |
| `npm run check`                                   | Pass                            |
| `npm test -- --maxWorkers=2`                      | 99 files, 682 tests passed      |
| `cargo fmt --check`                               | Pass                            |
| `cargo clippy --all-targets -- -D warnings`       | Pass                            |
| `cargo test`                                      | 830 passed, 0 failed, 1 ignored |
| RPM install and smoke test                        | Pass                            |
| Secrets and tunnel audit                          | Pass                            |
| Windows and macOS                                 | Not run; no hosts available     |

The default `npm test` worker count hit environment timeouts on this shared
host during two SQL-workspace tests. The same tests passed individually and the
full suite passed with two workers, which is the result recorded here. CI runs
the same suite without the shared-host load.

CI results for the pull request are recorded on the pull request itself.

## Section 14 checklist

| #   | Criterion                                                                 | Result   | Evidence                                                                                                                                                                                                                                                                                                                                                           |
| --- | ------------------------------------------------------------------------- | -------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| 1   | Connect to a remote MySQL database through the existing SSH connection    | **Pass** | `prof-3` created a draft test, saved the profile, tested it, and connected. The snapshot reported MySQL `8.4.13`, an active tunnel on `127.0.0.1`, and remote `169.254.1.2:3307`. Queries, metadata, rows, SQL, CSV, dump, roles, sessions, and health all ran through that tunnel.                                                                                |
| 2   | Connect to a remote PostgreSQL database through the same SSH tunnel model | **Pass** | `pg-prof-1` repeated the draft/save/test/connect path on PostgreSQL `18.6` through the same `TunnelManager`. The snapshot reported an active loopback tunnel to `169.254.1.2:5433`; discovery, rows, SQL, CSV, dump, roles, sessions, health, and ER used it.                                                                                                      |
| 3   | Save, test, reconnect, and remove database connection profiles            | **Pass** | `prof-3` and `pg-prof-1` saved and tested profiles. `mysql-failure-3` and `pg-failure-2` reconnected after connection loss. `mysql-failure-4` deleted a saved profile after disconnecting and verified it disappeared from `list_database_profiles`; the credential reference is removed by the same command.                                                      |
| 4   | Browse databases, schemas, tables, and columns                            | **Pass** | `mysql-struct-2` and `pg-struct-2` listed tables, columns, indexes, and constraints. PostgreSQL also listed schemas. `mysql-er-1` loaded a tables explorer node and object counts; PostgreSQL discovery in `pg-struct-2` listed databases and `public`. The UI check expanded the real explorer to `app / Tables / acc_users`.                                     |
| 5   | Inspect table structure, indexes, and constraints                         | **Pass** | `mysql-struct-2` and `pg-struct-2` read live column, index, and constraint metadata. `mysql-struct-4/5` and `pg-struct-4` created, renamed, and dropped indexes and check constraints with metadata re-read verification. The UI Structure tab rendered the table structure after the explorer-kind defect below was fixed.                                        |
| 6   | Create, rename, and drop a table and a column with safe confirmations     | **Pass** | MySQL: `mysql-struct-1` created `acc_users`; `mysql-struct-3` added, altered, renamed, reordered, and dropped a column; `mysql-struct-6` created, renamed, and dropped `acc_table_lifecycle`. PostgreSQL repeated the table and column lifecycle in `pg-struct-1/3/5`. Unconfirmed drop and type-change attempts were rejected.                                    |
| 7   | Browse table rows with server-side pagination and filters                 | **Pass** | `mysql-rows-3` and `pg-rows-2` read pages 1 and 3 of 24 rows (10 + 10 + 4), reported `hasMore`, exact count 24, a `score >= 20` filter returning 4 rows, descending sort, and a single-row search.                                                                                                                                                                 |
| 8   | Insert, edit, and delete rows safely when a stable row identity exists    | **Pass** | `mysql-rows-1` and `pg-rows-1` inserted by identity, re-read the row, updated with an original-value conflict guard, rejected a stale update as `conflict` with rollback, rejected an unconfirmed delete, and deleted with confirmation. A no-identity table is covered by the live row-browser tests and remains read-only.                                       |
| 9   | Run SQL and display results, errors, duration, and query history          | **Pass** | MySQL `mysql-sql-1/2/3` and PostgreSQL `pg-sql-1` returned columns, rows, affected counts, duration, typed timeout and cancellation errors, and history. History recorded success, failure, and cancellation and never contained the database password. Snippets were created, updated, listed, and deleted.                                                       |
| 10  | Handle connection loss, timeouts, permissions, and cancellation clearly   | **Pass** | `mysql-failure-3` and `pg-failure-2` stopped the database server, received a typed `connection` error with a clear message, restarted it, and reconnected. Query timeouts returned `timeout`; cancellation returned `cancelled` and closed the protocol stream as documented. Read-only and limited-account profiles were rejected with typed `permission` errors. |
| 11  | Keep secrets out of logs, history, snippets, and temporary files          | **Pass** | `sec-1` scanned SQLite, WAL/SHM, WebKit storage, history, snippets, driver logs, local work files, and the SSH fixture's `/home/deploy` and `/tmp`. No database password, private-key block, or acceptance key comment was found. The dump tools reported `tempCleaned: true`, and the remote temporary credential search returned no files.                       |
| 12  | Close tunnels when sessions end                                           | **Pass** | `mysql-failure-4` captured a live loopback port, disconnected, and verified the port refused connections. `tunnel-1` confirmed every live listener was `127.0.0.1`-only. `tunnel-2` closed the packaged application and verified its last tunnel port was closed and the process was gone.                                                                         |

## Extended feature checks

| Feature                   | Result   | Evidence                                                                                                                                                                                                                                                                                                                               |
| ------------------------- | -------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Database lifecycle        | **Pass** | `mysql-actions-1` created, set active, and dropped a MySQL database and confirmed rename is capability-rejected. `pg-actions-1` created, renamed, set active, and dropped a PostgreSQL database.                                                                                                                                       |
| CSV import/export         | **Pass** | `mysql-transfer-1` exported 24 rows and imported 24 rows into `acc_users_import`. `pg-transfer-1` exported and imported 24 PostgreSQL rows.                                                                                                                                                                                            |
| SQL backup/restore        | **Pass** | `mysql-backup-1/2` detected `mysqldump`/`mysql` 8.4.13, created a verified 5,838-byte remote backup with temporary cleanup, restored it into `acc_restore`, and read 24 rows. `pg-backup-1` detected `pg_dump`/`pg_restore`/`psql` 18.7, created a verified 5,981-byte backup, restored it into `acc_pg_restore`, and read 24 rows.    |
| Roles and grants          | **Pass** | `mysql-roles-1` created `acc_reader`, previewed and granted table SELECT, inspected effective privileges, revoked it, and dropped the role. `pg-roles-1` repeated the flow for `acc_pg_reader`. Role mutations required confirmation; read-back verification passed.                                                                   |
| Sessions and cancellation | **Pass** | `mysql-sessions-1` and `pg-sessions-1` listed sessions from an observer profile, found the uniquely marked long query on the target connection, cancelled it with the exact confirmation phrase, and verified the query was gone.                                                                                                      |
| Health                    | **Pass** | `mysql-health-1` returned MySQL 8.4.13, a live latency sample, database size, and bounded table sizes. `pg-health-1` returned PostgreSQL 18.6, latency, database size, and table counts.                                                                                                                                               |
| ER metadata               | **Pass** | `mysql-er-1` and `pg-health-1` returned object counts and table dependencies; the UI Diagram tab rendered the read-only relationship view and was captured in `04-diagram.png`.                                                                                                                                                        |
| Explorer and workspace UI | **Pass** | `ui-1` showed both saved profiles after reload. `ui-2` connected the MySQL profile from the UI, expanded the real tree to `acc_users`, and opened Structure, Data, SQL, Diagram, Access, and Sessions workspaces. Screenshots `01-databases.png`, `02-data.png`, `03-sql.png`, and `04-diagram.png` are retained in the run workspace. |

## Defects found and fixed

1. **Trusting a host key crashed the application.** `TunnelConnectionEventSink`
   called `tokio::spawn` from the synchronous `trust_host_key` command, which
   is not running inside a Tokio reactor. The process panicked at
   `src/core/ssh/tunnel.rs`. The sink now skips cleanup when no tunnel exists,
   reuses a captured runtime handle, and falls back to a short-lived runtime on
   a plain thread if a synchronous event occurs before the first async connect.
   Regression: `host_key_approval_from_a_synchronous_thread_does_not_require_a_reactor`.
2. **MySQL timestamp defaults were treated as generated columns.**
   `information_schema.COLUMNS.EXTRA` contains `DEFAULT_GENERATED` for
   `DEFAULT CURRENT_TIMESTAMP`. The broad `%GENERATED%` match made the row
   editor reject writes to those columns. The metadata query now matches only
   `VIRTUAL GENERATED` and `STORED GENERATED`. Regression:
   `column_metadata_distinguishes_generated_columns_from_default_generated_values`
   and the live MySQL row-write test.
3. **PostgreSQL session cancellation always failed.** SQLx binds the session id
   as a bigint, and PostgreSQL has no `pg_cancel_backend(bigint)` overload. The
   fixed statements now cast `$1::integer`. Regression:
   `session_actions_cast_the_bound_id_to_integer`; live verification in
   `pg-sessions-1`.
4. **PostgreSQL dumps passed an unsupported `--sslmode` flag.** `pg_dump`,
   `pg_dumpall`, `pg_restore`, and `psql` rejected it before connecting. The
   mode is now passed as `PGSSLMODE` in the tool environment. Regression:
   `ssl_mode_is_mapped_to_the_client_spelling`; live backup/restore passed.
5. **MySQL role create verification compared the wrong metadata shape.**
   The projected summary marked `superuser` and `createDatabase` unavailable
   while the live adapter returns booleans. The projection now matches the
   adapter shape. Regression:
   `mysql_create_projection_matches_the_live_role_metadata_shape`.
6. **MySQL table-grant verification synthesized a schema.** The adapter
   returned `schema = database` for table grants while the validated request
   contract forbids a schema, so every live grant reported unverified. The
   query now returns a null schema. Regression:
   `table_grant_metadata_does_not_synthesize_a_schema`; live grant/revoke passed.
7. **Every table selection was classified as a view in the explorer.** The
   row model's default object kind was `"view"`, so selecting `acc_users`
   opened the view inspector instead of the table Structure panel. Table rows
   now carry no object kind and use the table structure/inspection path.
   Regression: `databaseExplorerTreeModel.test.ts`; the packaged UI run opened
   the real Structure, Data, SQL, Diagram, Access, and Sessions tabs.

Two further live-only issues were confirmed as documented behavior rather than
defects: a cancelled or timed-out SQL execution closes the protocol stream as
designed, and MySQL `KILL QUERY` can make `SELECT SLEEP(...)` return instead of
raising; the session disappears from the running-query list either way.

## Secret and tunnel audit method

- Recursively scanned the application data directory, including SQLite,
  WAL/SHM, WebKit local storage, and caches, for the MySQL and PostgreSQL
  fixture passwords, the private-key block, and the acceptance key comment.
- Queried the local SQL history and saved snippets for the same values.
- Scanned the automation log, local export/dump work files, and the SSH
  fixture under `/home/deploy` and `/tmp`.
- Confirmed dump temp directories were cleaned and no `.cnf`/`.pgpass` or
  ServerDesk temporary credential file remained on the remote fixture.
- Read `list_database_tunnels` and cross-checked `ss -ltnp`: every ServerDesk
  database listener was bound to `127.0.0.1`, never a wildcard address.
- Disconnected one profile and verified its port closed; exited the packaged
  application and verified the remaining port closed and the process exited.

## Known limitations

- Windows and macOS were not available on this Linux host and were not run.
- The RPM was installed and smoke-tested in a Fedora 44 container, and the
  exact packaged binary completed the full workflow on the host. The full
  workflow was not repeated from inside the container. No result is claimed for
  a system-wide host installation, Windows, or macOS.
- MySQL database rename is unsupported and remains a documented capability
  rejection; the acceptance test asserts that rejection.
- PostgreSQL can browse only its connected catalog in one profile; the restore
  verification used set-active to read the restored database, matching the
  documented limitation.
- Query timeout and cancellation close the MySQL/PostgreSQL protocol stream;
  the UI and acceptance reconnect before the next command. This is documented
  in `docs/database-architecture.md`.
- MySQL row counts are engine estimates except for the opt-in exact count used
  by the pagination check.
- The SSH, MySQL, and PostgreSQL containers bind loopback-only host ports and
  are test fixtures, not part of the application. The application itself opens
  only loopback tunnel listeners.
- The Tauri bundle step prints an updater-signing warning because
  `TAURI_SIGNING_PRIVATE_KEY` is not present; the DEB and RPM themselves build
  successfully. Release signing is performed by the release workflow.

## Reproducing the run

The acceptance harness is intentionally external to the repository because it
needs disposable SSH and database containers. The repository commands used for
the reproducible portions are:

```bash
npm run check
npm test
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml
```

Live database suites are opt-in:

```bash
SERVERDESK_TEST_MYSQL_HOST=127.0.0.1 \
SERVERDESK_TEST_MYSQL_PORT=3307 \
SERVERDESK_TEST_MYSQL_DATABASE=app \
SERVERDESK_TEST_MYSQL_USER=app \
SERVERDESK_TEST_MYSQL_PASSWORD=apppass \
  cargo test --manifest-path src-tauri/Cargo.toml --test database_mysql

SERVERDESK_TEST_POSTGRES_HOST=127.0.0.1 \
SERVERDESK_TEST_POSTGRES_PORT=5433 \
SERVERDESK_TEST_POSTGRES_DATABASE=app \
SERVERDESK_TEST_POSTGRES_USER=app \
SERVERDESK_TEST_POSTGRES_PASSWORD=apppass \
  cargo test --manifest-path src-tauri/Cargo.toml --test database_postgres
```
