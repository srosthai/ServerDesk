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
  `./ServerDesk_0.1.0_amd64.AppImage --appimage-extract-and-run`.
- Confirm the file is executable: `chmod +x ServerDesk_0.1.0_amd64.AppImage`.
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
