# Features

ServerDesk keeps server inventory and metadata on your computer while all remote work runs over SSH.

## Credentials

SSH passwords and private key material are stored in the operating system credential store through the Rust keyring integration. SQLite stores only a credential reference. Secret input is available in the desktop build, not the browser preview.

Private key authentication supports OpenSSH, PEM (RSA, ECDSA, Ed25519), and PuTTY PPK keys, including passphrase-protected keys. The passphrase is stored with the key inside the credential store entry, so SQLite still holds only the credential reference. Key material can also be supplied as a local file path. Authentication failures return the distinct `authentication` error code; unreadable keys and incorrect passphrases return `credential`. Credentials, key material, and passphrases are redacted from debug output and are never written to logs.

## Connections

A Rust connection manager owns one SSH session per server profile, reuses healthy sessions, rejects duplicate concurrent connects, and emits `connection-state-changed` events to the frontend. Connection states are `disconnected`, `connecting`, `host-key-verification`, `connected`, `reconnecting`, and `error`. A reused session is revalidated with the fixed read-only `true` command before it is reported as connected, so a half-open socket is replaced instead of being presented as live. Connection attempts, session health checks, discovery commands, session closes, and remote commands are bounded by the configured timeouts. A command that times out on a healthy session returns `timeout`; a command whose session died returns `network` and moves the connection to `error`. Running a command with no managed session returns `connection` rather than a missing-record error.

Host keys are checked against the local `known_hosts` records before authentication. An unknown fingerprint opens a blocking verification dialog with the key type and fingerprint; the key is trusted only after explicit acceptance. A changed fingerprint blocks the connection with a typed `host_key` error and is never saved automatically.

The add/edit server form includes a connection test. It reports visible progress, keeps non-secret field values after a failed test, and uses the same host-key verification flow before sending credentials. Auth failures, host-key mismatches, timeouts, DNS failures, and network loss keep distinct error codes and UI labels. Tests against an unsaved profile trust an accepted key only for that test; saving and connecting verifies the key again.

## Interactive terminal

Every terminal session opens a dedicated SSH channel on the managed connection, requests a remote PTY (`xterm-256color` by default), and starts an interactive shell. Multiple independent sessions can share one server connection because each session owns its own channel. Output is streamed to the frontend as base64-encoded `terminal-event` payloads so arbitrary bytes, ANSI control sequences, and colors are preserved. Input and resize events are sent back through typed Tauri commands; resize requests update the remote PTY dimensions.

Closing a session sends an SSH channel close, remote exit statuses are reported, and disconnecting a server closes its terminals before the SSH session is released. Application shutdown also closes every terminal channel. Terminal input is never written to application logs or frontend state, and debug output for terminal input and raw output payloads is redacted.

The terminal page renders each PTY session with xterm.js: full ANSI colors, cursor movement, mouse selection, and a 5,000-line scrollback. Each server can keep multiple shell tabs open, with one independent SSH channel and scrollback buffer per tab. Copy, paste, select-all, and clear actions are available in the terminal toolbar, and resize events are recalculated from both container and window changes. The header always shows the server name, SSH user, host and port, connection state, and active session state. Losing the SSH connection disables input and shows a deliberate reconnect action; remote termination displays the exit reason and exit code. All xterm instances, observers, timers, and event listeners are disposed when a tab closes or the page unmounts. `cargo test --manifest-path src-tauri/Cargo.toml --test terminal_sessions` starts an in-process `russh` server and verifies PTY/shell setup, ANSI output, input routing, resize forwarding, multiple sessions, remote termination, disconnect cleanup, and command execution while a terminal is open. The frontend tests cover tab lifecycle, close cleanup, header context, clipboard actions, resize reporting, and xterm disposal.

## Remote file browser and transfers

The File Manager browses, uploads, downloads, creates, renames, and deletes remote files over the SFTP subsystem on the existing authenticated SSH connection. It uses the same SSH port as terminals, commands, and monitoring; no separate public file-transfer listener is opened. Each operation opens one short-lived SFTP channel and returns name, type, size, `ls -l` style permissions, and modification time for directory listings.

The browser keeps a per-server directory history with Back and Forward controls, and breadcrumbs always start at the selected root. Refreshes and failed navigation do not add duplicate or fabricated history entries. The table lists name, type, size, permissions, and modification time, and stays readable at the 1280 px default window width by using compact row actions. Arrow keys move between rows, Space toggles selection, Enter opens the focused directory, Escape clears selection, and Ctrl/Cmd+A selects the visible entries. Checkboxes support shift-click range selection and a select-all control; selected files and directories can be downloaded into an explicit local folder or deleted together.

