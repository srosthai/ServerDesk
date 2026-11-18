# Database Manager: setup and safe use

The Database Manager connects ServerDesk to MySQL, MariaDB, and PostgreSQL
servers through the SSH profile that is already open. It does not accept a
direct database connection from this computer and does not install or configure
a database server.

This guide covers the first connection, least-privilege database accounts,
read-only mode, TLS choices, SSH tunnel behavior, destructive-action
confirmations, backup and restore requirements, and common failures. The
internal command contract, capability matrix, and test fixtures are in
[database-architecture.md](database-architecture.md).

## What the Database Manager supports

- One saved profile per database connection, attached to an existing server
  profile.
- MySQL/MariaDB and PostgreSQL through a loopback-only SSH local-forward tunnel.
- Read-only profiles, explicit TLS modes, database and schema browsing, table
  structure changes, row editing, SQL, CSV import/export, roles, sessions, and
  SQL dumps.
- Database tooling that runs on the remote server over the authenticated SSH
  session. ServerDesk does not bundle database client tools or upload a desktop
  client to the server.

ServerDesk does not support direct TCP database connections, local Unix-socket
connections, or a database server that is unreachable from the SSH server. The
optional **Socket** profile field is saved as context only; the tunnel still
forwards TCP to **Host** and **Port**.

## Before the first connection

You need:

1. A working ServerDesk server profile with a trusted SSH host key and enough
   access to run the database client and backup tools on that server. See
   [install.md](install.md) for the first ServerDesk connection and
   [ssh-setup.md](ssh-setup.md) for a dedicated SSH account.
2. A database server that listens on an address reachable from the SSH server.
   It does not need to be reachable from your desktop.
