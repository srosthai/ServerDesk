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
  structure.rs      structure preview and table lifecycle execution + audit
  tunnels.rs        loopback SSH tunnel inspection commands

src-tauri/src/database/
  mod.rs            public re-exports and dialect/capability factories
  traits.rs         DatabaseAdapter and typed metadata
  actions.rs        lifecycle requests, DDL preview, verified result service
  structure.rs      table/column/index/constraint validation, DDL previews, and table execution
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
    ddl.rs          MySQL table/column/index/constraint DDL planning
    adapter.rs      live SQLx MySQL adapter and error mapping
    capabilities.rs MySQL version gates
    sql.rs          MySQL-only metadata SQL
  postgres/
    mod.rs          PostgreSQL dialect
    actions.rs      PostgreSQL create/rename/drop/set-active planning
    ddl.rs          PostgreSQL table/column/index/constraint DDL planning
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
- engine-specific structure previews for table, column, index, and constraint
  changes;
- table lifecycle execution (create, rename, duplicate, truncate, drop) through
  the validated preview path;
- lifecycle execution and read-only detection;
- table, view, sequence, column, index, and constraint metadata;
- bounded dependency inventories for one table before destructive changes;
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
size, storage engine/access method, and the server-reported statistics refresh
time where available. Views report owner, size, and whether they are
materialized. PostgreSQL sequences report data type, start value, and
increment. MySQL does not expose sequences; the command returns a typed
`capability` error and its object count is unavailable.

Table statistics refresh time comes from `pg_stat_user_tables` on PostgreSQL
and `mysql.innodb_table_stats.last_update` on MySQL. Most MySQL application
accounts cannot read the statistics table, so that optional query is separate
from the table list and its failure becomes `Unavailable`; a table listing never
fails because the account cannot read statistics metadata.

Columns report name, engine type, character length, numeric/temporal precision,
scale, nullability, default, primary-key membership, identity kind,
generated-column status and expression, ordinal position, comment, and
collation. `MetadataValue<Option<T>>` distinguishes "no value for this type"
from "the engine could not report it". For example, a numeric column has
`length: available(null)` and a MySQL 5.7 server reports generated-column
metadata as `unavailable`.

Indexes report name, ordered columns, uniqueness, primary status, access method,
and partial-index predicate where the engine supports one. Constraints report
primary key, unique, foreign key, check, and other constraints with ordered
columns, referenced schema/table/columns, update/delete actions, check
expression, and deferrability. MySQL reports deferrability as unavailable.
PostgreSQL check expressions contain `pg_get_constraintdef` output; MySQL check
expressions contain `CHECK_CLAUSE` from `information_schema.CHECK_CONSTRAINTS`.

`get_database_table_dependencies` returns bounded groups for incoming foreign keys,
dependent views/materialized views, owned sequences, and triggers. Each group is
capped by the same page limit, and each group is independently `available` or
`unavailable`. MySQL reports incoming foreign keys and triggers; MariaDB 11.4
does not expose `information_schema.VIEW_TABLE_USAGE`, and MySQL has no owned
sequences, so those groups are explicitly unavailable instead of being guessed
from view definition text.

Table, column, index, and constraint catalogs are server-paginated. MySQL
selects one page of index or constraint names in a derived table before reading
their column rows, so a table with a very wide index cannot make one metadata
call unbounded. PostgreSQL applies `LIMIT`/`OFFSET` directly to its index and
constraint queries.

PostgreSQL size lookups are gated with `has_table_privilege` /
`has_sequence_privilege`. A restricted account sees the object but receives an
unavailable size instead of a guessed value or a failed list. Any permission
error the engine does raise is still mapped to the typed `permission` code.

Metadata reads attach a typed object scope to permission failures. A failure
while reading one table's columns returns `code: "permission"` with
`scope: { "object": "table", "name": "users" }`, so the explorer can keep the
error on that object instead of replacing the whole workspace. The scope and
message contain no SQL, driver text, connection string, or credential.

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

## Structure preview and execution commands

