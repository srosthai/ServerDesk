# Troubleshooting

This guide covers the errors most often seen while connecting, reading logs and
files, running service actions, and installing the desktop application. Start
with the exact error code or message shown in ServerDesk; do not retry a failed
operation against a remote server until you have checked whether the previous
attempt changed anything.

ServerDesk has no file logger, crash reporter, or telemetry. Error details are
shown in the interface and are not uploaded. When reporting a problem, include
the ServerDesk version from `serverdesk --version`, your operating system and
package type, the error code, and the visible sanitized message. Never include a
password, private key, passphrase, API token, or `.env` value.

## Connection problems

### The hostname cannot be resolved

Error code: `dns`.

- Check the hostname for a typo, or use the server's IP address.
- Confirm that DNS works from the same computer:
  `getent hosts server.example.com`.
- Check VPN, split-DNS, `/etc/hosts`, and local resolver settings. ServerDesk
  uses the operating system resolver.
- A hostname can resolve to an IPv6 address that your network cannot reach.
  Test the IPv4 address directly to narrow this down.

### The connection times out

Error code: `timeout`.

- Confirm the SSH port. The default is 22, but providers and hardened servers
  often use another port.
- Check the cloud firewall or security group and the server firewall. Allow
  your current public IP, not the whole internet, where possible.
- Confirm that `sshd` is running and listening:
  `sudo systemctl status ssh` or `sudo systemctl status sshd`, and
  `sudo ss -ltnp | grep ':22'`.
- Check `fail2ban`, provider-level connection filtering, and whether a VPN is
  required.
- A hostname with several addresses makes ServerDesk try them in order. A
  blackholed IPv6 address can delay the IPv4 attempt; test the IPv4 address
  directly.

### Authentication failed

Error code: `authentication`.

- Verify the username, not just the display name of the profile.
- For a password, confirm the password works in a terminal. If the server has
  disabled password authentication, use a key.