The user selects an absolute root directory. Every request is normalized and rejected before any SFTP operation when it contains parent segments, relative components, control characters, or a path outside that root. The root and requested path are also resolved through the SFTP server: if a symlink resolves outside the canonical root, the operation is blocked. Directory entries that are symlinks are marked with their target and whether they stay inside the root; broken and unresolved links are marked separately and are never followed silently. Symbolic links and special files are skipped during recursive transfers rather than followed. A permission-denied response gets its own visible state naming the server, SSH account, and requested path, with a retry action that never claims the listing is live.

Uploads and downloads report `file-transfer-event` progress with byte and file counts. A running transfer can be cancelled; the backend stops at a chunk boundary, keeps the partial file, and returns a result whose status is `cancelled` and whose `mayBeIncomplete` flag and `partialPaths` describe the preserved data. Interrupted transfers return a `failed` result with the same partial information instead of deleting evidence or reporting success. ServerDesk writes to a sibling partial file and renames it only after every byte and file count has been verified, so an existing complete destination is never replaced by an incomplete transfer. After an upload rename, the destination is read again and its size and file type must match the source; a mismatch rolls the destination back (restoring the previous file when overwrite was confirmed) and returns a failed result instead of success. A second transfer to the same server and destination is rejected while the first is running, so a retry cannot write the same path twice. Completion verifies byte and file counts, the destination re-read, and the rename; it does not compute or compare a content checksum. The UI offers retry and a fresh transfer starts from a new partial path.

An existing upload or download destination requires explicit confirmation. The backend returns a typed `conflict` error before writing, and the UI asks the user to confirm overwrite. Rename and delete always require confirmation; deleting a directory additionally requires recursive confirmation. Every create, rename, and delete is followed by an SFTP re-read that reports whether the expected state was verified. The transfer queue lists every active and finished transfer for the selected server with byte/file progress, active count, cancel, retry, and dismiss controls. Overwrite and delete confirmations name the server and every target path.

Listings are capped at 2,000 entries, 15 seconds, and 2,000 transfer operations for recursive directory plans. `cargo test --manifest-path src-tauri/Cargo.toml --test files` starts an in-process `russh` server with an SFTP subsystem and a local fixture tree. It verifies exact upload and download contents, recursive directories, overwrite conflicts, upload and download cancellation with preserved partial data, simulated interruptions and safe retry, rejected duplicate destinations, post-rename size verification with rollback, typed mutation confirmations and partial-delete disclosure, traversal rejection before channel creation, symlink escape detection and blocking, permission and missing-path errors, disconnection, and the entry cap.

## Database Manager workspace

The Databases page provides the Database Manager workspace shell; the backend now exposes MySQL and PostgreSQL adapters but the profile and connection UI is still in development. Its navigation item and `/databases` route reuse the collapsible sidebar, active-server state, SSH connection state, AMOLED theme, shared tabs, dialogs, badges, and empty-state components.

The shell has a top bar with the active server, SSH state, engine placeholder, refresh feedback, and an Add connection entry point; a database explorer rail with disabled object search and connection placeholders; tabs for Overview, Structure, Data, SQL, Indexes, and Operations; and a bottom status area for connection and query activity. Loading, server-profile error, no-server, disconnected, and empty workspace states are explicit. The Add connection flow is a placeholder dialog: it lists the planned steps, saves nothing, loads no driver, and opens no tunnel. The workspace fits the 1280 px default window without horizontal overflow, its tabs support arrow-key navigation, and the explorer rail stacks above the tab area below the desktop breakpoint.

The shared database adapter foundation defines MySQL and PostgreSQL connect, ping, version, close, and metadata operations behind one Rust trait. Engine and version capabilities are exposed as action flags so unsupported UI actions can be disabled before execution. Identifier quoting and value placeholders are centralized per engine, values are never interpolated into SQL, and result pages are bounded. Engine-specific metadata SQL stays in separate `mysql` and `postgres` modules and never crosses the Tauri boundary.