`src-tauri/src/commands/database/structure.rs` owns the structure command
surface:

| Command                             | Purpose                                                              |
| ----------------------------------- | -------------------------------------------------------------------- |
| `preview_database_structure_action` | Validate and generate DDL for one structure change, no execute       |
| `run_database_structure_action`     | Execute one table, column, index, or constraint change and verify it |

`src-tauri/src/database/structure.rs` validates requests into
`ValidatedStructureAction` values and asks the adapter for a
`StructureDdlPreview`. `mysql::ddl` and `postgres::ddl` are pure planners: they
receive validated identifiers, type names, and typed values only. Execution is
implemented for table create, rename, duplicate, truncate, and drop; column
add, edit, rename, drop, and engine-supported reorder; index create, rename
where supported, and drop; and primary-key, unique, foreign-key, and check
constraint add/drop. The full structure editor remains #174, but it uses this
same preview/capability/execution contract. Every execution command regenerates
the preview before touching the database.

Requests are tagged by `kind` and cover table, column, index, and constraint
changes:

```json
{ "kind": "create_table", "database": "app", "schema": "public", "name": "users", "columns": [{ "name": "id", "dataType": "bigint", "nullable": false, "identity": "by_default" }], "primaryKey": ["id"], "uniqueKeys": [{ "name": "users_email_key", "columns": ["email"] }], "options": { "unlogged": true, "comment": "Users" } }
{ "kind": "duplicate_table", "database": "app", "schema": "public", "name": "users", "newName": "users_copy" }
{ "kind": "add_column", "database": "app", "table": "users", "column": { "name": "email", "dataType": "varchar(320)", "nullable": false }, "position": { "kind": "after", "column": "id" } }
{ "kind": "create_index", "database": "app", "table": "users", "name": "users_email_idx", "columns": ["email"], "unique": true }
{ "kind": "add_constraint", "table": "orders", "constraint": { "kind": "foreign_key", "name": "orders_user_fk", "columns": ["user_id"], "referencedTable": "users", "referencedColumns": ["id"], "onDelete": "cascade" } }
```

Responses carry the action, engine, a structured affected object, statements,
display DDL, warnings, and destructive/confirmation markers:

```json
{
  "action": "add_column",
  "engine": "mysql",
  "object": { "kind": "column", "database": "app", "table": "users", "name": "email" },
  "statements": ["ALTER TABLE `app`.`users` ADD COLUMN `email` varchar(320) NOT NULL"],
  "ddl": "...",
  "warnings": [],
  "destructive": false,
  "requiresConfirmation": false
}
```

Identifiers are validated by the shared `Identifier` rules and quoted by the
selected engine dialect. They are never passed as bind values. User values such
as defaults are typed (`null`, `string`, `number`, `boolean`, or
`current_timestamp`) and rendered as escaped literals; arbitrary default
expressions are not accepted. Generated-column, check, and partial-index
expressions must be a single balanced expression with no statement separator,
comment, or top-level comma. A request that is not a single expression fails
validation before an engine planner runs.

Capabilities gate type changes, column reorder, table/column/index rename,
table duplicate, generated columns, identity columns, auto-increment columns,
check constraints, and partial indexes. MySQL and PostgreSQL add their own
restrictions: MySQL requires the full column definition for `MODIFY COLUMN`,
does not support standard identity columns or partial indexes, and has no safe
reorder syntax other than replacing the full definition; PostgreSQL does not
support `AUTO_INCREMENT`, virtual generated columns, or stored-column
reordering. Unsupported actions return a typed `capability` error before any
DDL is generated. Destructive previews (drop table, truncate, drop column, and
type changes) set `destructive`; drop index and drop constraint set
`requiresConfirmation` without claiming data loss. Previews never execute SQL or
touch the live session beyond the connection check.

