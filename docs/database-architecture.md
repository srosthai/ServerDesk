# Database adapter architecture

The Database Manager shares one adapter contract between MySQL and PostgreSQL.
Engine-specific SQL, metadata queries, version checks, and driver errors stay
inside the adapter layer. Tauri commands and React consume typed metadata,
capability documents, stable error kinds, and bounded pages only.

This slice adds the shared foundation and live SQLx adapters for both MySQL and
PostgreSQL. Both adapters connect through a loopback tunnel endpoint, enforce
read-only sessions, discover the server version, and derive capability flags
from it. The fake adapter remains available for deterministic tests and demos.

## Boundaries

```text
React
  -> src/services (typed Tauri calls; no SQL)
  -> src-tauri/src/commands/database (validation, state lookup, serialization)
  -> src-tauri/src/database/traits.rs (DatabaseAdapter)
  -> mysql/ or postgres/ adapter implementation (engine SQL and driver mapping)
```

Commands must not import `database::mysql::sql` or `database::postgres::sql`.
React never receives SQL text, engine identifiers, driver diagnostics, or
credentials. Capability responses contain action names and boolean flags only.

## Module layout

The structure follows the Database Manager plan's adapter split:

```text
src-tauri/src/commands/database/
  mod.rs            command boundary
  actions.rs        database create/rename/drop/set-active commands + audit
  capabilities.rs   engine/version capability document
  discovery.rs      lazy explorer-node metadata commands
  profiles.rs       saved profile CRUD, test, connect, reconnect, disconnect
  tunnels.rs        loopback SSH tunnel inspection commands

src-tauri/src/database/
  mod.rs            public re-exports and dialect/capability factories
  traits.rs         DatabaseAdapter and typed metadata
  actions.rs        lifecycle requests, DDL preview, verified result service
  discovery.rs      explorer-node request/response and metadata service
  capabilities.rs   action model, flags, version parsing
  dialect.rs        identifier validation/quoting and value binding
  errors.rs         shared database error taxonomy
  pagination.rs     bounded results and page requests
  validation.rs     loopback endpoint and request validation
  fake.rs           in-memory adapter for tests and demos
  mysql/
    mod.rs          MySQL dialect
    actions.rs      MySQL create/drop/set-active planning
    adapter.rs      live SQLx MySQL adapter and error mapping
    capabilities.rs MySQL version gates
    sql.rs          MySQL-only metadata SQL
  postgres/
    mod.rs          PostgreSQL dialect
    actions.rs      PostgreSQL create/rename/drop/set-active planning
    adapter.rs      live SQLx PostgreSQL adapter and error mapping
    capabilities.rs PostgreSQL version gates
    sql.rs          PostgreSQL-only metadata SQL
```

The existing tunnel implementation remains in `src-tauri/src/core/ssh/tunnel.rs`
and is reused by the command layer. Future command modules from the plan
(`connections`, `databases`, `schemas`, `tables`, `columns`, `indexes`,
`constraints`, `rows`, `queries`, `transfers`) add thin typed wrappers over the
same adapter contract.

## Adapter contract

`DatabaseAdapter` covers:

- `connect`, `ping`, `version`, and `close`;
- database and schema listing;
- one exact database summary re-read after a lifecycle action;
- engine-specific DDL previews for create, rename, drop, and set-active;
- lifecycle execution and read-only detection;
- table, view, sequence, column, index, and constraint metadata;
- bounded object counts for one explorer node;
- capability checks through `require_capability`;
- connection checks through `require_connection`.

All list methods return `Page<T>`. The adapter owns the engine dialect and
capabilities, so commands do not branch on MySQL versus PostgreSQL SQL.

## Metadata availability

Database, schema, table, view, and sequence summaries use `MetadataValue<T>`:

```json
{ "status": "available", "value": "utf8mb4" }
{ "status": "unavailable" }
```

`Unavailable` means the engine has no such concept, the account may not read
it, or the server version does not expose it. Adapters must not guess a value
from a missing or null field. Unsupported object counts are unavailable too.
For example, MySQL reports table and view counts but marks sequences and
materialized views unavailable.

