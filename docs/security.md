# Security and privacy model

ServerDesk is a local-first desktop application. It has no hosted account,
cloud backend, server-side agent, analytics service, crash reporter, or
auto-updater. The local desktop session is trusted; the SSH server and
everything it returns are untrusted.

## Privacy summary

### Data kept on this computer

ServerDesk stores its working data locally in SQLite:

- server profiles: name, host, port, username, authentication type, tags, and
  notes;
- preferences: theme, monitoring interval, and the last active server;
- detected Laravel project metadata;
- trusted SSH host-key fingerprints;
- bounded, sanitized operation history.

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

### No telemetry

ServerDesk does not send usage data, analytics, crash reports, diagnostics,
server inventory, or credentials to the project or any third party. It has no
telemetry setting to disable because there is no telemetry client.

The application's only network activity is the SSH connection to a server
profile you explicitly test or connect to, plus the DNS lookup needed to resolve
that host. ServerDesk has no update checker, so it does not contact GitHub or any
release service. The optional opener plugin is present in the desktop
capability set but the current interface does not use it to load remote content;
the webview loads local application assets.

The remote SSH server necessarily sees the connection, the fixed commands
ServerDesk runs, and any terminal input you type. That server and its
administrators are outside ServerDesk's trust boundary. Log and command output
is treated as untrusted input and is redacted on a best-effort basis.

### Removing local data

- Deleting a server profile removes the profile, its credential reference, its
  trusted host keys, and its linked project records. Operation history keeps its
  sanitized rows but clears the server reference.
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