3. A database account for ServerDesk. Create a dedicated account as described
   in [Create a least-privilege database account](#create-a-least-privilege-database-account).
4. For SQL dump and restore only: the vendor's client tools installed on the
   remote server. See [Backup and restore](#backup-and-restore).

The database server is not installed by ServerDesk. Use your distribution or
database vendor documentation to install MySQL, MariaDB, or PostgreSQL first.
For example:

```bash
# Ubuntu/Debian
sudo apt install mysql-server       # MySQL
sudo apt install mariadb-server     # MariaDB
sudo apt install postgresql         # PostgreSQL

# Fedora
sudo dnf install mysql-server       # MySQL
sudo dnf install mariadb-server     # MariaDB
sudo dnf install postgresql-server  # PostgreSQL
```

These commands install the database server itself. ServerDesk does not run them
and does not manage the server's service, configuration, or upgrades.

### Check the endpoint on the SSH server

Connect to the SSH server with your normal SSH client and check that the
database is listening. Use the address that the SSH server will connect to.

```bash
ss -ltn | grep -E ':(3306|5432)\b'
```

Then test with the database's own client if it is installed:

```bash
# MySQL or MariaDB; enter the password at the prompt.
mysql -h 127.0.0.1 -P 3306 -u serverdesk_ro -p -e 'SELECT 1'

# PostgreSQL; enter the password at the prompt.
psql -h 127.0.0.1 -p 5432 -U serverdesk_ro -d app -c 'SELECT 1'
```

The failed alternative is important: a server that listens only on a public
interface, or a container port that is not published on the host, is not
automatically reachable through the tunnel. **Host** in ServerDesk is resolved
by the SSH server, not by your desktop.

## Add a database connection

1. Open **Servers**, connect the target server, and complete host-key
   verification.
2. Open **Databases**.
3. Select **Add connection** and fill in the profile:

   | Field             | What to enter                                                                                                       |
   | ----------------- | ------------------------------------------------------------------------------------------------------------------- |
   | Profile name      | A name for this saved connection. Names are unique per server, case-insensitively.                                  |
   | Server            | The existing ServerDesk server profile whose SSH session carries the tunnel.                                        |
   | Engine            | **MySQL** (also used for MariaDB) or **PostgreSQL**. The default port changes to 3306 or 5432.                      |
   | Host              | The database address as seen from the SSH server. Use `127.0.0.1` when the database runs on the SSH server itself.  |
   | Port              | The database TCP port. Defaults are 3306 for MySQL/MariaDB and 5432 for PostgreSQL.                                 |
   | Socket (optional) | A Unix socket path recorded for context only. It is not used to connect.                                            |
   | Database          | A default database. PostgreSQL connects to this catalog; see [PostgreSQL catalog scope](#postgresql-catalog-scope). |
   | Username          | The saved database account, not the SSH account.                                                                    |
   | Password          | The database password. It is written to the OS credential store, not SQLite, and is never shown again.              |
   | SSL mode          | The TLS behavior inside the tunnel. **Engine default** is usually the safest initial choice.                        |
   | Read-only         | Enforces a read-only profile in ServerDesk. It is not a substitute for database grants.                             |

4. Select **Test connection**. Testing opens a transient tunnel, authenticates,
   reads the engine version, and closes the transient tunnel before returning.
   Testing a saved profile does not disturb its active session.
5. Select **Save connection**, then **Connect** on the saved profile.

The workspace stays behind the server profile. If the SSH session is
disconnected, connect it first; the profile test and connect actions need a live
SSH session.

### MySQL and MariaDB first connection

On the SSH server, the normal same-host setup is:

- **Host:** `127.0.0.1`
- **Port:** `3306`
- **Database:** the application database, for example `app`
- **Username:** the dedicated account, for example `serverdesk_ro`
- **SSL mode:** **Engine default** or **Required** if the server enforces TLS

Make sure the account is allowed from the address the database sees, not from
your desktop's address. For a database on the SSH server, grant the account for
`'serverdesk_ro'@'127.0.0.1'`. For a database on a separate host, use the SSH
server's address or private network range in the account host. Do not grant
`'%'` merely to make the first connection work.

MySQL is commonly bound to `127.0.0.1` by the distribution package. Check
`bind-address` and `ss -ltn`. If the database runs in a container, publish its
port on the SSH host (for example `-p 127.0.0.1:3306:3306`) and use
`127.0.0.1:3306`; a container name or container-internal address is not
reachable from the normal host network namespace unless you have arranged
routing.

### PostgreSQL first connection

On the SSH server, the normal same-host setup is:

- **Host:** `127.0.0.1`
- **Port:** `5432`
- **Database:** the catalog to browse, for example `app`
- **Username:** the dedicated account, for example `serverdesk_ro`
- **SSL mode:** **Engine default** or **Required** if the server enforces TLS

PostgreSQL must accept a TCP password connection from the SSH server in
`pg_hba.conf`. A common rule for a same-host database is:

```text
host    app    serverdesk_ro    127.0.0.1/32    scram-sha-256
```

Reload PostgreSQL after changing `pg_hba.conf`, and verify the account with
`psql` from the SSH server before saving the profile. Peer authentication is
for local Unix sockets and does not apply to ServerDesk's tunneled TCP
connection.

### PostgreSQL catalog scope

A PostgreSQL connection can browse only the database it connected to. Selecting
another database in the explorer reopens the session against that catalog; it
does not create one connection that can cross databases. Set a default
**Database** when the profile should open on a specific catalog. Tables and
queries outside the connected catalog are rejected before SQL runs.

MySQL treats a schema as a database, so the database and schema concepts are
the same there.

## Create a least-privilege database account

Use separate accounts for separate jobs. A typical installation has:

- a read-only account for browsing, reports, and evidence gathering;
- a writable account that can change rows only in the tables it needs;
- a DDL/administrative account used deliberately for schema changes;
- optionally, a backup account and a session-observer account.

Do not save a database superuser, `root`, or the `postgres` account in an
ordinary profile. The connected account is the final authority for every
statement ServerDesk sends.

### MySQL and MariaDB

Run these statements as a MySQL administrator. On a default Ubuntu/Debian
installation, `sudo mysql` opens an administrative session; on other systems,
use the administrative account and password you configured. Replace `app`, the
account names, the source host, and the password. Use `127.0.0.1` only for a
database on the SSH server; otherwise use the SSH server's address.

Read-only browsing and reporting:

```sql
CREATE USER 'serverdesk_ro'@'127.0.0.1'
  IDENTIFIED BY 'replace-with-a-long-password';

GRANT SELECT, SHOW VIEW ON `app`.* TO 'serverdesk_ro'@'127.0.0.1';
```

Writable data access, limited to one database:

```sql
CREATE USER 'serverdesk_rw'@'127.0.0.1'
  IDENTIFIED BY 'replace-with-a-long-password';

GRANT SELECT, INSERT, UPDATE, DELETE
  ON `app`.* TO 'serverdesk_rw'@'127.0.0.1';
```

Schema changes inside an existing database:

```sql
CREATE USER 'serverdesk_ddl'@'127.0.0.1'
  IDENTIFIED BY 'replace-with-a-long-password';

GRANT SELECT, INSERT, UPDATE, DELETE, CREATE, ALTER, DROP, INDEX, REFERENCES,
      CREATE VIEW, SHOW VIEW, CREATE ROUTINE, ALTER ROUTINE, EXECUTE,
      TRIGGER
  ON `app`.* TO 'serverdesk_ddl'@'127.0.0.1';
```

Creating a database needs the global `CREATE` privilege; dropping one needs
`DROP` on that database (or a broader grant). Grant only what the account should
be allowed to create or remove:

```sql
-- Required to create arbitrary new databases.
GRANT CREATE ON *.* TO 'serverdesk_admin'@'127.0.0.1';

-- Limit database drops to the databases this account administers.
GRANT DROP ON `app`.* TO 'serverdesk_admin'@'127.0.0.1';
```

A backup account for the app's current dump options needs table read access, the
`PROCESS` global privilege, and trigger read access. `mysqldump` is invoked with
`--single-transaction`, `--quick`, `--triggers`, and `--hex-blob`; it does not
add `--no-tablespaces`. MySQL documents `PROCESS` as required for
`mysqldump` without `--no-tablespaces`, and `PROCESS` exposes the server's
process list, so keep this on a dedicated backup profile:

```sql
CREATE USER 'serverdesk_backup'@'127.0.0.1'
  IDENTIFIED BY 'replace-with-a-long-password';

GRANT SELECT, SHOW VIEW, TRIGGER ON `app`.* TO 'serverdesk_backup'@'127.0.0.1';
GRANT PROCESS ON *.* TO 'serverdesk_backup'@'127.0.0.1';
```

If GTID mode and `gtid_purged` are both enabled, `mysqldump` can also need
`RELOAD` or `FLUSH_TABLES`; grant it only if the tool reports that failure.
Restores execute the dump's SQL, so they need the object and data privileges of
the statements in the file, not just the backup account's read privileges.
Triggers in a dump can also require definer-related privileges.

Some metadata will be unavailable to a restricted account. MySQL hides objects
the account cannot see, and statistics refresh time commonly needs access to
`mysql.innodb_table_stats`. Unavailable means unavailable; it is not an empty
result. To list or control other users' sessions, see
[Session observation and control](#session-observation-and-control).

### PostgreSQL

Run these statements as a PostgreSQL administrator connected to the target
database. On a default Debian/Ubuntu installation, `sudo -u postgres psql`
opens an administrative session. Replace `app`, role names, schema names, and
passwords.

Read-only browsing and reporting:

```sql
CREATE ROLE serverdesk_ro LOGIN PASSWORD 'replace-with-a-long-password';

GRANT CONNECT ON DATABASE app TO serverdesk_ro;
GRANT USAGE ON SCHEMA public TO serverdesk_ro;
GRANT SELECT ON ALL TABLES IN SCHEMA public TO serverdesk_ro;
GRANT SELECT ON ALL SEQUENCES IN SCHEMA public TO serverdesk_ro;
```

`GRANT ... ON ALL TABLES` applies to tables that exist when it runs. Add
`ALTER DEFAULT PRIVILEGES` for tables created later; an example appears below.

Writable data access:

```sql
CREATE ROLE serverdesk_rw LOGIN PASSWORD 'replace-with-a-long-password';

GRANT CONNECT ON DATABASE app TO serverdesk_rw;
GRANT USAGE ON SCHEMA public TO serverdesk_rw;
GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA public TO serverdesk_rw;
GRANT USAGE, SELECT, UPDATE ON ALL SEQUENCES IN SCHEMA public TO serverdesk_rw;
```

PostgreSQL has no database-wide `ALTER` or `DROP` privilege for objects owned by
someone else. A schema-change account must own the objects it changes, or be a
member of a role that does. Create new objects as that role, or transfer
ownership deliberately:

```sql
CREATE ROLE serverdesk_ddl LOGIN PASSWORD 'replace-with-a-long-password';
GRANT CONNECT, TEMPORARY ON DATABASE app TO serverdesk_ddl;
GRANT USAGE, CREATE ON SCHEMA public TO serverdesk_ddl;

-- Only when you intend this role to own existing objects:
ALTER TABLE public.users OWNER TO serverdesk_ddl;
```

Use a deliberate database-creation role when the Database Manager should create
databases. `CREATEDB` is narrower than superuser but still allows creating new
catalogs:

```sql
ALTER ROLE serverdesk_ddl CREATEDB;
```

Creating a database is not the same as dropping one. A role can drop a database
only when it owns that database or has superuser rights. Create databases with
the intended owner, or transfer ownership deliberately, before using drop from
a non-superuser profile.

`ALTER DEFAULT PRIVILEGES` helps future objects, but it applies only to objects
created by the role that runs it. For example, if `app_owner` creates the
tables, run:

```sql
ALTER DEFAULT PRIVILEGES FOR ROLE app_owner IN SCHEMA public
  GRANT SELECT ON TABLES TO serverdesk_ro;
```

A per-database dump needs read access to the objects being dumped. A
cluster-wide `pg_dumpall` includes every database and role, so it normally
requires a superuser. Restores need enough privilege to create the objects in
the dump; `pg_dumpall` restores can also change roles and should be treated as
a server-wide administrative operation.

### Session observation and control

Sessions are privilege-sensitive even when the engine supports them.

MySQL reads `information_schema.PROCESSLIST`. Without `PROCESS`, the account
sees only its own sessions. Cancelling another account's query or connection
needs `CONNECTION_ADMIN` (or equivalent administrative privilege); cancelling
your own is narrower. Put these privileges on a dedicated observer/operator
profile, not on the read-only application account:

```sql
GRANT PROCESS ON *.* TO 'serverdesk_observer'@'127.0.0.1';
GRANT CONNECTION_ADMIN ON *.* TO 'serverdesk_observer'@'127.0.0.1';
```

PostgreSQL exposes visible sessions through `pg_stat_activity`. Membership in
`pg_monitor` provides the read-all-statistics roles needed to see other
sessions; `pg_signal_backend` allows cancelling or terminating other
non-superuser backends. Neither is required for ordinary browsing:

```sql
GRANT pg_monitor TO serverdesk_observer;
GRANT pg_signal_backend TO serverdesk_observer;
```

`pg_signal_backend` does not grant the right to signal superuser backends.
ServerDesk rejects the current ServerDesk session and re-checks the target
before sending a cancellation.

### Verify the grants

From the SSH server, connect with the exact account and check what it can see:

```sql
-- MySQL or MariaDB
SELECT CURRENT_USER(), DATABASE();
SHOW GRANTS FOR CURRENT_USER();
```

```sql
-- PostgreSQL
SELECT current_user, current_database();
SELECT has_table_privilege('serverdesk_ro', 'public.users', 'SELECT');
```

Then reconnect the ServerDesk profile. A profile does not pick up a changed
password or changed grants until it opens a new database session; use
**Reconnect**, or edit and save the profile when connection details changed.

### The Access tab

The **Access** tab lists visible users, roles, and effective privileges. It can
create a role, alter role attributes, grant or revoke privileges, and assign
role membership on servers whose version supports the action. Every role
mutation requires an exact preview and explicit confirmation.

Use the Access tab for changes you intend to make through ServerDesk. It does
not set or display passwords or authentication plugins. Use the server's
administrative client for the first account bootstrap, for password and
authentication-plugin changes, and for privileges the current account cannot
grant. A role operation that the signed-in account is not allowed to perform
returns a typed permission error; it does not elevate itself.

## Read-only mode

**Read-only connection** is an application safety mode. It is enforced in three
places:

1. The database adapter applies a server-side read-only session before it
   reports a successful connection. MySQL/MariaDB uses
   `SET SESSION TRANSACTION READ ONLY`; PostgreSQL uses
   `SET SESSION CHARACTERISTICS AS TRANSACTION READ ONLY`. If that statement
   fails, the connection fails rather than silently becoming writable.
2. Every mutating service checks the profile and returns a `permission` error
   before generating DDL, starting a row transaction, or calling a mutation.
3. The SQL workspace classifies writes, locking reads, session changes, and
   sequence/administrative functions as writes. A read-only profile blocks them
   before execution and locks the SQL workspace read-only switch on.

The SQL workspace also has its own **Read-only** switch for writable profiles.
That switch is a local guard for the current workspace, not a saved profile
setting. The profile's read-only flag remains the source of truth for the
connection and every database action.

Read-only mode is not a database grant. A local user who can edit the profile
can turn it off, so use a database account that has only `SELECT` (and any
required metadata privileges) when read-only must be enforced against the local
user. The server-side read-only session is still a second layer, not a
replacement for grants.

Changing a profile's connection details, TLS mode, or read-only flag closes the
live database session and tunnel. Saving the change does not leave a writable
session running under a profile that now says read-only.

## TLS and SSL mode choices

Every database connection already runs through the SSH tunnel. TLS is a
separate, optional layer between the database driver and the database server.
**SSL mode** is saved in the profile and is also passed to the remote dump and
restore tools.

| Mode                | Behavior                                                                                                                                  |
| ------------------- | ----------------------------------------------------------------------------------------------------------------------------------------- |
| **Engine default**  | MySQL/MariaDB `preferred`; PostgreSQL `prefer`. Use TLS when the server offers it and fall back when it does not.                         |
| **Disabled**        | No database TLS. Traffic is still inside SSH while it is on the tunnel path.                                                              |
| **Preferred**       | Try TLS, fall back to plaintext if the server does not support it.                                                                        |
| **Required**        | Require TLS; do not validate the server certificate. Use this when the server enforces TLS but its certificate is issued by a private CA. |
| **Verify CA**       | Require TLS and validate the certificate chain, but not the hostname.                                                                     |
| **Verify identity** | Require TLS, validate the chain, and match the certificate identity to the endpoint used by the driver.                                   |

The application's database driver validates **Verify CA** and
**Verify identity** against its built-in public trust roots. It does not expose
a custom CA file. A certificate issued by a private CA will fail **Verify CA**,
even if the same certificate works with your distribution's `mysql` or `psql`
client. In that case use **Required** and rely on the authenticated SSH tunnel
for transport protection, or install a publicly trusted certificate.

**Verify identity** is usually a poor fit for this tunnel model: the
application's driver connects to the loopback tunnel endpoint (`127.0.0.1`), so
the database certificate must match that address (including an IP subject
alternative name). Remote dump tools instead connect to the saved **Host** and
may validate a different name. A profile can therefore connect while a dump
tool fails certificate validation, or the reverse. Use **Required** for a
private CA, or use **Verify CA** with a publicly trusted certificate.

If the database is on a separate host from the SSH server, disabling TLS leaves
the SSH-server-to-database segment unencrypted. Use TLS there when the network
between those hosts is not trusted.

## SSH tunnel behavior

Each connected database profile uses the already authenticated SSH session:

```text
ServerDesk database driver
  -> 127.0.0.1:<ephemeral port>       local listener, loopback only
  -> SSH direct-tcpip channel
  -> <Host>:<Port>                     resolved from the SSH server
  -> database server
```

- The tunnel listener binds `127.0.0.1` with an OS-assigned ephemeral port. It
  never binds `0.0.0.0`, a public address, or an IPv6 wildcard. The port is
  shown in the profile and in the workspace status; use `ss -ltnp` on Linux if
  you want to confirm that it is loopback-only.
- ServerDesk opens a separate `direct-tcpip` channel for each forwarded client
  connection inside the one SSH session. It does not open a second SSH
  connection for the database and does not create a remote listening port.
- The SSH server resolves the saved **Host**. It must permit local TCP
  forwarding. A hardened `sshd_config` with `AllowTcpForwarding no` or a
  `PermitOpen` restriction that excludes the database host and port will fail
  the tunnel even when the SSH login itself works.
- Tunnel setup is attempted for 10 seconds. Up to 16 database tunnels can be
  active at once, with up to eight forwarded connections inside one tunnel. A
  duplicate request for the same profile and endpoint reuses the active tunnel;
  a conflicting endpoint or an opening tunnel returns a typed `conflict`
  error.
- The optional **Socket** value is profile context only. It does not change the
  local listener, the SSH channel, or the database driver's TCP connection.

### Cleanup and lifetime

Tunnels are closed when any of these happens:

- the database profile is disconnected;
- the database profile is deleted;
- the SSH server profile is disconnected, deleted, or loses its session;
- the application exits normally;
- a connection test finishes (its transient tunnel is closed before the test
  returns).

A failed connect closes the tunnel that the failed attempt opened. Closing the
application window through the normal shutdown path closes every managed
tunnel. A hard crash, `SIGKILL`, or power loss cannot run cleanup logic, but the
listener belongs to the ServerDesk process and is released by the operating
system when the process dies. Remote database sessions opened through a dead
tunnel are closed by their TCP/SSH connection or by the database server's own
timeout; ServerDesk cannot run a cleanup statement after a hard crash.

### Tunnel layouts

```text
Same host:
Desktop -> SSH -> 127.0.0.1:3306 or 127.0.0.1:5432 -> database

Separate database host:
Desktop -> SSH server -> 10.0.0.15:5432 -> database
                           (Host = 10.0.0.15 in the profile)

Container with a published loopback port:
Desktop -> SSH server -> 127.0.0.1:3307 -> published container port
                           (Host = 127.0.0.1, Port = 3307)

Not supported:
Desktop -> database host directly
Desktop -> Unix socket on the SSH server
ServerDesk -> database via a container name that only resolves inside Docker
```

## Workspace layout

The page uses the normal ServerDesk sidebar and active-server context. The
Database Manager area is arranged as follows:

```text
+--------------------------------------------------------------------------------+
| Active server | SSH state | engine | database/schema context | Refresh | Add   |
+----------------------+---------------------------------------------------------+
| Saved connections    | Overview | Structure | Diagram | Data | SQL | Indexes    |
| and explorer tree    | Operations | Access | Sessions                            |
|                      |                                                         |
| - connections        | Selected database or object content                     |
| - databases          | DDL previews, metadata, result grids, wizards           |
| - schemas            |                                                         |
| - tables/views       |                                                         |
| - routines/triggers  |                                                         |
+----------------------+---------------------------------------------------------+
| Connection/query activity | tunnel, version, latency, database, task state    |
+--------------------------------------------------------------------------------+
```

The explorer loads one node at a time and refreshes a node in place. Selecting a
database shows the health overview; selecting a table opens its structure,
data, diagram, indexes, and operations context. The bottom status area reports
the live connection, tunnel endpoint, version, latency, active database, and
task state. Fields that the engine or account cannot expose are labeled
**Unavailable**, not filled with a guess.

## Destructive actions and confirmations

ServerDesk requires an explicit confirmation for every data-loss or
state-changing action, and the Rust command path repeats the check so bypassing
the dialog does not run the action. A confirmation names the server, database
or object, and the exact action. Structural changes also show generated DDL and
dependency warnings before execution.

| Action                                                                   | Confirmation and effect                                                                                                                                                                                                                     |
| ------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Drop database                                                            | Names server, engine, database, profile, exact `DROP DATABASE`, and generated DDL. Permanently removes every object and row. The current database cannot be dropped; set another active.                                                    |
| Drop or truncate table                                                   | Names the table and exact DDL. Drop removes rows, indexes, and constraints; truncate removes rows but keeps the table definition.                                                                                                           |
| Drop column or change column type                                        | Names the column and exact DDL. A drop removes its values and dependent indexes/constraints; a type change can fail or lose precision.                                                                                                      |
| Drop index, primary key, unique key, foreign key, or check constraint    | Names the object and exact DDL. Dropping a constraint removes a structural guarantee; it does not itself delete rows.                                                                                                                       |
| Drop or replace a view, routine, trigger, sequence, or materialized view | Names the object and exact DDL. Replace and drop can invalidate dependent objects.                                                                                                                                                          |
| Create, alter, grant, revoke, or assign a role                           | Exact preview and explicit confirmation for every role action.                                                                                                                                                                              |
| Delete table rows                                                        | Names the connection, table, and exact target count. A bulk delete applies only to explicitly checked rows.                                                                                                                                 |
| Cancel a query or terminate a session                                    | Requires the exact phrase `cancel query <id>` or `terminate session <id>`. The target account and database are re-checked, the current ServerDesk session is refused, and the outcome is re-read.                                           |
| Destructive SQL in the SQL workspace                                     | `DROP`, `TRUNCATE`, `ALTER ... DROP`, and `DELETE`/`UPDATE` without a scoping `WHERE` or with an always-true predicate require confirmation. The warning repeats the exact SQL and statement index; the backend requires `confirmed: true`. |
| Restore a SQL dump or backup                                             | Requires a read-only preview plus a confirmation document naming the server, target database, tool, and exact replaced-data text. A bare boolean or mismatched name is rejected before any connection or tool starts.                       |
| Clear query history or delete a saved snippet                            | Local confirmation before the local rows are deleted. This does not run SQL.                                                                                                                                                                |
| Delete a saved database profile                                          | Names the profile, server, engine, and local data removed. Deletes the saved password and local profile; it does not drop the database or delete remote data.                                                                               |

Examples of actions that are preview-first rather than destructive
confirmations:

- **Create database** and **rename database** require a DDL preview before
  Apply. MySQL does not support database rename, so the action is disabled with
  an explanation; create a new database and migrate the data instead.
- **Create table**, **rename table**, and **duplicate table** require a preview.
  Duplicate copies the table definition, indexes, defaults, and comments, but
  not rows or foreign keys.
- **Create view/routine/trigger** and **alter sequence** require a preview.
  Their definitions can contain user-authored SQL; review it even when the
  action is not classified as destructive.

Database lifecycle and structure actions re-read metadata after execution and
report `verified: true` only when the expected state is observed. A successful
exit status alone is not treated as proof. An operation that cannot be verified
is shown as unverified, not as a success.

## Backup and restore

The backup and restore wizards run the database vendor's own tools on the
remote server over the existing SSH session. The tools and their privileges must
exist on that server. ServerDesk does not install packages, copy a desktop
client to the server, or fall back to a different tool.

### Tool requirements

| Engine           | Export tools            | Restore tools        |
| ---------------- | ----------------------- | -------------------- |
| MySQL or MariaDB | `mysqldump`             | `mysql`              |
| PostgreSQL       | `pg_dump`, `pg_dumpall` | `pg_restore`, `psql` |

The tool's major version must be at least the database server's major version.
A missing or older tool is reported before the transfer starts and is not
silently replaced. Matching the major version is the safest choice; PostgreSQL
client tools are normally backward-compatible with older servers but cannot
dump a server newer than the client.

Install the client tools on the remote server, not on your desktop.

Ubuntu and Debian:

```bash
sudo apt update

# MySQL client on Ubuntu. On Debian, default-mysql-client is the MariaDB client;
# install a matching MySQL client from the vendor repository when required.
sudo apt install mysql-client

# MariaDB client alternative
sudo apt install mariadb-client

# PostgreSQL client from the distribution; install the matching major version
# from the PostgreSQL package repository when the distribution client is older.
sudo apt install postgresql-client
```

Fedora:

```bash
# MySQL client
sudo dnf install mysql

# MariaDB client alternative
sudo dnf install mariadb

# PostgreSQL client
sudo dnf install postgresql
```

Check the installed versions on the SSH server:

```bash
command -v mysqldump mysql pg_dump pg_dumpall pg_restore psql
mysqldump --version
pg_dump --version
psql --version
```

The **Operations** tab shows a **Dump tools ready** badge after detection. Use
**Check tools** to repeat detection. A tool can be present but incompatible; the
report names the detected version and leaves export or restore disabled.

### Scopes and formats

| Engine           | Scopes                                                           | Formats                   |
| ---------------- | ---------------------------------------------------------------- | ------------------------- |
| MySQL or MariaDB | One database, or up to 100 selected tables                       | Plain SQL only            |
| PostgreSQL       | One database, one schema, up to 100 tables, or the whole cluster | Plain SQL, custom, or tar |

A PostgreSQL cluster dump uses `pg_dumpall` and plain SQL only. PostgreSQL
directory-format dumps are not offered because they produce multiple files.
MySQL dumps include tables, views, and triggers; they do not include stored
routines or events because ServerDesk does not pass `--routines` or `--events`.
Use the vendor tools directly if those objects must be in the backup.

A restore target database must already exist. Create an empty target (or use an
existing disposable database) before starting the wizard. PostgreSQL restore
does not create a new catalog for you.

### Verified output

For an export, ServerDesk writes to a sibling partial file and only moves it
into place after all of these are true:

- the tool exits with status 0;
- the output exists and is a regular, non-empty file;
- the observed size matches the expected size;
- for a local download, the downloaded byte count matches the remote size.

Only then is the result marked with **Size verified: yes**. ServerDesk does not
compute a content checksum or compare the dump's logical contents, so size
verification proves that a complete-looking file was written, not that every
row is logically correct. A local export still writes a temporary dump on the
remote server first, so the server needs free space even when the final file is
on your desktop.

The terminal result reports the tool and version, status, exit code, bytes,
duration, warnings, `partialRemoved`, and `tempCleaned`. A failed or cancelled
export normally leaves the destination unchanged and reports
`mayBeIncomplete: false`. If a moved file or a local download cannot be
size-verified after it was written, the result is failed with
`mayBeIncomplete: true`; inspect and remove that destination before retrying.

For a restore, ServerDesk verifies that the tool exits successfully and reports
the bytes read, tool version, warnings, and cleanup flags. It does not inspect
every restored table or row, and it cannot prove that a restore is logically
complete. A failed or cancelled restore reports `mayBeIncomplete: true` because
the target may contain partially restored objects. ServerDesk does not attempt
an automatic rollback. Verify the schema and data before retrying; a retry can
duplicate rows or fail on already-created objects.

### Restore confirmation and replaced data

The restore wizard first builds a read-only preview. The preview names the
server, database, tool, format, and the exact replaced-data text. Confirm only
after comparing all of them. The backend rejects:

- a bare `confirmed: true` without the confirmation document;
- a different server or target database;
- a different replaced-data string;
- a read-only profile;
- a tool that cannot perform the selected format or scope.

For PostgreSQL, **Clean before restore** changes the confirmation text to state
that existing objects and data may be dropped. For **Omit object ownership**,
the option removes owner commands from an export or archive restore; plain SQL
restores do not accept the option at restore time, so apply it when creating the
dump. Ownership failures are common when restoring a dump created by a
different role; use a disposable target and decide ownership deliberately.

### Paths and cleanup

- **Remote** output stays on the server. **Local** output downloads to this
  desktop. Both paths must be absolute.
- The destination directory must exist. The wizard refuses a missing directory
  and does not create parent directories for you.
- An existing destination is rejected unless **Replace an existing output** is
  selected. The overwrite flag is explicit; there is no second confirmation
  document for a file overwrite.
- The tool writes to a remote temporary directory under
  `/tmp/serverdesk-dump-<uuid>` with mode `0700`. The database password is
  written to a file named `password`, mode `0600`, and read through
  `MYSQL_PWD` or `PGPASSWORD`; it is not placed in process arguments, results,
  progress events, history, or logs.
- The temporary directory, remote partial file, and local partial file are
  removed on success, failure, cancellation, and timeout. A force-quit or
  `SIGKILL` can leave the temporary directory and password file behind; the
  next transfer uses a fresh random directory and does not reuse the old one.
- A process running as the same SSH account, or root, may be able to read the
  environment variable or the temporary password file while the tool runs.
  This is an accepted limitation of running vendor tools over SSH.
- `pg_dumpall` includes role definitions and password hashes. Treat a cluster
  dump as sensitive data and protect its destination and retention. A cluster
  restore follows the dump's `\connect` statements and should be tested on a
  disposable server first.

## Troubleshooting

Start with the exact error code shown in ServerDesk. The backend deliberately
does not forward raw driver messages because they can contain account names, SQL
text, or connection details. The server's own client tools are the best place to
get a specific database error.

### The tunnel cannot be opened

Typical codes: `tunnel`, `connection`, `timeout`, or `conflict`.

1. Confirm the SSH profile is connected and the host key is trusted. A tunnel
   cannot exist without an authenticated SSH session.
2. From the SSH server, test the saved endpoint:
   `ss -ltn | grep ':5432'` and
   `psql -h 127.0.0.1 -p 5432 -U serverdesk_ro -d app -c 'SELECT 1'`.
   Use the MySQL equivalent for port 3306.
3. If the SSH server is the database host, use `127.0.0.1`. If the database is
   on another host, use an address reachable from the SSH server, not from your
   desktop.
4. Check SSH forwarding policy as an administrator on the SSH server:
   `sshd -T | grep -Ei 'allowtcpforwarding|permitopen'`. The server must allow
   local TCP forwarding to the database host and port. `AllowTcpForwarding no`
   or a restrictive `PermitOpen` blocks the channel even when SSH login works.
5. A published container port bound to `127.0.0.1` on the host is reachable;
   a container-only address or unpublished port is not.
6. A `conflict` error means the profile already has an opening or active tunnel
   to a different endpoint, or 16 tunnels are already open. Close the other
   profile tab or disconnect the old connection and try again.

### Database authentication fails

Typical code: `database_auth`.

- Verify the saved database username and password with the vendor client from
  the SSH server. Do not use the SSH username as the database username.
- Check the account's allowed source host. The database sees the SSH server's
  connection, not the desktop's address. For a same-host database, grant
  `'user'@'127.0.0.1'` (MySQL) or a matching `pg_hba.conf` TCP rule
  (PostgreSQL).
- For PostgreSQL, check that the target database exists and that
  `pg_hba.conf` uses a password method that applies to this TCP connection.
  Reload PostgreSQL after editing it.
- For MySQL, confirm the account's authentication plugin is supported by the
  server and client. A password reset in the server does not update the saved
  credential; edit the profile and enter the new password.
- A saved password that is missing or locked in the OS credential store
  produces a credential error before the database connection starts. Unlock
  the keyring or re-enter the password.

### The connection or a query times out

Typical code: `timeout`.

- Tunnel setup has a 10 second bound. Database connect and metadata operations
  use a 10 second connect timeout. A firewall, a stopped database, or an
  unreachable host will usually surface here.
- SQL and row reads default to 10 seconds and allow at most 30 seconds. Raise
  the timeout in the SQL tab for a legitimately slow statement; the Data tab
  and metadata reads do not expose a per-request timeout.
- A database lock, a long-running DDL operation, or a saturated server can
  exceed the bound. Check the server's process list with its own tools.
- A timed-out or cancelled SQL request closes the database protocol session.
  Reconnect the profile before running another query. ServerDesk does not
  reuse a session whose protocol stream may be in an unknown state.
- If only backups time out, the dump may be large; the transfer has its own
  bounds and cancellation. Check the remote disk and the tool's stderr through
  the redacted failure summary, not by rerunning a destructive restore.

### A database action is denied

Typical code: `permission`.

- The saved profile may be read-only. Check the badge on the profile and save a
  writable profile if the action is intended. The SQL workspace's local
  read-only switch can also block a statement; profile read-only is locked on.
- The database account may lack the privilege. Re-run the action's exact
  statement with the vendor client or inspect grants. Metadata may hide
  objects the account cannot see; that is not an empty database.
- PostgreSQL schema changes require ownership of the object or membership in
  the owning role; `CREATE` on the schema alone does not allow altering someone
  else's table.
- Session listing and cancellation need the privileges described in
  [Session observation and control](#session-observation-and-control).
- A role change requires account-management privileges and an exact
  confirmation preview. Use a deliberate administrative account for the first
  bootstrap.

### Backup or restore tools are missing or incompatible

Typical code: `capability` or `not_found`.

- Run `command -v` and `--version` on the SSH server for the tool named in the
  error. The tool must be in the SSH account's `PATH`.
- Install the client tools on the server, not on the desktop. The release
  package does not ship them.
- Upgrade a client whose major version is older than the server's. ServerDesk
  refuses the transfer before writing a file.
- `mysqldump` 8 against an older MySQL server can report an unknown
  `COLUMN_STATISTICS` table. ServerDesk does not pass `--column-statistics=0`
  because MariaDB clients reject that flag. Install a client matching the
  server's vendor and major version.
- Check that the destination directory exists, is writable by the SSH account,
  and has enough free space. A local destination still needs space in the
  remote temporary directory.
- If PostgreSQL restore fails on ownership, create the dump with **Omit object
  ownership** or use an archive restore with that option. Plain SQL stores the
  ownership commands in the file.

### A restore failed or may be incomplete

A failed, timed-out, or cancelled restore is not rolled back automatically.
`mayBeIncomplete: true` means the target database may contain some restored
objects or rows.

1. Do not retry blindly. Inspect the target with the vendor client.
2. Compare the target schema and row counts with the dump's intended state.
3. Use a disposable target or a fresh database for the next attempt.
4. If the target must be reset, use a database snapshot or vendor backup taken
   before the restore. ServerDesk does not provide a rollback command.
5. A restore into a read-only profile is rejected before the password or tool is
   loaded. Create a writable profile and connect it first.

### TLS or certificate errors

Typical code: `connection` with a TLS-related message.

- **Verify CA** uses ServerDesk's built-in public trust roots and has no custom
  CA file. A private CA certificate fails even when the machine's `psql` or
  `mysql` client trusts it. Use **Required** over the authenticated SSH tunnel,
  or install a publicly trusted certificate.
- **Verify identity** checks the certificate against `127.0.0.1` for the app's
  driver. Most database certificates do not include that IP address. Use
  **Verify CA** or **Required** unless you deliberately issued a matching
  certificate.
- Remote dump tools use the saved **Host**, not the loopback listener. They may
  report a TLS failure that the profile does not. Align the certificate names
  or use **Required** for the profile and the tools.
- If the server enforces TLS and the profile says **Disabled**, the connection
  is refused. Match the server's requirement.

### Connection loss during an action

If the SSH or database session drops during a destructive action, ServerDesk
does not report a successful result. It records an unknown or failed outcome
where the operation may have reached the server. Reconnect, inspect the object
with the vendor client or a fresh profile, and only then retry. See
[security.md](security.md) for the full confirmation and audit model.

## Current limitations

- Only the SSH local-forward tunnel is implemented. Direct TCP and Unix-socket
  database connections are not supported.
- The optional **Socket** field is metadata; it does not switch the connection
  to a socket.
- One PostgreSQL connection browses one catalog. Switching databases reopens
  the session and resets session-local state.
- MySQL has no safe database-rename operation, so rename is disabled.
- MySQL table row counts are engine estimates. Exact counts are opt-in and can
  be expensive.
- Rows are editable only when a stable primary key or complete non-null unique
  index is available. Tables without one remain read-only.
- MySQL dumps do not include stored routines or events. PostgreSQL directory
  format is not offered. A cluster dump is plain SQL and includes roles.
- Export verification is file-size verification, not a content checksum.
  Restore verification is tool exit status; a restore can still be logically
  incomplete.
- Dump and CSV temporary files can remain after a force-quit. Clean them
  according to your local and server retention policy.
- The SQL workspace's destructive analyzer is a safety aid, not a SQL parser.
  A stored function with an unexpected side effect is governed by the database
  account's privileges and the server-side read-only session.

## Related documentation

- [Install and first run](install.md)
- [Prepare a least-privilege SSH account](ssh-setup.md)
- [Security and privacy model](security.md)
- [Database adapter architecture](database-architecture.md)
- [Troubleshooting](troubleshooting.md)
- [Database Manager acceptance verification](database-acceptance.md)