Database and schema summaries report owner, encoding, collation, size, and
engine where the engine exposes them. Tables report estimated rows plus owner,
size, and storage engine/access method. Views report owner, size, and whether
they are materialized. PostgreSQL sequences report data type, start value, and
increment. MySQL does not expose sequences; the command returns a typed
`capability` error and its object count is unavailable.

PostgreSQL size lookups are gated with `has_table_privilege` /
`has_sequence_privilege`. A restricted account sees the object but receives an
unavailable size instead of a guessed value or a failed list. Any permission
error the engine does raise is still mapped to the typed `permission` code.

`DatabaseConnectRequest` carries the endpoint, username, read-only flag, SSL
mode, timeout, and a `DatabasePassword` whose debug output is always
`<redacted>`. Adapters convert driver, tunnel, authentication, permission,
timeout, query, capability, and cancellation failures into `DatabaseError`.

## MySQL adapter

`database::mysql::MysqlDatabaseAdapter` uses SQLx with the Tokio runtime and
rustls. It accepts only `127.0.0.1` or `::1` endpoints because the application
never opens a database port outside the SSH tunnel; `DatabaseConnectionRegistry`
feeds it the loopback host and ephemeral port from `TunnelManager`.

The adapter:

- opens one authenticated connection within the profile's connect timeout;
- applies `SET SESSION TRANSACTION READ ONLY` before reporting a read-only
  connect as successful, so a write fails instead of silently running;
- reads `VERSION()` once at connect and builds the capability document from
  that exact server version (including MariaDB vendor suffixes);
- supports `preferred`, `disabled`, `required`, `verify-ca`, and
  `verify-identity` SSL modes;
- maps authentication codes (1045, 1698, and related), permission codes (1044,
  1142, 1143, 1227, 1290, 1370, 1792, 1836), timeouts, protocol failures, and
  connection failures to the shared error kinds;
- bounds connect, ping, version, and metadata operations with timeouts;
- keeps credentials out of debug output, serialized errors, and logs.

Driver diagnostics deliberately contain only the MySQL error number and
SQLSTATE. The server message is not forwarded because it can echo account
names, SQL text, or connection details. A wrong password therefore produces a
`database_auth` error, an unreachable loopback endpoint produces `connection`,
and neither includes the password or a connection string.

## PostgreSQL adapter

`database::postgres::PostgresDatabaseAdapter` uses the same SQLx runtime,
rustls configuration, and loopback-only request validation as the MySQL
adapter. It opens one authenticated connection to the tunnel endpoint, reads
`version()` and `current_database()` once, and builds the capability document
from that exact server version.

The adapter:

- applies `SET SESSION CHARACTERISTICS AS TRANSACTION READ ONLY` before
  reporting a read-only connect as successful, so writes to non-temporary
  objects fail with SQLSTATE `25006` mapped to `permission`;
- supports `prefer`/`preferred`, `disabled`, `allow`, `required`, `verify-ca`,
  and `verify-full` SSL modes;
- maps authentication SQLSTATEs (`28P01`, `28000`), permission SQLSTATEs
  (`42501`, `25006`), timeouts (`57014`, `55P03`), connection failures
  (`3D000`, `57P03`, `53300`, class `08`), and unsupported features (`0A000`)
  to the shared error kinds;
- bounds connect, ping, version, and metadata operations with timeouts;
- keeps credentials out of debug output, serialized errors, and logs.

PostgreSQL stores databases as separate catalogs, so one connection can only
browse schemas, tables, columns, indexes, and constraints in the connected
database. A `MetadataScope` or `TableRef` that names another database returns a
`validation` error before any SQL runs. Table metadata carries both the current
database and the schema, and schema filters flow through every metadata query.
The capability document exposes both `browse_databases` and `browse_schemas`;
`database` is always populated after a successful connect.

Driver diagnostics contain only the PostgreSQL SQLSTATE. Server messages are
deliberately omitted because they can echo account names, SQL text, connection
details, or values from the statement.