MySQL and MariaDB now have a live SQLx adapter that connects only to the loopback endpoint published by an SSH tunnel. It reads the server version to build capability flags, applies read-only sessions before reporting a successful connect, supports explicit SSL modes, and normalizes authentication, timeout, permission, connection, and protocol failures into typed errors without exposing raw driver text, credentials, or connection strings. PostgreSQL uses the same contract and tunnel model: one connection browses its connected catalog, schema-scoped metadata carries both database and schema names, and read-only sessions reject writes before reporting success. A fake adapter still exercises the full contract without a database, and opt-in container tests cover the live paths for both engines. See [database-architecture.md](database-architecture.md) for the module layout, per-engine/version action matrix, error taxonomy, and fixture instructions.

Typed discovery commands now return bounded pages for databases, schemas, tables, views, and sequences, plus one-row object counts per explorer scope. Every summary field is explicitly `available` or `unavailable`, so MySQL reports its missing owner or sequence concepts as unavailable instead of guessing. `load_database_explorer_node` and `refresh_database_explorer_node` take a tagged node request and return only that node's payload. The React service and Zustand explorer store cache one entry per profile/node key, track loading and typed permission/connection errors per node, and replace only the refreshed node. The tree UI follows in issue #166.

## Remote commands

The Overview page provides the first end-to-end vertical slice: add a profile, connect, verify the host key, authenticate, run fixed `hostname` and `uptime` commands, inspect real stdout, stderr, the exit code, and the connection state, then disconnect and release the session. The Rust command path accepts a closed `RemoteCommand` enum and maps it to fixed command templates, so user text is never concatenated into a shell command. A non-zero exit status is returned as a structured result rather than lost as a transport error.

`cargo test --manifest-path src-tauri/Cargo.toml --test remote_commands` starts an in-process `russh` server and exercises the real client transport, host-key verification, password authentication, command execution, failure output, and disconnect cleanup.

## System identity

The Overview page samples server identity over the authenticated session. Hostname, distribution, and version come from fixed read-only templates (`hostname` and `cat /etc/os-release`), kernel and architecture come from `uname -srm`, and uptime comes from `cat /proc/uptime`. Parsing is per field: if `/etc/os-release` is missing or a distribution does not publish `VERSION_ID`, only those values are shown as unavailable.

Every identity command has a 16 KiB output bound and uses the connection manager's command timeout. The typed `get_system_identity` Tauri command returns a `SystemIdentity` sample with a local sample timestamp, duration, and connection state. The frontend calls it through `src/services/identity.ts` and stores per-server samples in `useSystemIdentityStore`.

## Monitoring snapshots

The Overview page collects lightweight, snapshot-based monitoring over the authenticated session. CPU load averages come from `/proc/loadavg`, the logical CPU count from `nproc`, memory from `/proc/meminfo`, filesystem usage from `df -Pk`, and interface counters from `/proc/net/dev`. Every command is a fixed read-only template with a 32 KiB output bound and the connection manager's command timeout.

Each metric group carries its own sample timestamp and an explicit status: `available`, `unsupported`, or `unavailable`. A transport failure aborts the refresh; the frontend keeps the last successful snapshot, marks it stale, and keeps showing its original sample time. Polling uses the monitoring interval from Settings, clamped to a five-second minimum, and stops whenever no server is selected or the session is disconnected. Manual refresh always remains available while connected.

## Systemd service discovery

The Services page discovers service units that actually exist on the target machine instead of assuming a fixed package list. One fixed read-only template runs over the authenticated session: `systemctl list-units --type=service --all --no-pager --no-legend --full`. Every returned unit is parsed generically, so common units such as nginx, apache2, mysql, postgresql, redis, supervisor, versioned PHP-FPM units, and unlisted custom services all appear when the server reports them. ServerDesk never adds a common name that systemd did not return.

The command has a 64 KiB output bound, a 10 second duration bound, and a 512 unit parse limit. Active, inactive, failed, and unknown states stay distinct; the raw systemd active and sub states are retained for units such as `activating` that do not fit those four labels.

The typed `get_systemd_services` Tauri command returns an `available`, `unsupported`, or `unavailable` discovery result. Systems without systemd return `unsupported` with a bounded explanation rather than an empty successful list. The frontend calls it through `src/services/services.ts`, stores per-server results in `useServiceStore`, and renders the unit list with inspect, start, stop, and restart actions.

