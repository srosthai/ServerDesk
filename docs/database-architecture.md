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
  capabilities.rs   engine/version capability document
  tunnels.rs        existing loopback SSH tunnel commands

src-tauri/src/database/
  mod.rs            public re-exports and dialect/capability factories
  traits.rs         DatabaseAdapter and typed metadata
  capabilities.rs   action model, flags, version parsing
  dialect.rs        identifier validation/quoting and value binding
  errors.rs         shared database error taxonomy
  pagination.rs     bounded results and page requests
  validation.rs     loopback endpoint and request validation
  fake.rs           in-memory adapter for tests and demos
  mysql/
    mod.rs          MySQL dialect
    adapter.rs      live SQLx MySQL adapter and error mapping
    capabilities.rs MySQL version gates
    sql.rs          MySQL-only metadata SQL
  postgres/
    mod.rs          PostgreSQL dialect
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
- table, column, index, and constraint metadata;
- capability checks through `require_capability`;
- connection checks through `require_connection`.

All metadata methods return `Page<T>`. The adapter owns the engine dialect and
capabilities, so commands do not branch on MySQL versus PostgreSQL SQL.

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
connect, ping, disconnect, capability lookup, and shutdown with the tunnel for
both engines. Tauri commands for this lifecycle are added by the
connection-contract issue; the registry is registered as managed state so those
commands can use it without duplicating driver or tunnel logic.

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

| Action                      | MySQL                              | PostgreSQL    |
| --------------------------- | ---------------------------------- | ------------- |
| `browse_databases`          | Both                               | Both          |
| `browse_schemas`            | Both (schema is a database)        | Both          |
| `browse_tables`             | Both                               | Both          |
| `browse_columns`            | Both                               | Both          |
| `browse_indexes`            | Both                               | Both          |
| `browse_constraints`        | Both                               | Both          |
| `browse_views`              | Both                               | Both          |
| `browse_materialized_views` | Not supported                      | 9.3+          |
| `browse_sequences`          | Not supported                      | Both          |
| `browse_routines`           | Both                               | Both          |
| `browse_triggers`           | Both                               | Both          |
| `browse_roles`              | 8.0+                               | Both          |
| `browse_sessions`           | Both                               | 9.2+          |
| `show_object_size`          | Both                               | Both          |
| `create_database`           | Both                               | Both          |
| `rename_database`           | Not supported                      | Both          |
| `drop_database`             | Both                               | Both          |
| `create_schema`             | Both (schema is a database)        | Both          |
| `drop_schema`               | Both (schema is a database)        | Both          |
| `create_table`              | Both                               | Both          |
| `rename_table`              | Both                               | Both          |
| `truncate_table`            | Both                               | Both          |
| `drop_table`                | Both                               | Both          |
| `add_column`                | Both                               | Both          |
| `edit_column`               | Both                               | Both          |
| `rename_column`             | Both                               | Both          |
| `drop_column`               | Both                               | Both          |
| `reorder_columns`           | Both                               | Not supported |
| `create_generated_column`   | 5.7.6+                             | 12+           |
| `create_identity_column`    | Not supported (use AUTO_INCREMENT) | 10+           |
| `create_index`              | Both                               | Both          |
| `rename_index`              | 5.7+                               | 9.2+          |
| `drop_index`                | Both                               | Both          |
| `create_check_constraint`   | 8.0.16+                            | Both          |
| `create_view`               | Both                               | Both          |
| `create_routine`            | Both                               | Both          |
| `create_trigger`            | Both                               | Both          |
| `refresh_materialized_view` | Not supported                      | 9.3+          |
| `manage_roles`              | 8.0+                               | Both          |
| `kill_session`              | Both                               | 9.2+          |
| `cancel_query`              | Both                               | Both          |
| `explain_query`             | Both                               | Both          |
| `run_query`                 | Both                               | Both          |
| `insert_row`                | Both                               | Both          |
| `edit_row`                  | Both                               | Both          |
| `delete_row`                | Both                               | Both          |

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
and the unavailable-host path. They bind no public port; the container mapping
is explicit `127.0.0.1`. The live path has been exercised against both
MariaDB 11.4 and MySQL 8.4 containers.

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
constraint metadata, typed authentication failure, and the unavailable-host
path. They bind no public port. The live path has been exercised against a
PostgreSQL 18 container.

## Current limitations

- One PostgreSQL connection browses only its connected database catalog.
  Switching databases requires reconnecting with the target database selected;
  cross-database `MetadataScope` and `TableRef` values return `validation`.
- PostgreSQL table metadata does not yet populate estimated row counts, table
  sizes, or materialized-view entries through the shared contract.
- MySQL metadata methods cover databases, schemas, tables, columns, indexes,
  and constraints, but large-catalog paging, object counts, and richer column
  detail remain follow-up work.
- MySQL index and constraint rows are grouped after the metadata query, so a
  table with an unusually large number of index columns is not independently
  paged inside one call.
- Real query execution, transactions, cancellation, streaming, and row editing
  are follow-up work.
- Capability checks are only as accurate as the server version string. An
  unknown version disables version-dependent actions rather than guessing.
- The capability command accepts an engine and raw version; the connection
  flow will replace this with capabilities observed after a real connect.