`DatabaseConnectionRegistry` owns live adapters by profile id and coordinates
connect, ping, test, disconnect, capability lookup, and shutdown with the tunnel
for both engines. A test uses a transient tunnel key and adapter, so it never
disturbs the active session for the same profile.

## Connection command contract

`src-tauri/src/commands/database/profiles.rs` owns the profile command surface:

| Command                         | Purpose                                                      |
| ------------------------------- | ------------------------------------------------------------ |
| `list_database_profiles`        | List saved profiles for one server                           |
| `get_database_profile`          | Read one profile                                             |
| `create_database_profile`       | Validate, create the saved password, persist metadata        |
| `update_database_profile`       | Validate, replace or create the password, persist metadata   |
| `delete_database_profile`       | Close the session/tunnel, delete credential, delete metadata |
| `test_database_profile`         | Probe through a transient tunnel and adapter                 |
| `connect_database_profile`      | Connect through the profile's SSH tunnel                     |
| `reconnect_database_profile`    | Close and reconnect deterministically                        |
| `disconnect_database_profile`   | Close the adapter and tunnel                                 |
| `get_database_connection_state` | Read a typed connection snapshot                             |

DTOs are camelCase. `DatabaseProfile` exposes `hasCredential` rather than the
opaque credential reference, and neither the reference nor a password is ever
serialized. `DatabaseConnectionSnapshot` exposes the engine, server version,
read-only mode, capabilities, tunnel snapshot, and a structured `AppError`.
Input validation rejects unsafe hosts, zero ports, relative or control-character
socket paths, empty or oversized names, malformed usernames, unknown engines,
and unknown SSL modes. Names are unique per server, case-insensitively, and
duplicates return the `conflict` error code.

`src/services/databaseProfiles.ts` is the only React module that invokes these
commands. `src/stores/databaseProfileStore.ts` owns profile lists, loading and
error state, connection snapshots, per-profile busy flags, and test results.

## Database lifecycle commands

`src-tauri/src/commands/database/actions.rs` owns the lifecycle surface:

| Command                   | Purpose                                                         |
| ------------------------- | --------------------------------------------------------------- |
| `preview_database_action` | Generate engine-specific DDL and warnings without executing     |
| `run_database_action`     | Execute create, rename, drop, or set-active and verify metadata |

Requests are tagged by kind:

```json
{ "kind": "create", "name": "app_next", "encoding": "UTF8", "template": "template0" }
{ "kind": "rename", "name": "app", "newName": "app_next" }
{ "kind": "drop", "name": "app_next" }
{ "kind": "set_active", "name": "app_next" }
```

The service validates every name and option before SQL generation. MySQL
create supports character set and collation; MySQL rejects owner and template
options with a typed `capability` error. PostgreSQL create supports owner,
template, encoding, and collation/locale. PostgreSQL rename is
`ALTER DATABASE ... RENAME TO ...`; MySQL has no safe rename operation at any
version, so the UI disables it and the backend returns a `capability` error
with an explanation.

Drop always requires `confirmed: true`. A missing confirmation returns a typed
`cancelled` error and no statement runs. Structural actions are rejected with
`permission` when the profile is read-only; engine privilege failures are also
mapped to `permission`. The current database cannot be renamed or dropped; the
user must set another database active first.

Every successful execution re-reads one database summary. Create succeeds only
when the new database appears, rename only when both names match the expected
state, drop only when the database is absent, and set-active only when the
connection reports and metadata lists the requested database. An operation
that cannot be verified returns `verified: false` with a safe explanation
instead of claiming success. The result carries only safe metadata, generated
DDL statements, duration, and verification text.

Set-active reopens the session against the selected catalog for both engines.
PostgreSQL needs a new catalog connection; MySQL also reopens rather than
depending on session-only `USE` state. The cached `DatabaseConnectionSnapshot`
is updated so the explorer and SQL context see the new active database.

Audit history records one `database` category row per lifecycle action. The
safe summary names the action, engine, database, and connection profile only
when metadata verification succeeded. Generated SQL, connection details,
credentials, and driver text are never stored.