- For a key, confirm that the public key is in the account's
  `~/.ssh/authorized_keys` and that the file permissions are correct. See
  [Prepare a least-privilege SSH account](ssh-setup.md#add-an-ssh-key).
- Confirm the account is not locked, expired, or restricted by `AllowUsers`,
  `AllowGroups`, PAM, or an SSH certificate policy.
- Check the server's authentication log:
  `sudo journalctl -u ssh -n 50` or `/var/log/auth.log`.

### The network connection was lost

Error code: `network`.

The SSH session died while an operation was running. This can happen after a
VPN drop, a server reboot, an idle NAT timeout, or a network change. Reconnect
and verify the remote state before retrying a command that changes anything.
ServerDesk does not replay commands automatically.

### The app says there is no connection

Error code: `connection`.

The operation needs a live managed session. Return to the server and select
**Connect**. A saved profile does not keep a session open after a restart.

### The command times out but the session is healthy

Error code: `timeout`.

The remote command exceeded its time bound. The session may still be connected.
Read-only refreshes can be retried. For a service or Artisan action, check the
remote state first because the command may have completed after the client
stopped waiting. ServerDesk reports the result as `unknown` rather than claiming
success.

### Services are unavailable or systemd is not running

Some containers and WSL environments do not run systemd as PID 1. ServerDesk
returns an `unsupported` result instead of inventing a service list. Use a
systemd-based host, or manage services outside ServerDesk in that environment.

## Host key problems

### The server key is unknown on first connection

This is expected for a new profile. Compare the key type and SHA256 fingerprint
with the server through the provider console or another independent channel,
then select **Trust and connect**. Reject the key if it does not match. See
[Host key verification and recovery](host-keys.md).

### The SSH host key changed

Error code: `host_key`.

ServerDesk blocked the connection before authentication. The saved fingerprint
was not changed. This can be a legitimate rebuild, a DNS or load-balancer
change, or an interception attempt.

1. Do not remove the saved trust yet.
2. Verify the new fingerprint through the provider console or another trusted
   channel.
3. If the change is intentional, remove all trusted fingerprints for the
   profile, reconnect, and verify the new key.
4. If the change is not intentional, investigate before reconnecting further.

The exact recovery procedure is in
[Host key verification and recovery](host-keys.md#recover-from-an-intentional-key-change).

### I need to start over with a server's host key

Open **Servers**, select the fingerprint icon for the profile (**Trusted host
keys**), remove the entries, and connect again. This changes only local trust.
If the server itself was rebuilt, verify its new key first.

## Permission and sudo problems

### Permission denied on a file or directory

Error code: `permission`.

- Check the Unix owner, group, and mode with
  `namei -l /path/to/file`. Every parent directory needs execute permission.
- Test as the SSH account:
  `sudo -u serverdesk test -r /path && echo readable`.
- SELinux or AppArmor can deny access even when Unix modes look correct. Check
  the audit log on the server if enforcing.
- The File Manager only operates inside the selected root. A symlink that
  resolves outside the canonical root is blocked. Choose a root that contains
  the files you need.
- ServerDesk never elevates for SFTP or logs. Fix account permissions rather
  than granting sudo for file access.

### Logs show as missing or unreadable

- ServerDesk reads only a fixed list of system log paths and `.log` files
  directly inside a Laravel `storage/logs` directory. Arbitrary paths are
  rejected before a remote command is sent.
- Check that the file exists and is readable by the SSH account:
  `sudo -u serverdesk test -r /var/log/auth.log && echo readable`.
- On Debian and Ubuntu, add the account to the log group, then reconnect:
  `sudo usermod -aG adm serverdesk`.
- Check for rotation. A file can disappear between discovery and reading.
- A missing log is reported as `missing`; an unreadable one as `unreadable`.
  ServerDesk does not show an empty successful tail for either case.

### A service action says sudo requires a password

The account has no matching `NOPASSWD` rule for that exact command. ServerDesk
never prompts for a sudo password and never falls back to a shell.

1. Confirm the binaries are at `/usr/bin/sudo` and `/usr/bin/systemctl`:
   `command -v sudo systemctl`.
2. Compare the rule with
   [Grant scoped sudo for services](ssh-setup.md#grant-scoped-sudo-for-services).
   The arguments must match exactly, including `--`, `--no-pager`, and every
   `--property` flag.
3. Validate with
   `sudo visudo -cf /etc/sudoers.d/serverdesk`, then check the mode:
   `sudo stat -c '%a %U:%G' /etc/sudoers.d/serverdesk` should show
   `440 root:root`.
4. Test the read-only command directly with
   `sudo -u serverdesk sudo -n -- /usr/bin/systemctl show ...`.
5. Reconnect the profile after changing group membership or SSH configuration.

If only status inspection is needed, grant only the `show` line. Start, stop,
and restart will continue to fail with a clear permission error.

### A service action ran but the state is wrong

ServerDesk re-queries the unit with `systemctl show` after every action. A zero
exit status is not treated as proof that the service reached the requested
state. Review the result panel:

- `Verified`: the observed state matches the action.
- `Mismatched`: the command exited successfully but the observed state
  contradicts it.
- `Unverified`: the follow-up query failed.

Check the unit's dependencies, start limits, and service logs with
`sudo journalctl -u <unit>`. Retry only after deciding whether the previous
action changed the remote state.

### The unit is missing or cannot be inspected

- Confirm the exact unit name on the server:
  `systemctl list-unit-files --type=service`.
- ServerDesk accepts only simple `.service` names made of ASCII letters, digits,
  `:`, `_`, `.`, `@`, and `-`. Shell syntax, whitespace, slashes, and option-like
  names are rejected.
- A missing sudo rule produces a permission error, not an empty service list.

## Database Manager problems

The Database Manager adds a database login and an SSH tunnel on top of the
normal server connection. The full setup and recovery guide is
[database-manager.md](database-manager.md); the checks below cover the most
common failure groups.

### The database tunnel cannot be opened

Error codes: `tunnel`, `connection`, `timeout`, or `conflict`.

- Connect the server profile and verify its host key first. A database tunnel
  needs an authenticated SSH session.
- Test the endpoint from the SSH server, not from your desktop:
  `ss -ltn | grep ':5432'` and
  `psql -h 127.0.0.1 -p 5432 -U serverdesk_ro -d app -c 'SELECT 1'`.
- Use `127.0.0.1` only when the database is on the SSH server. For a separate
  host, use an address reachable from that server; a container name or a
  container-only address normally is not.
- Check forwarding policy with
  `sshd -T | grep -Ei 'allowtcpforwarding|permitopen'` on the SSH server.
- Close unused database tabs if 16 tunnels are already active. A `conflict`
  also means the profile is already opening or using a different endpoint.

### The database login fails

Error code: `database_auth` (the UI may label it as an authentication failure).

- Confirm the database username, password, and database name with the vendor
  client from the SSH server. The SSH username is not the database username.
- Check the account's source host. The database sees the SSH server's address,
  not this computer's address. For MySQL, grant the account for
  `127.0.0.1` only when the database is on the SSH server.
- For PostgreSQL, confirm the target database exists and that `pg_hba.conf`
  has a matching TCP password rule before any broader rule. Reload PostgreSQL
  after changing it.
- Edit the saved profile and re-enter the password after a server-side password
  change. A locked OS keyring also prevents the saved credential from loading.

### A database connection or query times out

Error code: `timeout`.

- Tunnel setup and database connect are bounded at 10 seconds. Check server
  load, firewall rules, and whether the database is running.
- SQL and row reads default to 10 seconds and allow at most 30 seconds. Raise
  the timeout in the SQL tab for a slow statement; metadata and Data-tab reads
  do not offer a per-request timeout.
- Locks, long DDL, and a saturated database can exceed the bound. Use the
  server's process list to inspect them.
- A timed-out or cancelled SQL request closes the database session. Reconnect
  the profile before running another statement.

### A database action is denied

Error code: `permission`.

- Check whether the profile is read-only. A writable database account is still
  required after the profile flag is off.
- Inspect the account's grants with `SHOW GRANTS` on MySQL or
  `has_table_privilege` / `\dp` on PostgreSQL.
- PostgreSQL object changes require ownership or membership in the owning role.
- Listing other users' sessions needs `PROCESS` (MySQL) or `pg_monitor`
  (PostgreSQL); cancelling them needs `CONNECTION_ADMIN` or
  `pg_signal_backend` respectively. ServerDesk never elevates the account.
- Some metadata is unavailable when the account cannot read it. It is not an
  empty database.

### Backup or restore tools are missing or incompatible

Error codes: `capability` or `not_found`.

- Install `mysqldump` and `mysql` (MySQL/MariaDB) or `pg_dump`, `pg_dumpall`,
  `pg_restore`, and `psql` (PostgreSQL) on the SSH server.
- Confirm the binaries are on the SSH account's `PATH` and that the client
  major version is at least the server major version.
- ServerDesk does not install packages or use a desktop-side client.
- Check that the destination directory exists and is writable and that the
  remote server has free space, including for a local download's temporary
  dump.
- If `mysqldump` 8 reports an unknown `COLUMN_STATISTICS` table, install a
  matching server-vendor client. ServerDesk does not pass
  `--column-statistics=0` because MariaDB clients reject it.

### A restore failed or may be incomplete

A failed, cancelled, or timed-out restore is not rolled back automatically.
`mayBeIncomplete: true` means objects or rows may already have been restored.

1. Do not retry blindly. Inspect the target database with the vendor client.
2. Use a disposable target or a fresh database for the next attempt.
3. Restore ownership or schema problems with a dump created using **Omit object
   ownership**, or use an archive restore with that option.
4. Restore a pre-restore snapshot or vendor backup if the target must be reset.
   ServerDesk has no rollback command.

A read-only profile is rejected before the restore password or tool is loaded.
Create and connect a writable profile first.

### TLS or certificate validation fails

- **Verify CA** uses ServerDesk's built-in public trust roots and has no custom
  CA file. A private CA fails even when the system `psql` or `mysql` client
  trusts it; use **Required** over the authenticated SSH tunnel or install a
  publicly trusted certificate.
- **Verify identity** checks the certificate against `127.0.0.1` for the app's
  driver. Most certificates do not include that address. Use **Verify CA** or
  **Required** unless the certificate deliberately matches the tunnel endpoint.
- Remote dump tools use the saved **Host**, so they can fail certificate
  validation when the profile connects successfully. See
  [TLS and SSL mode choices](database-manager.md#tls-and-ssl-mode-choices).

## Key and credential problems

### The private key is encrypted

Error code: `credential`.

Enter the key passphrase in the server profile and test again. ServerDesk stores
the key and passphrase together in the OS credential store, never in SQLite.

### The private key cannot be read

- If you entered a path, use an absolute path or a path beginning with `~/` that
  is readable by your desktop user. ServerDesk reads the file locally before
  connecting.
- Check the file mode; a key readable only by root cannot be used by your desktop
  session.
- Paste the key contents instead of a path if the file is in a location your
  desktop session cannot read.
- Convert an unsupported legacy format to OpenSSH with
  `puttygen key.ppk -O private-openssh -o key.openssh` or
  `ssh-keygen -p -f keyfile`, then try again.

### The passphrase is rejected

Error code: `credential`.

Check the passphrase in a terminal first:

```bash
ssh-keygen -y -f ~/.ssh/serverdesk_ed25519
```

If the passphrase is wrong, update it in the profile. Re-enter both the key and
passphrase if the saved credential is stale.

### The OS credential store is unavailable

Error code: `credential`, with a message such as
`The OS credential store is unavailable.`

- **Linux:** ServerDesk needs Secret Service through a running desktop session.
  Unlock GNOME Keyring or KWallet, or start the keyring daemon for the session.
  A headless session without a Secret Service provider cannot save credentials.
- **macOS:** allow the Keychain prompt when ServerDesk saves or reads a
  credential. A denied prompt can leave the credential inaccessible.
- **Windows:** confirm Credential Manager is available and that a security
  policy has not disabled generic credentials for your account.
- After an OS keychain reset or profile migration, re-enter the secret in the
  server profile. SQLite contains only a reference, not a recoverable copy.

### The saved credential is missing

Error code: `not_found` or `credential`.

The SQLite record still exists but the matching keyring entry does not. This can
happen after a manual keychain cleanup or restoring the app-data directory on a
new machine. Edit the server profile and enter the password or private key
again, then test the connection.

## Update problems

### The update check says the device is offline

ServerDesk skips startup checks while the operating system reports no network
connection. Manual checks show an offline error instead of silently doing
nothing. Reconnect and select **Check for updates** again.

### The update server cannot be reached

A proxy, firewall, DNS failure, or missing release manifest can block the check.
Confirm that
`https://github.com/srosthai/ServerDesk/releases/latest/download/latest.json`
opens in a browser. If GitHub is reachable but ServerDesk is not, check the
system proxy settings. Install the release manually if the update check keeps
failing; the current installation is not changed by a failed check.

### The update signature was rejected

ServerDesk refuses an update whose minisign signature is missing, corrupt, or
signed by a different key. Do not bypass this prompt. Download the installer
from the official GitHub Releases page and verify `SHA256SUMS`. If a release
continues to fail signature verification, report it rather than installing it.

### The update is not permitted

The desktop capability may be missing from the build, or the OS may block the
replacement. Confirm that the installed package came from this repository and
that the app is running as your normal desktop user. On Linux, DEB and RPM
installs intentionally do not self-update; use the package manager.

### The update installed but ServerDesk did not reopen

Quit ServerDesk completely and open it again. If the version still does not
change, reinstall from the latest release. On AppImage, check that the
AppImage file and its directory are writable by your user. On macOS, confirm
that the app is Developer ID signed and notarized; Gatekeeper can block an
unsigned replacement.

### A DEB or RPM update is available but there is no install button

This is intentional. Package-manager installs cannot be replaced safely behind
the package manager's back. Select **Open download page**, then install the
`.deb` or `.rpm` with `apt` or `dnf`. The full platform matrix is in
[updates.md](updates.md#platform-support).

## Packaging and startup problems

### The Linux app fails to start because WebKitGTK is missing

Install the WebKitGTK 4.1 runtime for your distribution. On Debian and Ubuntu:

```bash
sudo apt install libwebkit2gtk-4.1-0
```

The package name can differ on other distributions. The DEB and RPM declare
their WebKitGTK 4.1 and GTK 3 dependencies, so a normal package-manager install
handles this automatically. Older distributions that provide only WebKitGTK 4.0
are not supported.

### The AppImage does not start

- Install FUSE 2. On Ubuntu 22.04, `sudo apt install libfuse2`.
- If FUSE is unavailable, use
  `./ServerDesk_0.1.1_amd64.AppImage --appimage-extract-and-run`.
- Confirm the file is executable: `chmod +x ServerDesk_0.1.1_amd64.AppImage`.
- The AppImage is not installed, so no package manager resolves missing system
  libraries. Check the terminal output for the missing library.

### The DEB was built on the wrong distribution

A DEB built locally on Fedora can require a newer glibc than Ubuntu 22.04
provides. It may install with a `GLIBC_2.39` weak-version warning. Release DEBs
are built on Ubuntu 22.04 by the release workflow. Use the release artifact for
Ubuntu 22.04 and Debian 12, or rebuild on the oldest distribution you support.
Check the local glibc with `ldd --version`.

### macOS blocks the app

An unsigned or unnotarized build is blocked by Gatekeeper. Use a signed release
from the repository's Releases page. If you built the app locally for testing,
you can open it once through Finder's **Open** context menu, or allow it in
**System Settings > Privacy & Security**. Do not disable Gatekeeper globally.
Verify the build's signature with:

```bash
codesign --verify --deep --strict --verbose=2 "ServerDesk.app"
spctl --assess --type execute --verbose=4 "ServerDesk.app"
```

A release without the Apple signing secrets is intentionally unsigned and is
for internal testing only.

### Windows shows a SmartScreen warning

Windows builds are currently unsigned, so SmartScreen may warn about an unknown
publisher. Confirm that the installer came from this repository's Releases page.
If you trust the source, use **More info > Run anyway**. Do not bypass the
warning for an installer from another source. Authenticode signing is not
configured yet.

### The Windows installer cannot download WebView2

The per-user installer uses a WebView2 bootstrapper and needs network access on
the first install. Install the Microsoft Evergreen WebView2 Runtime manually,
then rerun the installer. Windows 10 and 11 usually include WebView2 already.

### Cleanup could not remove credentials

`--purge-data` is fail-safe: if it cannot delete every credential, it keeps the
SQLite database and credential index so you can retry. Log in to the desktop
session that owns the credentials and run the cleanup again:

```bash
serverdesk --purge-data
```

On macOS, run the binary inside the application bundle. On Windows, run
`serverdesk.exe --purge-data` from the install directory. On Linux, make sure
the Secret Service session is unlocked. See
[packaging.md](packaging.md#uninstall-and-credential-cleanup) for manual
credential-manager fallbacks.

### The window is blank on Linux

This is usually a WebKitGTK rendering or GPU-driver problem rather than an SSH
problem. Try starting the application with compositing disabled:

```bash
WEBKIT_DISABLE_COMPOSITING_MODE=1 serverdesk
```

If that helps, update or change the graphics driver and report the distribution,
GPU, and WebKitGTK version. Do not run the application as root to work around a
display problem.

### Where are the application logs?

There is no application log file. ServerDesk deliberately has no file logger,
crash reporter, or diagnostic-bundle writer because those can capture secrets.
Use the error message shown in the interface and the server's own system logs.
If you need to inspect the local database, copy it only while the application is
closed; it contains metadata and sanitized history, never plaintext secrets.

## Getting more help

When opening an issue, include:

- ServerDesk version (`serverdesk --version`) and installation type.
- Operating system and version.
- SSH server distribution and version, if known.
- The error code and the full sanitized message.
- Whether the problem happens on connect, a specific action, or application
  startup.

Remove secrets from any pasted output. ServerDesk redacts known secret patterns,
but redaction is best-effort.