The Services table is built for large unit lists: free-text filtering covers unit names, descriptions, load state, and raw systemd states; state filters show active, inactive, failed, and unknown counts; columns sort by unit, state severity, or description; and results paginate 25 units at a time. Every state is shown with an icon and text label as well as color, so color never carries meaning alone. Stop and restart require a confirmation dialog that names both the server and the unit.

## Verified service actions

Status inspection, start, stop, and restart use fixed command templates and a validated unit name. Actions run through non-interactive, narrowly scoped sudo: `/usr/bin/sudo -n -- /usr/bin/systemctl <verb> -- <unit>`, and status uses the same non-interactive path with `systemctl show`. The SSH account is expected to be a dedicated non-root account. ServerDesk never prompts for a sudo password and never assumes unrestricted sudo. Stop and restart are rejected by the Rust backend unless the request carries explicit confirmation, and the confirmation dialog names the server, unit, and operation. If sudo denies an action, the result includes the exact `NOPASSWD` sudoers rule for that single unit, plus the remote stdout, stderr, and exit status.

After every action the unit is queried again with `systemctl show`, then the unit list is refreshed so counts, unaffected rows, and the sample time stay in sync. The UI updates the row from the verified re-query and labels the result as verified, mismatched, or unverified; a zero exit status alone is never treated as proof that the service reached the requested state. The result panel shows the exact command, exit status, duration, stdout, and stderr in one terminal-like panel whose Copy action copies the whole transcript. Action output, exit code, and duration are returned to the UI and kept in memory only. They are never written to SQLite or logs. Invalid unit names, including shell operators, whitespace, slashes, option-like names, and non-`.service` units, are rejected before any remote command is sent. There is no generic shell command endpoint.

## Server overview

The default Overview workspace is the server health dashboard. It shows the selected server's connection state, hostname, distribution, kernel, architecture, and uptime alongside CPU load, memory, disk, and network cards. Every metric carries a unit and a sample time, and the dashboard summary reports whether the latest data is live, stale, or unavailable.

Manual refresh updates identity and monitoring in place without a page reload. Monitoring polling uses the interval configured in Settings, clamped to a five-second minimum, and only runs while the selected session is connected. Identity refreshes on demand. A disconnected server never shows live values: the dashboard either shows an explicit unavailable state or retains the last successful sample with its original timestamp and a visible stale marker.

## Laravel project detection

The Projects page never scans a server automatically. The user enters an absolute remote path, browses only its immediate child directories, or explicitly scans that directory and at most one level of subdirectories. The Rust path normalizer collapses duplicate and trailing slashes, removes `.` segments, and rejects relative paths, parent (`..`) segments, control characters, and oversized paths before any remote command is sent. Each path is shell-quoted as a single argument and displayed before scan or navigation actions.

A directory is accepted as a Laravel project only when `artisan`, `composer.json`, `bootstrap/`, and `storage/` are all present. Non-Laravel directories are rejected and never create a project record. Accepted projects are stored locally with the server ID, name, normalized remote path, type, detection time, optional Laravel and PHP versions, and optional queue and scheduler hints.

Version and hint detection is bounded and non-secret:

- The Laravel framework version comes from `vendor/composer/installed.json`; no project code is executed.
- The PHP CLI version comes from the fixed `php --version` command and is reported as unavailable when the command is missing or its output cannot be parsed.
- The queue hint extracts only `QUEUE_CONNECTION` from `.env` when readable, and notes Laravel Horizon when Composer reports it.
- The scheduler hint notes a `schedule:run` cron entry that references the project path, or reports scheduler files without claiming a cron entry exists.

Project records are upserted by the unique `(server_id, remote_path)` pair, so rescanning updates metadata and detection time instead of creating duplicates. Saved projects and their SQLite records survive application restarts. The dashboard shows recent recorded operations with success, failure, or unknown status, the Laravel log path, quick links to the terminal at the project path and the log viewer, and the stored queue and scheduler hints.

## Queue and Supervisor status

The project dashboard can inspect Laravel queue workers and Supervisor without changing either. Detection is read-only and uses fixed commands: a `command -v supervisorctl`/`supervisord` presence probe, `supervisorctl status`, and the server-wide `ps -eo pid=,etimes=,args=` process list. ServerDesk never reads Supervisor configuration, rewrites it, starts or stops Supervisor, or installs packages.