`src/services/databaseActions.ts` is the only React module that invokes these
commands. The explorer exposes database-row actions for create, set active,
rename, and drop; the action dialog requires a DDL preview before create or
rename, and the drop confirmation names the server, engine, database, exact
`DROP DATABASE` action, and previewed DDL. After any action the explorer
refreshes the databases node and selects the created, renamed, or active
database.

## Discovery command contract

`src-tauri/src/commands/database/discovery.rs` owns metadata browsing:

| Command                          | Purpose                                           |
| -------------------------------- | ------------------------------------------------- |
| `list_database_databases`        | Bounded database list                             |
| `list_database_schemas`          | Bounded schema list                               |
| `list_database_tables`           | Bounded base-table list for one scope             |
| `list_database_views`            | Bounded view/materialized-view list for one scope |
| `list_database_sequences`        | Bounded sequence list; MySQL returns `capability` |
| `get_database_object_counts`     | One-row table/view/sequence counts for one scope  |
| `load_database_explorer_node`    | Lazy load one typed explorer node                 |
| `refresh_database_explorer_node` | Refresh exactly that node and no siblings         |

Explorer requests are tagged by kind:

```json
{ "kind": "tables", "database": "app", "schema": "public" }
```

Responses carry the same tag and one bounded payload:

```json
{ "nodeKind": "tables", "payload": { "items": [], "limit": 100, "offset": 0, "hasMore": false } }
```

Database/schema names are validated by the shared `Identifier` rules before an
adapter runs. Page limits are clamped to the shared bound (default 100, maximum
500). A single-node refresh is a new request for that node only; the backend
keeps no explorer cache.

`src/services/databaseDiscovery.ts` is the only React module that invokes these
commands. `src/stores/databaseExplorerStore.ts` caches one entry per
profile/node key, tracks loading and typed error state per node, replaces only
the refreshed key, and appends later pages to the same cached node.

## Explorer tree UI

`src/features/databases/DatabaseExplorerTree.tsx` renders the navigation tree.
`databaseExplorerTreeModel.ts` builds a flat, virtualized row model from the
profiles, connection snapshots, cached node entries, and expanded-row set.
Connections are always present; databases, schemas, object groups, and leaves
appear only after their parent is expanded and the matching discovery payload is
cached. Expanding one row issues at most one `loadNode` request for that row.
MySQL database nodes expose Tables/Views directly; PostgreSQL database nodes
expose a Schemas group because one connection can only browse its connected
catalog, and other database nodes explain that the active database must change
first.

The tree keeps at most one DOM window of rows mounted (plus the focused row),
uses a fixed row height, and offers a Load more row for pages with `hasMore`.
Row identifiers are built from the profile and object path, not array indexes,
so a parent refresh preserves the selected child. Status rows for loading,
empty, permission, connection, and generic metadata errors render under the
affected node instead of replacing the workspace. Search filters the loaded
branch set and keeps matching ancestors visible; it does not silently issue
unbounded discovery requests. Roving tabindex plus Arrow/Home/End/Enter
handling implement the ARIA tree keyboard model.

## Capability model

`DatabaseCapabilities` combines:

- the selected `DatabaseEngine`;
- an optional parsed `DatabaseVersion`;
- `CapabilityFlags` for schemas, sequences, materialized views, routines,
  triggers, roles, sessions, transactions, savepoints, cancellation, explain,
  check constraints, generated columns, identity columns, column reordering,
  database rename, index rename, and query timeouts;
- the `DatabaseAction` list derived from those flags.

`CapabilitySnapshot` is the only document sent to React. It contains the
engine, version string, flags, and supported action names. `ensure(action)`
returns a `capability` error before execution when the UI or a caller tries an
unsupported action.

Unknown versions keep baseline actions enabled and disable version-dependent
actions. A server must report a usable version before role management,
materialized-view tooling, generated columns, or identity columns become
available.

### Actions by engine and version

Version ranges below are minimums. "Both" means the action is available for
the engine's baseline and every newer supported version.