Index create supports ordered columns, uniqueness, an engine access method, and
a PostgreSQL partial-index predicate. Index rename is available on MySQL 5.7+
and PostgreSQL 9.2+; on an older server the capability check disables the action
before DDL is generated and returns the explanation the UI shows beside the
disabled control. MySQL has no partial indexes, and both engines reject an
unknown access method before planning. Constraints cover primary key, unique,
foreign key, and check. Foreign keys carry the referenced schema, table, ordered
columns, and ON UPDATE/ON DELETE actions; MySQL rejects `SET DEFAULT` where
InnoDB cannot express it. Check constraints are gated at MySQL 8.0.16+ and are
baseline on PostgreSQL.

Table create supports typed columns, a primary key, named unique keys, and
engine-specific options. MySQL accepts a storage engine, character set,
collation, and table comment; PostgreSQL accepts `UNLOGGED`, a tablespace, and
a table comment. Passing a PostgreSQL-only option to MySQL, or the reverse,
returns a typed `capability` error before DDL is generated. Duplicate copies the
table definition (`CREATE TABLE ... LIKE` in MySQL and
`CREATE TABLE ... (LIKE ... INCLUDING ALL)` in PostgreSQL) but never rows or
foreign keys, and the preview says so.

Execution requires `confirmed: true` for drop table, truncate, drop column, type
changes, drop index, and drop constraint. A missing confirmation returns
`cancelled` and no statement runs. Read-only profiles return `permission` before
execution. Drop and truncate inspect incoming foreign keys and, for drop,
dependent views. Drop index inspects the constraint it backs and drop of a
primary-key or unique constraint inspects incoming foreign keys that reference
its columns; a blocking object returns a typed `dependency` error naming the
blockers. The preview adds the same dependency notices before the user
confirms. Driver foreign-key and dependent-object codes map to the same typed
error.

After execution the service re-reads metadata. Create verifies the table exists
with the expected columns; rename verifies the old name is gone and the new name
exists; duplicate verifies the target exists and its columns match the source;
truncate verifies the table still exists and its reported row estimate is zero
when available; drop verifies the table is absent. Index create verifies the
index exists with the requested ordered columns, uniqueness, method, and
predicate where reported; index rename verifies the old name is gone and the new
name exists; index drop verifies it is absent. Constraint create verifies the
kind, ordered columns, referenced table/columns, update/delete actions, and
check expression where reported; constraint drop verifies the exact named kind
is absent. A failed or partially applied operation returns `verified: false`
with a safe explanation instead of claiming success.

Audit history records one `database` category row per structure action. The safe
summary names the action, engine, table, object, and connection profile only
when metadata verification succeeded, is marked `unknown` when verification
fails or a timeout/connection error leaves the outcome unclear, and is marked
`cancelled` when confirmation is missing. Generated DDL, row data, credentials,
and driver text are never stored.

Golden DDL fixtures live in `src-tauri/tests/fixtures/database_structure/` and
are compared through the fake adapter by `src-tauri/tests/database_structure_ddl.rs`.
Table, column, index, and constraint execution tests use the same fake adapter
and the opt-in MySQL and PostgreSQL container fixtures. Index and constraint
tests cover create, rename, drop, capability rejection, dependency blocking,
read-only denial, verification failure, and audit history. Column execution
verifies type, nullability, default, comment, identity/generated state, and
position where applicable; MySQL MODIFY COLUMN requests must include the
complete type and nullability, and PostgreSQL rejects unsupported stored-column
reordering with a capability error.

`src/services/databaseStructure.ts` is the only React module that invokes the
structure commands. It exposes typed preview, table create/rename/duplicate/
truncate/drop helpers, column add/edit/rename/drop/reorder helpers, and index
create/rename/drop plus constraint add/drop helpers. Truncate, column drop, drop
index, drop constraint, and other destructive actions require an explicit
confirmation argument. The full structure UI remains #174.

## Discovery command contract

`src-tauri/src/commands/database/discovery.rs` owns metadata browsing:

| Command                           | Purpose                                           |
| --------------------------------- | ------------------------------------------------- |
| `list_database_databases`         | Bounded database list                             |
| `list_database_schemas`           | Bounded schema list                               |
| `list_database_tables`            | Bounded base-table list for one scope             |
| `list_database_views`             | Bounded view/materialized-view list for one scope |
| `list_database_sequences`         | Bounded sequence list; MySQL returns `capability` |
| `list_database_columns`           | Bounded column metadata for one table             |
| `list_database_indexes`           | Bounded index metadata for one table              |
| `list_database_constraints`       | Bounded constraint metadata for one table         |
| `get_database_table_dependencies` | Bounded dependency inventory for one table        |
| `get_database_object_counts`      | One-row table/view/sequence counts for one scope  |
| `load_database_explorer_node`     | Lazy load one typed explorer node                 |
| `refresh_database_explorer_node`  | Refresh exactly that node and no siblings         |

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

## Workspace tabs

`src/stores/databaseWorkspaceStore.ts` owns one tab per open database profile.
Each tab stores its active workspace view, selected database/schema/object,
SQL draft, last result document, and background-task records. Tabs are keyed by
profile id and kept in an explicit order; switching or closing a tab never
copies fields into another connection.

The Database Manager renders the open connections as a tab strip above the
workspace. Every connection tab shows the profile name and engine, and the
workspace header for the Overview, Structure, Data, SQL, Indexes, and
Operations views repeats the connection name, engine, endpoint, connection
state, and selected database/schema. A tab opens from the connection list, the
explorer, or Connect; connecting another profile leaves existing tabs open.

Closing a tab removes only that profile's tab state and explorer cache, then
disconnects the profile. The disconnect command closes the adapter and its
loopback tunnel even when no live adapter remains. Disconnecting without
closing keeps the tab's context so a later connect or reconnect can restore the
selected database, SQL draft, and last results. The explorer remains scoped by
profile id, so neither disconnect nor close can mutate another connection's
tree.

## Capability model

`DatabaseCapabilities` combines:

- the selected `DatabaseEngine`;
- an optional parsed `DatabaseVersion`;
- `CapabilityFlags` for schemas, sequences, materialized views, routines,
  triggers, roles, sessions, transactions, savepoints, cancellation, explain,
  check constraints, generated columns, identity columns, column reordering,
  table duplicate, database rename, index rename, and query timeouts;
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

