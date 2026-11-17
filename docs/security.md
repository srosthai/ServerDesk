# Security and privacy model

ServerDesk is a local-first desktop application. It has no hosted account,
cloud backend, server-side agent, analytics service, or crash reporter. Installed
builds check GitHub Releases for signed updates, as described below. The local
desktop session is trusted; the SSH server and everything it returns are
untrusted.

## Privacy summary

### Data kept on this computer

ServerDesk stores its working data locally in SQLite:

- server profiles: name, host, port, username, authentication type, tags, and
  notes;
- preferences: theme, monitoring interval, and the last active server;
- detected Laravel project metadata;
- trusted SSH host-key fingerprints;
- bounded, sanitized operation history;
- bounded, sanitized database query history and local SQL snippets.

The database, WAL, and shared-memory sidecars are restricted to the owning user
(`0600`) on Unix. SQLite contains credential references, not passwords, private
keys, passphrases, or raw command output. The application-data and credential
locations are listed in [packaging.md](packaging.md#local-data-and-credentials).

### Credentials and the keychain

SSH passwords, private keys (including key passphrases), and database passwords
are stored in the operating system credential store through the `keyring` crate:

| Platform | Credential store                                                |
| -------- | --------------------------------------------------------------- |
| Linux    | Secret Service (GNOME Keyring, KWallet, or compatible provider) |
| macOS    | Keychain                                                        |
| Windows  | Credential Manager                                              |

Entries use the service `com.serverdesk.app` and an account reference such as
`serverdesk:ssh:<uuid>` for SSH or `serverdesk:db:<uuid>` for database passwords.
Database profiles store only that opaque reference; database passwords are
never written to SQLite, command errors, or logs. A desktop Linux session needs
a running, unlocked Secret Service provider; a headless session without one
cannot save or read credentials. The security of the credential store is the
security boundary for secrets at rest. Malware or another process running as
the same OS user can use the unlocked store.

Private keys are wrapped in a versioned JSON envelope inside the credential
entry. Debug output for key material and passphrases is redacted. Deleting a
server profile deletes its credential and trusted host-key records. Deleting a
database profile removes its credential before the profile row, and a locked or
unavailable credential store blocks the deletion so an orphaned secret cannot
be left behind. Package uninstallers and the documented macOS/AppImage cleanup
run `--purge-data`, which reads both the database and the credential index and
removes every credential known to the installation before deleting the local
database.

### Database tunnels

Database connections use an SSH local-forward tunnel by default. ServerDesk
binds the tunnel listener to `127.0.0.1` on an OS-assigned ephemeral port; it
never binds a wildcard or public address, and it never opens a separate SSH
connection for the tunnel. Each local client is carried in a `direct-tcpip`
channel inside the authenticated SSH session.

Forwarded payloads are streamed between the local socket and the SSH channel
without being logged, persisted, or included in errors. Tunnel metadata is
limited to the database profile identifier, remote host and port, optional Unix
socket metadata, lifecycle state, and local loopback port. Listeners and
channels are closed when the SSH session disconnects, the database profile is
deleted, the tunnel is closed, or the application exits.

### Network activity and updates

ServerDesk does not send usage data, analytics, crash reports, diagnostics,
server inventory, or credentials to the project or any third party. It has no
telemetry setting to disable because there is no telemetry client.

The application's network activity is limited to:

- the SSH connection to a server profile you explicitly test or connect to, plus
  the DNS lookup needed to resolve that host;
- update checks and signed update downloads against
  `https://github.com/srosthai/ServerDesk/releases/latest/download/latest.json`
  and the release assets it names;
- opening a release page in the system browser when you explicitly choose a
  manual-download action.

Update checks send an ordinary HTTPS request to GitHub. They do not send server
inventory, credentials, history, preferences, or a device identifier. The
request necessarily exposes normal network metadata such as the public IP
address to GitHub, subject to GitHub's privacy policy. Update checks are skipped
in browser previews and development builds, are throttled on startup, and are
skipped while the device reports that it is offline.

Downloaded updates must match the embedded minisign public key before the
updater replaces anything. A missing or invalid signature is rejected. The
update signature is separate from Windows Authenticode and Apple notarization.
There is no automatic rollback and no remote command channel; see
[updates.md](updates.md) for release signing, platform support, and rollback
limitations.

The opener plugin is used only for explicit release-page links. The webview
loads local application assets; it does not load remote content.

The remote SSH server necessarily sees the connection, the fixed commands
ServerDesk runs, and any terminal input you type. That server and its
administrators are outside ServerDesk's trust boundary. Log and command output
is treated as untrusted input and is redacted on a best-effort basis.

### Removing local data

- Deleting a server profile removes the profile, its credential reference, its
  trusted host keys, its linked project records, and every saved database
  profile for that server. Each database password is deleted from the OS
  credential store before the server row is removed, so a server delete cannot
  orphan database secrets. Operation history keeps its sanitized rows but
  clears the server reference.
- Deleting a database profile removes its saved password from the OS credential
  store before removing the profile row and credential index entry.
- Clearing operation history deletes the local history rows after confirmation.
- Uninstalling with `--purge-data` removes credentials and the application-data
  directory. The exact platform behavior and retry path are in
  [packaging.md](packaging.md#uninstall-and-credential-cleanup).
- There is no cloud copy to delete because there is no cloud account.

## Security audit

This section records the security audit of every remote execution path, the
fixes that were applied, and the risks that remain accepted. The audit assumes
the local desktop session is trusted and the SSH server is untrusted.

## Command construction

All dynamic remote command lines are built in one place:
`src-tauri/src/core/command.rs`.

- User-influenced paths pass through `normalize_remote_path` first. It requires
  an absolute path, rejects control characters, `..` segments, and oversized
  values, and normalizes duplicate separators.
- Paths are quoted as one POSIX shell argument with `shell_quote`. Single quotes
  are escaped as `'\''`, so spaces, quotes, globs, `$()`, backticks, `;`, `|`,
  and redirections stay inside the argument.
- Artisan action names, systemd verbs, and remote command names come from closed
  enums. They are `&'static str` values and never accept caller-provided command
  text.
- Systemd unit names pass `validate_service_unit_name`, which allows only ASCII
  letters, digits, `:`, `_`, `.`, `@`, and `-`, requires the `.service` suffix,
  and rejects whitespace, slashes, option-like names, and control characters.
- Log paths must resolve to the fixed system-log allowlist or to a `.log` file
  directly inside a Laravel `storage/logs` directory. The remote command tails a
  fixed byte bound and never accepts arbitrary files.
- File operations use the SFTP subsystem, not shell commands. Paths are
  normalized, checked against the selected root, and resolved again through the
  SFTP server so symlinks cannot escape the root.
- Host names and SSH usernames reject control characters, whitespace, and shell
  metacharacters before a connection attempt.

The interactive Terminal intentionally sends user keystrokes to a remote shell
as the connected SSH account. It is an explicit product feature, not a command
construction path; it cannot be made injection-free by quoting.

## Confirmations

High-impact actions require an explicit confirmation that names the server,
target, and operation. The backend independently rejects an unconfirmed request
before building or sending a remote command, so bypassing the UI does not run
the action.

| Action                                                | Confirmation                                                    |
| ----------------------------------------------------- | --------------------------------------------------------------- |
| Artisan `config:cache`, `config:clear`, `cache:clear` | Server, project path, and exact fixed command                   |
| Artisan `optimize:clear`, `queue:restart`             | Server, project path, and exact fixed command                   |
| Systemd `stop` and `restart`                          | Server, unit, and `systemctl` operation                         |
| SFTP rename                                           | Server and source/target paths                                  |
| SFTP delete                                           | Server, path, and recursive flag for directories                |
| SFTP upload/download overwrite                        | Server, local/remote paths, and overwrite flag                  |
| Clear operation history                               | Local confirmation before deleting history rows                 |
| Delete server profile                                 | Server and the local records/credential reference being removed |
| Delete saved project                                  | Project and its local record link                               |
| Remove trusted host key                               | Server and key type                                             |

Read-only actions (status inspection, service discovery, log tails, project
browsing, monitoring, identity, fixed `hostname`/`uptime`) do not require a
confirmation.

## Credentials and local storage

- SSH passwords and private keys, including key passphrases, are stored in the
  operating-system credential store through the `keyring` crate. SQLite stores
  only the credential reference.
- Private key credentials are wrapped in a versioned JSON envelope inside the
  credential-store entry. Debug output for key material and passphrases is
  redacted.
- The SQLite database, WAL, and shared-memory sidecars are restricted to mode
  `0600` on Unix. `secure_delete` is enabled so deleted history rows are
  overwritten rather than left recoverable in free pages.
- Operation history stores bounded, sanitized metadata only. Full command
  output, terminal input, `.env` contents, and secrets are never written.
- Database query history stores bounded, sanitized SQL and a short sanitized
  error summary — never result grids, parameters, or raw driver output. Saved
  snippets are sanitized the same way. History is capped at the newest 1,000
  rows, one history SQL body at 8,192 characters, and snippets at 65,536
  characters. Clearing history and deleting a snippet both require explicit
  confirmation.
- There is no file logger, crash reporter, or diagnostic-bundle writer in the
  application. Any future one must call the central redaction module before
  writing a byte; the security test suite guards against adding an unreviewed
  writer.

## Redaction

`src-tauri/src/core/redaction.rs` is the single redaction implementation. It is
applied at the `AppError` boundary, to Artisan and service output, to monitoring
and queue diagnostics, and to bounded log tails.

It removes or replaces:

- dotenv-style assignments, including apparently harmless ones;
- password, passphrase, secret, token, API key, app key, access key, credential,
  authorization, cookie, and session assignments;
- bearer tokens and credentials embedded in URLs;
- common token shapes such as GitHub, Slack, OpenAI-style, and AWS access keys;
- SQL password literals such as `IDENTIFIED BY '...'` and `PASSWORD '...'`;
- private-key blocks;
- ANSI control sequences and terminal control characters.

No redaction is perfect. A secret that does not match a known shape, and that
appears without a recognizable assignment or token prefix, can still be shown
if a remote command prints it. Commands are fixed and read-only (or fixed
actions), which keeps this exposure narrow. The log viewer redacts known
patterns and shows a `Secrets redacted` badge, but the SSH server's own log
content remains untrusted input.

## Least-privilege SSH and sudo

ServerDesk does not require root and does not use a hosted relay.

- Use a dedicated, non-root SSH account for ServerDesk. Do not reuse an
  interactive administrator account.
- Prefer key authentication and protect the private key with a passphrase.
- Host keys are verified before authentication. Unknown keys require explicit
  trust; changed keys block the connection and are never replaced
  automatically.
- Log reading and SFTP use the SSH account's own permissions. ServerDesk never
  elevates for logs or files.
- Systemd status and actions use non-interactive sudo:
  `/usr/bin/sudo -n -- /usr/bin/systemctl <verb> -- <unit>`.
  ServerDesk never prompts for a sudo password and never falls back to root or
  unrestricted sudo.
- Grant one narrowly scoped `NOPASSWD` rule per unit and action, for example:

  ```text
  serverdesk ALL=(root) NOPASSWD: \
    /usr/bin/systemctl show --no-pager --property=Id --property=LoadState \
      --property=ActiveState --property=SubState --property=UnitFileState \
      --property=Description -- nginx.service, \
    /usr/bin/systemctl start -- nginx.service, \
    /usr/bin/systemctl stop -- nginx.service, \
    /usr/bin/systemctl restart -- nginx.service
  ```

  Never grant `ALL=(ALL) NOPASSWD: ALL` or unrestricted sudo.

- The interactive terminal grants whatever shell access the SSH account has.
  Use a dedicated account with only the privileges the operator needs.

## Database Manager security audit

Database Manager adds a second remote surface: SQL, DDL, row writes, CSV
transfers, SQL dumps, role management, session control, and an SSH tunnel per
connection. The audit follows the same trust model as the rest of the app: the
local desktop session is trusted; the SSH server, the database server, and all
returned metadata are untrusted.

### SQL construction and injection resistance

Every database statement is built in one of three reviewed places:

- fixed `&'static str` catalog statements in `database/mysql/sql.rs` and
  `database/postgres/sql.rs`;
- the pure DDL planners in `database/{mysql,postgres}/{ddl,objects,actions,roles}.rs`;
- the row, CSV, and dump builders, which consume validated metadata.

Values never enter SQL text. `ParameterizedQuery` keeps text and `SqlValue`
parameters separate, and the adapter binds them through SQLx placeholders.
MySQL identifiers use backticks and PostgreSQL identifiers use double quotes;
`Identifier` rejects empty, NUL, control, overlong, and multi-segment values
before quoting, and the dialect doubles the engine quote character. A
caller-supplied name that contains `"`, `` ` ``, `;`, or SQL keywords therefore
stays one identifier. Table and column filters/sorts/search terms are resolved
against live metadata first, so an unknown or injected column fails validation
instead of reaching the builder.

DDL literals (defaults, comments) use the engine-specific literal escaper:
MySQL doubles backslashes and quotes, PostgreSQL emits an `E''` string when the
value contains a backslash. Data types, storage engines, character sets,
collations, index methods, and referential actions are closed allowlists or
strict grammars; statement separators and commas outside a type length list are
rejected. MySQL account names use the quoted `'user'@'host'` form and allow only
letters, digits, `_`, `-`, `.`, `$`, and a restricted host pattern.

Generated-column expressions, check constraints, index predicates, view
queries, and routine/trigger bodies are user-authored SQL by design. They are
length and control-character bounded and shown verbatim in the preview, but
they are not parsed or rewritten; review them before execution, because they
run with the selected database account's privileges.

`src-tauri/tests/database_security.rs` is the injection regression suite. CI
runs it as the `Database security suite` step. It attacks filters, search,
sort and visible columns, table and object identifiers, DDL comments, data
types, table options, MySQL account names and hosts, CSV import mappings, dump
database names, session identities, and SQL confirmation paths on both
engines.

### Read-only enforcement

A read-only database profile is enforced at three independent layers:

1. `DatabaseConnectRequest.read_only` makes the adapter apply
   `SET SESSION TRANSACTION READ ONLY` (MySQL/MariaDB) or
   `SET SESSION CHARACTERISTICS AS TRANSACTION READ ONLY` (PostgreSQL) before
   the connection is reported as usable. If the setting fails, connect fails;
   ServerDesk never falls back to a writable session.
2. Every mutating service checks `adapter.is_read_only()` and returns
   `permission` before generating DDL, starting a transaction, or calling an
   adapter mutation, and the row-write and CSV-import adapter methods repeat the
   check before opening a transaction. This covers database lifecycle actions,
   structure changes, objects, roles, sessions, row writes, and CSV imports.
3. The SQL analyzer classifies DML, DDL, locking reads, session changes, and
   sequence/administrative function calls as writes. `set_config`, `setval`,
   `nextval`, `pg_terminate_backend`, `pg_cancel_backend`, `lo_import`,
   `lo_export`, and `dblink_exec` are included so a `SELECT` cannot quietly
   change session state or sequence state.

SQL dump restore is a separate path that runs `mysql`, `psql`, or `pg_restore`
over SSH with the saved profile password. It previously did not consult the
read-only flag. The command and `DatabaseDumpService::restore` now reject a
read-only profile before loading a password, detecting a tool, reading a dump,
or opening a remote session. Export remains available because it is read-only.

Changing a saved profile closes its live adapter and tunnel when connection
details or the read-only flag change, so a session opened writable can never
outlive the profile state that says it is read-only.

### Confirmations enforced in the backend

Every high-impact database action requires a backend confirmation. Bypassing
React and calling the Tauri command directly returns `cancelled` or
`validation` before SQL or a remote tool runs.

| Action                                                                                     | Backend requirement                                                          |
| ------------------------------------------------------------------------------------------ | ---------------------------------------------------------------------------- |
| Drop database                                                                              | `confirmed: true`                                                            |
| Drop table, truncate, drop column, change column type                                      | `confirmed: true`                                                            |
| Drop index or constraint                                                                   | `confirmed: true`                                                            |
| Drop or replace a view, routine, trigger, sequence                                         | `confirmed: true`                                                            |
| Create, alter, grant, revoke, or assign a role                                             | `confirmed: true` (every role action)                                        |
| Cancel a query or terminate a session                                                      | Typed phrase `cancel query <id>` or `terminate session <id>`                 |
| Delete table rows                                                                          | `confirmed: true` after reviewing the target count                           |
| `DROP`, `TRUNCATE`, `ALTER ... DROP`, unqualified `DELETE`/`UPDATE`, always-true predicate | `confirmed: true` in `execute_database_sql`                                  |
| Restore a SQL dump                                                                         | `confirmed: true` plus a document naming server, database, and replaced data |
| Clear query history or delete a snippet                                                    | `confirmed: true`                                                            |
| Delete a saved database profile                                                            | `confirmed: true`                                                            |

The structured builders also re-read metadata after execution and report
`verified: false` rather than claiming success when the engine did not reach
the requested state.

### Secrets in storage, history, errors, and temporary files

- Database passwords live only in the OS credential store. SQLite stores an
  opaque `serverdesk:db:<uuid>` reference. Profile responses never include the
  password or the reference.
- Query history and saved snippets are bounded and sanitized before storage.
  The sanitizer covers `IDENTIFIED BY`, `IDENTIFIED WITH ... AS`, `PASSWORD`,
  `SET PASSWORD`, `SET PASSWORD FOR`, `PASSWORD('...')`, generic assignments,
  bearer tokens, credentials in URLs, private-key blocks, token shapes, and
  ANSI control sequences.
- Query errors keep only a stable kind, a bounded safe message, the engine code
  or SQLSTATE, and a statement position. Driver messages that can echo SQL or
  account names are not forwarded.
- Dump tools receive the password in `MYSQL_PWD` or `PGPASSWORD`, read from a
  temporary file under `/tmp/serverdesk-dump-<uuid>` (mode `0700`, file mode
  `0600`). The password is never an argument, result field, progress event,
  history field, or log line.
- Dump and CSV partial files are created with mode `0600` and removed on
  success, failure, cancellation, and timeout. The remote dump directory is
  removed on every normal exit path.
- There is no file logger, crash reporter, or diagnostic bundle writer. If one
  is added, it must call the central redactor and extend `tests/security.rs`.
- `src-tauri/tests/database_security.rs` writes a database profile, query
  history, and a saved snippet with known secret fixtures, then scans the
  SQLite database and its WAL/SHM sidecars for plaintext. It also checks the
  database file mode.

### Least-privilege database accounts

ServerDesk does not create, use, or request a superuser database account, and
it never elevates with `SET ROLE`, `GRANT ALL`, or an administrative bypass.
Every statement runs as the account saved in the profile, through the SSH
account and loopback tunnel. Permission failures are mapped to `permission`
with the affected object.

Recommended setup:

- use a dedicated read-only account and a read-only profile for browsing, the
  SQL workspace, and evidence gathering;
- grant only the table and schema privileges the operator needs (`SELECT`,
  schema `USAGE`, and `CONNECT` where the engine requires it);
- use a separate writable profile with `INSERT`/`UPDATE`/`DELETE` only for the
  tables that need row editing;
- grant DDL (`CREATE`, `ALTER`, `DROP`) and role management only to an
  administrative profile that is used deliberately;
- session cancel/terminate needs the engine's narrow signal privilege
  (PostgreSQL `pg_signal_backend` membership or MySQL `CONNECTION_ADMIN` where
  applicable), not superuser;
- dump tools run as the SSH account but authenticate to the database with the
  saved DB account. A PostgreSQL cluster dump (`pg_dumpall`) requires broad
  read access and includes role password hashes; protect the destination file
  and prefer per-database dumps when cluster-wide access is not required.

## Audit findings fixed in this pass

1. Service stop and restart had UI-only confirmation. The backend now requires
   `confirmed = true` and rejects the request before command construction.
2. Server-profile, project, and trusted-host-key deletion had UI-only
   confirmation. Each backend command now fails closed without confirmation.
3. Artisan redaction missed standalone token shapes that the history sanitizer
   already handled. Both paths now use the same central redactor.
4. Error diagnostics, service output, and monitoring/queue details could carry
   remote text without redaction. `AppError` now sanitizes at construction and
   the result paths explicitly redact stream fields.
5. Bounded log tails returned raw content. Known secret patterns are now
   redacted and the UI discloses when that happened.
6. Dynamic command construction was spread across modules. Path quoting and
   every command template that accepts user-influenced input now live in
   `core/command.rs`.
7. Host and username validation only rejected whitespace. Both now reject
   control characters and shell metacharacters.
8. SQLite had no explicit file-mode or free-page handling. Database and WAL
   files are private and `secure_delete` is on.
9. There was no dedicated security regression suite. `tests/security.rs` now
   attempts injection through paths, unit names, action names, host/user
   fields, terminal types, transfer IDs, local paths, and history filters, and
   scans SQLite bytes for plaintext secrets.

### Database Manager findings fixed in this pass

1. SQL dump restore did not consult the saved profile's read-only flag. A
   read-only profile could still run `mysql`, `psql`, or `pg_restore` over SSH
   and change data. Both the command and the dump service now reject read-only
   profiles before loading the password or opening a remote session, and a
   regression test asserts that no remote command runs.
2. PostgreSQL dump/restore passed the database name to `--dbname`, which libpq
   also accepts as a full connection string. A value such as
   `host=attacker port=5432` would connect elsewhere and expose `PGPASSWORD`.
   Dump database names now reject `=`, `:`, `/`, `@`, `?`, `&`, `%`, `#`, and
   control characters for both engines, including the profile fallback and
   cluster scope.
3. Deleting a saved database profile had UI-only confirmation. The command and
   the profile service now require `confirmed: true` before closing a session,
   deleting the credential, or removing the row.
4. SQL history and snippet redaction missed MySQL `SET PASSWORD FOR ... =
'...'`, `IDENTIFIED WITH plugin AS '...'`, and `PASSWORD('...')`. These
   literals are now redacted before storage.
5. A read-only PostgreSQL session could be switched to writable across two
   executions with `SELECT set_config('default_transaction_read_only', 'off',
false)`. The analyzer now classifies `set_config` and related
   sequence/session functions as writes, and the adapter's server-side
   read-only transaction remains the second layer.
6. `ALTER TABLE ... DROP COLUMN`, `DROP CONSTRAINT`, and `DROP PARTITION` were
   not confirmation-gated in the SQL workspace because the first write keyword
   was `ALTER`. They are now treated as destructive `DROP` statements.
7. The tunnel listener factory contract ("must bind loopback") was not
   enforced. The manager now checks the bound address and refuses a wildcard or
   public listener even if a future factory implementation returns one.
8. `src-tauri/tests/database_security.rs` was added and wired into CI as the
   dedicated database security suite. It covers filters, sorts, identifiers,
   object names, import mappings, DDL, roles, sessions, dumps, read-only
   bypasses, confirmations, and plaintext-secret storage.
9. Deleting a server profile cascaded its database profile rows in SQLite but
   left each database password in the OS credential store. Server deletion now
   closes the database adapters and deletes every database credential and
   profile row before removing the server, and a regression test asserts the
   credentials cannot outlive the server.
10. MySQL dump table and database names were passed as positional tool
    arguments. A name beginning with `-` could be parsed as a `mysqldump` or
    `mysql` option instead of a name. Dump database and table names now reject a
    leading dash before command construction.
11. Routine `LANGUAGE` clauses interpolated an identifier without quoting it.
    A language value containing `;` could add a second statement. Language names
    are now restricted to a plain identifier allowlist, PostgreSQL quotes the
    value, and MySQL accepts `SQL` only.

## Accepted risks and limitations

- The local desktop session is trusted. Malware running as the same OS user can
  read the database and use the credential store; ServerDesk is not a sandbox.
- SQLite is not encrypted at rest. It contains server metadata, sanitized
  history, and credential references, never plaintext credentials. OS account
  compromise is outside the threat model.
- Redaction is best-effort pattern matching. A secret without a recognizable
  shape can appear in diagnostics or log content.
- The interactive terminal is unrestricted by design for the connected SSH
  account. Do not connect it with an over-privileged account.
- ServerDesk does not verify that a remote server is honest. It bounds, parses,
  and sanitizes output, but a malicious server can return misleading data.
- There is no crash reporter, diagnostic bundle, or remote telemetry. If one is
  added, it must use the central redactor and extend `tests/security.rs`.
- SFTP operations trust the SSH server's path resolution within the selected
  root. A malicious SFTP server can lie about canonical paths; this is the same
  trust implied by using SFTP at all.
- Known-host records are local trust-on-first-use decisions. Users must verify
  fingerprints through an independent channel.
- Transfer progress and partial-path state are held in memory. A crash or an
  application exit can leave `.serverdesk-*.part` files on disk or on the
  remote host without a visible retry record. ServerDesk never resumes a
  transfer automatically; a retry starts from a new partial path.
- Upload completion verifies the remote file type and byte count, not a content
  checksum. A malicious or buggy SFTP server can return metadata for different
  content of the same size.
- Long-running read-only commands are bounded by timeouts but have no per-command
  cancel control. Closing an SSH channel does not prove that a remote process
  stopped, so ServerDesk does not claim that cancellation is safe for commands
  that can change remote state.
- The SQL workspace intentionally runs arbitrary SQL as the configured database
  account. The destructive analyzer is a safety aid, not a SQL parser. A
  side-effecting function with a name outside the keyword list, or a stored
  function called from a plain `SELECT`, is ultimately governed by the account's
  privileges and by the server-side read-only transaction. Use a dedicated
  low-privilege or read-only account; do not treat the warning dialog as a
  security boundary.
- Structure and object editors accept user-authored expressions, view queries,
  index predicates, check constraints, and routine/trigger bodies. Those are
  legitimate DDL code, not values, so they are bounded and displayed verbatim
  but are not parsed. A user with DDL permission can create a definition that
  changes data when invoked; review the preview and use a least-privilege
  account.
- A hard crash, `SIGKILL`, or power loss can leave the remote
  `/tmp/serverdesk-dump-<uuid>` directory and its mode-0600 password file
  behind. Normal success, failure, cancellation, and timeout paths remove it.
  The directory is mode 0700 and the file is mode 0600, but a same-user or root
  process on the remote host can read it before cleanup.
- The dump password is exported in the remote tool process environment
  (`MYSQL_PWD`/`PGPASSWORD`). A same-user process, root, or a process with
  access to `/proc` can read that environment. The password is not in argv,
  history, events, or result payloads.
- Dump contents are user data, not application data. A PostgreSQL cluster dump
  can include role password hashes and every database; protect the destination
  with appropriate file permissions and retention. ServerDesk verifies the
  dump file size on transfer but does not verify its logical contents with a
  checksum.
- Database names containing `=`, `:`, `/`, `@`, `?`, `&`, `%`, or `#`, and
  names beginning with `-`, cannot be used as dump or restore targets because
  the underlying CLI tools can interpret them as connection strings or options.
  This is a deliberate fail-closed limitation.
- Object definitions and sequence details are rendered from server catalog
  text for display. A malicious database can return misleading definition
  text; ServerDesk quotes identifiers it controls but does not execute returned
  definitions automatically.
- The read-only flag is an application safety mode, not a substitute for
  database grants. A user who can edit the local profile can turn it off; use
  OS account separation and a read-only database account when read-only must be
  enforced against the local user.
- CSV exports write database values verbatim so they round-trip. A value that
  starts with `=`, `+`, `-`, or `@` can be interpreted as a spreadsheet formula
  by Excel or another consumer. ServerDesk does not alter exported values;
  treat CSV exports of untrusted data as untrusted input and open them with
  formula execution disabled.
- Temporary CSV and dump partial files contain user data, not credentials, and
  are mode 0600. A crash can leave them on disk; clean them according to local
  policy.
- Dump and CSV export overwrite an existing destination only when the request
  sets `overwrite: true`; the export wizard shows the destination and overwrite
  policy. There is no second backend confirmation document for a file overwrite
  because the flag is explicit and the destination is user-chosen. Protect
  important dump destinations with normal filesystem permissions.