| Action                      | MySQL                              | PostgreSQL                 |
| --------------------------- | ---------------------------------- | -------------------------- |
| `browse_databases`          | Both                               | Both                       |
| `browse_schemas`            | Both (schema is a database)        | Both                       |
| `browse_tables`             | Both                               | Both                       |
| `browse_columns`            | Both                               | Both                       |
| `browse_indexes`            | Both                               | Both                       |
| `browse_constraints`        | Both                               | Both                       |
| `browse_views`              | Both                               | Both                       |
| `browse_materialized_views` | Not supported                      | 9.3+                       |
| `browse_sequences`          | Not supported                      | Both                       |
| `browse_routines`           | Both                               | Both                       |
| `browse_triggers`           | Both                               | Both                       |
| `browse_roles`              | 8.0+                               | Both                       |
| `browse_sessions`           | Both                               | 9.2+                       |
| `show_object_size`          | Both                               | Both                       |
| `create_database`           | Both                               | Both                       |
| `rename_database`           | Not supported                      | Both                       |
| `drop_database`             | Both                               | Both                       |
| `set_active_database`       | Both (reopens the session)         | Both (reopens the session) |
| `create_schema`             | Both (schema is a database)        | Both                       |
| `drop_schema`               | Both (schema is a database)        | Both                       |
| `create_table`              | Both                               | Both                       |
| `rename_table`              | Both                               | Both                       |
| `truncate_table`            | Both                               | Both                       |
| `drop_table`                | Both                               | Both                       |
| `add_column`                | Both                               | Both                       |
| `edit_column`               | Both                               | Both                       |
| `rename_column`             | Both                               | Both                       |
| `drop_column`               | Both                               | Both                       |
| `reorder_columns`           | Both                               | Not supported              |
| `create_generated_column`   | 5.7.6+                             | 12+                        |
| `create_identity_column`    | Not supported (use AUTO_INCREMENT) | 10+                        |
| `create_index`              | Both                               | Both                       |
| `rename_index`              | 5.7+                               | 9.2+                       |
| `drop_index`                | Both                               | Both                       |
| `create_check_constraint`   | 8.0.16+                            | Both                       |
| `create_view`               | Both                               | Both                       |
| `create_routine`            | Both                               | Both                       |
| `create_trigger`            | Both                               | Both                       |
| `refresh_materialized_view` | Not supported                      | 9.3+                       |
| `manage_roles`              | 8.0+                               | Both                       |
| `kill_session`              | Both                               | 9.2+                       |
| `cancel_query`              | Both                               | Both                       |
| `explain_query`             | Both                               | Both                       |
| `run_query`                 | Both                               | Both                       |
| `insert_row`                | Both                               | Both                       |
| `edit_row`                  | Both                               | Both                       |
| `delete_row`                | Both                               | Both                       |

Additional version-dependent flags:

| Flag                 | MySQL   | PostgreSQL |
| -------------------- | ------- | ---------- |
| `query_timeout`      | 5.7.4+  | Both       |
| `check_constraints`  | 8.0.16+ | Both       |
| `generated_columns`  | 5.7.6+  | 12+        |
| `identity_columns`   | Never   | 10+        |
| `materialized_views` | Never   | 9.3+       |
| `roles`              | 8.0+    | Both       |
| `sessions`           | Both    | 9.2+       |
| `index_rename`       | 5.7+    | 9.2+       |
| `database_rename`    | Never   | Both       |
| `column_reorder`     | Both    | Never      |

Cancellation, forced session termination, and role changes are still
permission-sensitive. A capability flag says the engine can perform the action;
it does not claim the signed-in database account has the required privilege.

## Identifiers and values

`Identifier` rejects empty values, NUL, control characters, dotted segments, and
names longer than 63 bytes (the stricter PostgreSQL limit). Qualified names
must be built from one to three validated segments.

`IdentifierQuoter` owns engine quoting:

- MySQL uses backticks and doubles embedded backticks.
- PostgreSQL uses double quotes and doubles embedded double quotes.

`ValueParameterizer` owns placeholders:

- MySQL emits `?`.
- PostgreSQL emits `$1`, `$2`, and so on.

`ParameterizedQuery` keeps SQL text and `SqlValue` parameters separate. A value
is never interpolated into SQL. Range checks reject zero and oversized parameter
positions. Engine SQL modules only append trusted fixed fragments and identifiers
through these interfaces.

## Bounded results

`PageRequest` clamps the limit to 1–500 rows and the offset to 1,000,000.
`try_new` rejects out-of-range internal requests. `Bounded<T>` truncates to its
limit and reports `truncated`; `Page<T>` reports `has_more` from truncation or
an exact total. Adapters must page metadata and data instead of loading an
entire table by default.

## Error taxonomy

| Code            | Meaning                                             | Typical action                       |
| --------------- | --------------------------------------------------- | ------------------------------------ |
| `tunnel`        | SSH tunnel or loopback listener failed              | Reconnect SSH, check endpoint        |
| `database_auth` | Database rejected the account or password           | Check username, password, account    |
| `timeout`       | Connect, query, metadata, or cancellation timed out | Check server load and timeout bound  |
| `permission`    | Database account lacks the required privilege       | Grant the specific privilege         |
| `query`         | Engine rejected SQL or a metadata request           | Review the statement and diagnostics |
| `capability`    | Engine/version does not support the action          | Disable or choose another action     |
| `cancelled`     | User cancelled the operation                        | Retry when ready                     |
| `validation`    | Local input failed before reaching the engine       | Correct the request                  |
| `connection`    | No live database connection exists                  | Connect before retrying              |
| `internal`      | Local invariant failed                              | Restart or report the problem        |

Commands convert `DatabaseError` to `AppError`. `tunnel`, `database_auth`,
`query`, and `capability` are new stable app error codes; `timeout`,
`permission`, and `cancelled` reuse the existing codes. Error messages and
details are bounded and pass through the central redactor before crossing the
Tauri boundary.

## Fake adapter

`FakeDatabaseAdapter` implements the full trait with deterministic fixture
metadata for MySQL and PostgreSQL. It records connect, ping, and close calls,
can inject a typed connect failure, and returns bounded pages. Tests use it as a
trait object to prove that commands can depend on `DatabaseAdapter` alone
without a real database.

Run the focused suites with:

```bash
cargo test --manifest-path src-tauri/Cargo.toml database::
cargo test --manifest-path src-tauri/Cargo.toml --test database_adapter
cargo test --manifest-path src-tauri/Cargo.toml --test database_mysql
cargo test --manifest-path src-tauri/Cargo.toml --test database_postgres
```

### Live MySQL/MariaDB fixture

The live tests are opt-in and target a loopback-only container. They are
skipped when the fixture variables are absent, so CI does not need a database.
Use Podman or Docker:

```bash
podman run --rm -d --name serverdesk-mysql-test \
  -e MARIADB_ROOT_PASSWORD=rootpass \
  -e MARIADB_DATABASE=app \
  -e MARIADB_USER=app \
  -e MARIADB_PASSWORD=apppass \
  -p 127.0.0.1::3306 docker.io/library/mariadb:11.4

podman port serverdesk-mysql-test    # note the 127.0.0.1:<port> mapping

# The lifecycle fixture creates and drops databases, so the test account needs
# global CREATE and DROP in addition to its database-scoped privileges.
podman exec serverdesk-mysql-test mariadb -uroot -prootpass \
  -e "GRANT CREATE, DROP ON *.* TO 'app'@'%'; FLUSH PRIVILEGES;"

SERVERDESK_TEST_MYSQL_HOST=127.0.0.1 \
SERVERDESK_TEST_MYSQL_PORT=<port> \
SERVERDESK_TEST_MYSQL_DATABASE=app \
SERVERDESK_TEST_MYSQL_USER=app \
SERVERDESK_TEST_MYSQL_PASSWORD=apppass \
  cargo test --manifest-path src-tauri/Cargo.toml \
    --lib database::mysql::adapter::tests::live -- --nocapture

SERVERDESK_TEST_MYSQL_HOST=127.0.0.1 \
SERVERDESK_TEST_MYSQL_PORT=<port> \
SERVERDESK_TEST_MYSQL_DATABASE=app \
SERVERDESK_TEST_MYSQL_USER=app \
SERVERDESK_TEST_MYSQL_PASSWORD=apppass \
  cargo test --manifest-path src-tauri/Cargo.toml \
    --test database_mysql -- --nocapture
```