The typed result reports Supervisor as `available`, `missing`, or `unavailable`, and queue workers as `available` or `unavailable`. Program names and states come only from Supervisor; a missing or unreadable result is never replaced with guessed names. Queue worker detection recognizes `queue:work`, `queue:listen`, and Horizon processes and keeps only the PID, elapsed time, worker kind, and bounded queue/connection options. Raw process arguments are not stored or returned, so credentials in a command line cannot leak into the UI or history.

The panel states that a Laravel `queue:restart` is a graceful application-level restart, not a Supervisor or systemd restart. Queue and Supervisor data is a sample, not configuration, and is kept in memory only.

## Safe Artisan actions

The project dashboard exposes a closed allowlist of fixed commands: `about`, `route:list`, `config:cache`, `config:clear`, `cache:clear`, `optimize:clear`, `queue:restart`, and `migrate:status`. The frontend never sends command text or a project path for an action: it sends a typed action name and the saved project ID. The Rust backend looks up the stored path, validates it again, and builds the command as `cd -- '<project path>' && php artisan <fixed command>`.

Every card shows the exact command and working directory before a button can run it. Cache clears, config cache changes, optimize, and queue restart require an explicit confirmation dialog that repeats the server, target path, and command. The backend independently rejects a destructive action unless the request includes confirmation, so bypassing the UI cannot run one. `queue:restart` is described as a graceful Laravel worker restart, not a Supervisor or systemd restart.

Action output is bounded to 64 KiB per command and each stream is shortened at a display limit while the exit status, stdout, stderr, and duration stay visible. ANSI escapes are stripped. Environment-style lines, secret assignments, bearer tokens, credentialed URLs, common token prefixes, and private-key blocks are redacted by the central redactor before output reaches the UI or local history. Operation history stores only the fixed action name and a safe summary with the target path and exit status; it never stores raw output or `.env` values.

## Operation history

Service actions, Laravel Artisan actions, SFTP transfers and mutations, and the fixed `hostname` and `uptime` commands record one local history row. Each row stores the server reference, operation category, a fixed safe summary, the project or remote path when applicable, start time, duration, exit status, a result of `success`, `failure`, `cancelled`, or `unknown`, and a short sanitized output summary. A service action is only recorded as `success` when its post-action status query proves the expected state; an unverified or mismatched state is recorded as `unknown` or `failure`. Destructive Artisan, service, and file operations that lose the connection before returning are recorded as `unknown` with guidance to verify the remote state before retrying, never as a silent failure or success. Terminal keystrokes and raw command output are never recorded.

History text is sanitized again before it is written. Password, passphrase, secret, token, API key, private key, access key, credential, cookie, session, and authorization assignments are redacted; bearer tokens, credentialed URLs, common token prefixes, ANSI escapes, and private-key blocks are removed; and every dotenv-style line is replaced wholesale. The stored output summary is capped at 360 characters, and the safe summary and path fields are bounded as well.

The Command History page filters by server, category, result, and date range. A details dialog shows only the sanitized fields, never raw stdout or stderr. The local database keeps the newest 1,000 rows and prunes older rows automatically whenever a new operation is recorded. Users can clear the remaining rows from the page only after an explicit confirmation dialog; clearing never sends a remote command. History is stored in SQLite, so it survives application restarts.

## Bounded log viewer

The Logs page reads only a bounded tail from approved remote log locations. System logs come from a fixed allowlist of common paths such as `/var/log/syslog`, `/var/log/auth.log`, Nginx and Apache access and error logs, MySQL, Redis, and UFW. A Laravel log must be a `.log` file directly inside a `storage/logs` directory. Relative paths, parent segments, control characters, arbitrary files, and symlinks that resolve outside those locations are rejected before any remote command is sent.

Each request uses one fixed shell template with a quoted path and `tail -c` bounded to 64 KiB. The Rust side then keeps at most 2,000 lines (200 by default) and reports a typed `available`, `missing`, `not_file`, `unreadable`, or `unavailable` result. The result carries the requested and resolved remote path, retrieval time, duration, file size, returned line and byte counts, truncation flags, and whether the file rotated during the read. A partial first line from a byte-bounded tail is dropped and disclosed instead of displayed as corrupted text.

The page shows the remote path and retrieval time beside every tail, supports refresh, and filters the already retrieved text locally without another remote read. Missing files, permission errors, non-regular files, rotation, and truncation are shown as explicit alerts. Known secret patterns are redacted from the returned tail, and a `Secrets redacted` badge discloses when that happened. ServerDesk does not use sudo to read logs, and raw log content is kept in memory only.