| Action                      | MySQL                              | PostgreSQL                      |
| --------------------------- | ---------------------------------- | ------------------------------- |
| `browse_databases`          | Both                               | Both                            |
| `browse_schemas`            | Both (schema is a database)        | Both                            |
| `browse_tables`             | Both                               | Both                            |
| `browse_columns`            | Both                               | Both                            |
| `browse_indexes`            | Both                               | Both                            |
| `browse_constraints`        | Both                               | Both                            |
| `browse_views`              | Both                               | Both                            |
| `browse_materialized_views` | Not supported                      | 9.3+                            |
| `browse_sequences`          | Not supported                      | Both                            |
| `browse_routines`           | Both                               | Both                            |
| `browse_triggers`           | Both                               | Both                            |
| `browse_roles`              | 8.0+                               | Both                            |
| `browse_sessions`           | Both                               | 9.2+                            |
| `show_object_size`          | Both                               | Both                            |
| `create_database`           | Both                               | Both                            |
| `rename_database`           | Not supported                      | Both                            |
| `drop_database`             | Both                               | Both                            |
| `set_active_database`       | Both (reopens the session)         | Both (reopens the session)      |
| `create_schema`             | Both (schema is a database)        | Both                            |
| `drop_schema`               | Both (schema is a database)        | Both                            |
| `create_table`              | Both                               | Both                            |
| `rename_table`              | Both                               | Both                            |
| `duplicate_table`           | Both (`CREATE TABLE ... LIKE`)     | Both (`LIKE ... INCLUDING ALL`) |
| `truncate_table`            | Both                               | Both                            |
| `drop_table`                | Both                               | Both                            |
| `add_column`                | Both                               | Both                            |
| `edit_column`               | Both                               | Both                            |
| `rename_column`             | Both                               | Both                            |
| `drop_column`               | Both                               | Both                            |
| `reorder_columns`           | Both                               | Not supported                   |
| `create_generated_column`   | 5.7.6+                             | 12+                             |
| `create_identity_column`    | Not supported (use AUTO_INCREMENT) | 10+                             |
| `create_index`              | Both                               | Both                            |
| `rename_index`              | 5.7+                               | 9.2+                            |
| `drop_index`                | Both                               | Both                            |
| `create_check_constraint`   | 8.0.16+                            | Both                            |
| `create_view`               | Both                               | Both                            |
| `create_routine`            | Both                               | Both                            |
| `create_trigger`            | Both                               | Both                            |
| `refresh_materialized_view` | Not supported                      | 9.3+                            |
| `manage_roles`              | 8.0+                               | Both                            |
| `kill_session`              | Both                               | 9.2+                            |
| `cancel_query`              | Both                               | Both                            |
| `explain_query`             | Both                               | Both                            |
| `run_query`                 | Both                               | Both                            |
| `insert_row`                | Both                               | Both                            |
| `edit_row`                  | Both                               | Both                            |
| `delete_row`                | Both                               | Both                            |

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
write rejection, writable probe-table metadata, column identity/generated
fields, bounded index and constraint metadata, incoming-foreign-key and trigger
dependencies, typed authentication failure, database create/set-active/drop
verification, table create/duplicate/rename/truncate/drop verification, column
add/edit/rename/drop/reorder verification, read-only lifecycle and table
permission denial, the unsupported rename explanation, and the unavailable-host
path. They bind no public port; the container mapping is explicit `127.0.0.1`.
The live path has been exercised against both MariaDB 11.4 and MySQL 8.4
containers.

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
schema listing, schema-scoped table metadata, statistics refresh time,
cross-database rejection, read-only write rejection, identity/generated column
detection, column length/precision/collation, index and constraint metadata,
incoming foreign keys, dependent views, owned sequences, triggers,
create/rename/set-active/drop verification, table
create/duplicate/rename/truncate/drop verification, read-only lifecycle and
table permission denial, active-catalog rename rejection, typed authentication
failure, and the unavailable-host path. They bind no public port. The live path
has been exercised against a PostgreSQL 18 container.

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
- Table duplicate copies the table definition, indexes, defaults, and comments
  but not rows or foreign keys. Copying data is deferred to the import/export
  slice so a duplicate cannot unexpectedly copy a large table.
- MySQL table row counts are engine estimates (`TABLE_ROWS`); they are not exact.
- PostgreSQL `reltuples` can remain `-1` until the table is analyzed, so a
  freshly created table reports estimated rows and statistics refresh time as
  unavailable rather than zero.
- PostgreSQL size metadata requires a privilege check and is unavailable to
  accounts that cannot read the relation.
- MySQL statistics refresh time requires access to `mysql.innodb_table_stats`
  and is usually unavailable to application accounts.
- MySQL/MariaDB view dependencies are unavailable because
  `information_schema.VIEW_TABLE_USAGE` is not portable across supported
  servers. Incoming foreign keys and triggers remain available.
- PostgreSQL and MySQL report generated/identity metadata as unavailable on
  versions that predate those column concepts.
- Index column expressions are reported in the same ordered `columns` list as
  plain columns; the first version does not separately type expression versus
  column entries.
- Real query execution, transactions, cancellation, streaming, and row editing
  are follow-up work. The SQL tab owns a per-connection draft and result panel,
  but its Run action stays disabled until the query commands land.
- Workspace tabs live in frontend memory for the current application run. The
  selected database, SQL draft, and result document are restored across
  reconnect, not across an application restart.
- Capability checks are only as accurate as the server version string. An
  unknown version disables version-dependent actions rather than guessing.
- The standalone capability command accepts an engine and raw version for
  profiles that are not connected yet. Connected snapshots carry capabilities
  observed from the live server instead.
- Profile commands validate and persist metadata, but profile UI is tracked
  separately in issue #164.