The fixture tests cover connect, ping, version, capability flags, read-only
write rejection, writable probe-table metadata, typed authentication failure,
database create/set-active/drop verification, read-only lifecycle permission
denial, the unsupported rename explanation, and the unavailable-host path. They
bind no public port; the container mapping is explicit `127.0.0.1`. The live
path has been exercised against both MariaDB 11.4 and MySQL 8.4 containers.

### Live PostgreSQL fixture

The PostgreSQL live tests use the same opt-in pattern and a loopback-only
container:

```bash
podman run --rm -d --name serverdesk-postgres-test \
  -e POSTGRES_PASSWORD=apppass \
  -e POSTGRES_USER=app \
  -e POSTGRES_DB=app \
  -p 127.0.0.1::5432 docker.io/library/postgres:18

podman port serverdesk-postgres-test   # note the 127.0.0.1:<port> mapping

SERVERDESK_TEST_POSTGRES_HOST=127.0.0.1 \
SERVERDESK_TEST_POSTGRES_PORT=<port> \
SERVERDESK_TEST_POSTGRES_DATABASE=app \
SERVERDESK_TEST_POSTGRES_USER=app \
SERVERDESK_TEST_POSTGRES_PASSWORD=apppass \
  cargo test --manifest-path src-tauri/Cargo.toml \
    --lib database::postgres::adapter::tests::live -- --nocapture

SERVERDESK_TEST_POSTGRES_HOST=127.0.0.1 \
SERVERDESK_TEST_POSTGRES_PORT=<port> \
SERVERDESK_TEST_POSTGRES_DATABASE=app \
SERVERDESK_TEST_POSTGRES_USER=app \
SERVERDESK_TEST_POSTGRES_PASSWORD=apppass \
  cargo test --manifest-path src-tauri/Cargo.toml \
    --test database_postgres -- --nocapture
```

The fixture tests cover connect, ping, version, capability flags, database and
schema listing, schema-scoped table metadata, cross-database rejection,
read-only write rejection, identity/generated column detection, index and
constraint metadata, create/rename/set-active/drop verification, read-only
lifecycle permission denial, active-catalog rename rejection, typed
authentication failure, and the unavailable-host path. They bind no public
port. The live path has been exercised against a PostgreSQL 18 container.

## Current limitations

- One PostgreSQL connection browses only its connected database catalog.
  Switching databases reopens the session and updates the explorer context;
  cross-database `MetadataScope` and `TableRef` values return `validation`.
- MySQL set-active also reopens the session to avoid relying on session-local
  `USE` state, so any open result or query state belongs to the previous
  session.
- MySQL has no database owner, table owner, sequence, or materialized-view
  concept. Those fields are explicitly unavailable rather than guessed.
- MySQL has no safe database rename operation. The explorer disables rename
  and explains that a new database plus data migration is required.
- MySQL table row counts are engine estimates (`TABLE_ROWS`); they are not exact.
- PostgreSQL size metadata requires a privilege check and is unavailable to
  accounts that cannot read the relation.
- MySQL index and constraint rows are grouped after the metadata query, so a
  table with an unusually large number of index columns is not independently
  paged inside one call.
- Real query execution, transactions, cancellation, streaming, and row editing
  are follow-up work.
- Capability checks are only as accurate as the server version string. An
  unknown version disables version-dependent actions rather than guessing.
- The standalone capability command accepts an engine and raw version for
  profiles that are not connected yet. Connected snapshots carry capabilities
  observed from the live server instead.
- Profile commands validate and persist metadata, but profile UI is tracked
  separately in issue #164.
