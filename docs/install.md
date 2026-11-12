# Install and first run

This guide installs ServerDesk, adds a server profile, and makes the first
connection. ServerDesk stores data locally and requires no account or hosted
service.

Release builds are produced by the repository's release workflow and attached
to GitHub Releases. Download the asset for your platform. If the Releases page
has no published build yet, build from source with
[development.md](development.md) and [packaging.md](packaging.md).

The examples below use version `0.1.0`. Substitute the version and architecture
in the file name you downloaded.

## Requirements

- **Linux:** WebKitGTK 4.1, GTK 3, and a running Secret Service provider such as
  GNOME Keyring or KWallet. The release packages are x86_64 and target Ubuntu
  22.04+, Debian 12+, and Fedora 39+. AppImages need FUSE 2, or the
  extract-and-run flag described below.
- **macOS:** 10.15 or newer. The DMG contains a universal application for Intel
  and Apple silicon.
- **Windows:** 64-bit Windows 10 or newer. The per-user installer uses WebView2
  and downloads the runtime when it is missing.

A desktop Linux session without an unlocked Secret Service provider can display
data, but it cannot save or read SSH credentials. Run ServerDesk as your normal
desktop user, not with `sudo` or as root.

## Command-line install

The release also ships `install.sh` for Linux/macOS and `install.ps1` for
Windows. Both download the matching release asset, verify it against
`SHA256SUMS`, and install without requiring an administrator account. Download
and inspect the script before running it.

### Linux and macOS

```bash
curl -fsSLO https://github.com/srosthai/ServerDesk/releases/latest/download/install.sh
less install.sh
bash install.sh --version v0.1.0
```

On Linux the AppImage is installed to `~/.local/bin/serverdesk`. Add
`~/.local/bin` to `PATH` if needed. On macOS the application is installed to
`~/Applications/ServerDesk.app`.

### Windows

```powershell
Invoke-WebRequest -Uri https://github.com/srosthai/ServerDesk/releases/latest/download/install.ps1 -OutFile install.ps1
Get-Content .\install.ps1
powershell -ExecutionPolicy Bypass -File .\install.ps1 -Version v0.1.0
```

The NSIS installer runs per-user. Unsigned Windows and macOS builds may trigger
SmartScreen or Gatekeeper. Use a signed release when available, or allow the app
through the operating system security prompt.

## Linux

### Debian and Ubuntu (DEB)

```bash
cd ~/Downloads
sudo apt install ./ServerDesk_0.1.0_amd64.deb
```

`apt` installs the WebKitGTK 4.1 and GTK 3 runtime dependencies from the
configured repositories. Launch ServerDesk from the application menu or run:

```bash
serverdesk
```

The package name is `server-desk`; the installed binary is
`/usr/bin/serverdesk`.

To upgrade, install the newer DEB over the existing package. To uninstall:

```bash
sudo apt remove server-desk
```

The package pre-remove script removes saved credentials and local data for users
who have an active desktop session. See [Uninstall and cleanup](#uninstall-and-cleanup)
when that cleanup cannot run.

### Fedora (RPM)

```bash
cd ~/Downloads
sudo dnf install ./ServerDesk-0.1.0-1.x86_64.rpm
```

Launch ServerDesk from the application menu or run `serverdesk`. The package is
named `server-desk`, and the binary is `/usr/bin/serverdesk`.

To upgrade, install the newer RPM over the existing package. To uninstall:

```bash
sudo dnf remove server-desk
```

### AppImage

The AppImage is portable and is not installed system-wide:

```bash
cd ~/Downloads
chmod +x ServerDesk_0.1.0_amd64.AppImage
./ServerDesk_0.1.0_amd64.AppImage
```

AppImages need FUSE 2. On Ubuntu 22.04, install it with
`sudo apt install libfuse2`. If FUSE is unavailable, run the AppImage with:

```bash
./ServerDesk_0.1.0_amd64.AppImage --appimage-extract-and-run
```

Quit ServerDesk and run the AppImage with `--purge-data` before deleting the
file if you want to remove credentials and local data.

## macOS

1. Open the downloaded `ServerDesk_0.1.0_universal.dmg`.
2. Drag `ServerDesk.app` into `Applications`.
3. Eject the DMG and launch ServerDesk from `Applications`.

Release builds are signed and notarized when the repository's Apple signing
secrets are configured. An unsigned build is for local testing only; Gatekeeper
may block it. See [Troubleshooting](troubleshooting.md#macos-blocks-the-app).

The DMG has no uninstaller hook. Before deleting the application, quit
ServerDesk and run:

```bash
"/Applications/ServerDesk.app/Contents/MacOS/serverdesk" --purge-data
```

Then move `ServerDesk.app` to the Trash.

## Windows

Run the downloaded `*-setup.exe`. The NSIS installer installs ServerDesk for the
current user and does not require administrator rights. It downloads the
WebView2 runtime when it is missing, so the first install may need an internet
connection.

If the installer is unsigned, Windows SmartScreen may show a warning. Only
continue when you downloaded the installer from this repository's Releases page
and trust the build. See [Troubleshooting](troubleshooting.md#windows-shows-a-smartscreen-warning).

Uninstall from **Settings > Apps > Installed apps > ServerDesk**. The uninstaller
runs `serverdesk.exe --purge-data` before removing program files. If cleanup
fails, it keeps the local data directory so the purge can be retried.

## Updates

Installed builds check for signed updates on startup and when you select **Check for updates** in **Settings**. Successful checks are throttled to once every six hours; the update badge in the topbar opens the update panel when a newer version is available.

- **AppImage:** ServerDesk can download, verify, and install an AppImage update in place, then relaunch.
- **DEB and RPM:** ServerDesk detects available versions but does not replace package-manager installs. Use `apt` or `dnf` with the new package from [GitHub Releases](https://github.com/srosthai/ServerDesk/releases).
- **Windows:** The NSIS installer supports in-app updates. The app may close while the installer runs.
- **macOS:** The app bundle supports in-app updates when the release is signed and notarized.

Browser previews and development builds never check for updates. The signing key, `latest.json`, platform limitations, and rollback process are documented in [updates.md](updates.md).

## Add and connect to a server

1. Start ServerDesk. There is no sign-in.
2. Open **Servers** and select **Add server**.
3. Enter a display name, the hostname or IP address, the SSH port (normally 22),
   and the SSH username.
4. Choose **Password** or **Private key** authentication.
   - For a password, enter the SSH password.
   - For a private key, paste an OpenSSH, PEM, or PuTTY PPK key, or enter the
     path to a key file. Add the key passphrase when the key is encrypted.
5. Optionally select **Test connection**. The test resolves the host, verifies
   the host key, authenticates, and runs a remote check.
6. Select **Save server**, then connect from the Servers page or the Overview
   panel.

Secrets are written to the operating system credential store when you save.
SQLite stores only a credential reference. On Linux, the first save may prompt
your keyring to unlock.

### Verify the host key

On the first connection, ServerDesk shows the key type and SHA256 fingerprint
before it sends credentials. Compare the fingerprint with the server through an
independent channel, such as the hosting provider's console. If it matches,
select **Trust and connect**. If it does not, select **Reject key** and
investigate before connecting.

The terminal, file manager, services, projects, logs, and monitoring pages are
available after the connection is established. The exact verification and
recovery steps are in [host-keys.md](host-keys.md).

## Local data and credentials

ServerDesk keeps its SQLite database, preferences, project records, sanitized
history, and trusted host fingerprints in:

| Platform | Data directory                                                                     |
| -------- | ---------------------------------------------------------------------------------- |
| Linux    | `$XDG_DATA_HOME/com.serverdesk.app`, otherwise `~/.local/share/com.serverdesk.app` |
| macOS    | `~/Library/Application Support/com.serverdesk.app`                                 |
| Windows  | `%APPDATA%\com.serverdesk.app`                                                     |

SSH passwords and private keys are stored in the operating system credential
store: Secret Service on Linux, Keychain on macOS, and Credential Manager on
Windows. The database files are restricted to the owning user (`0600`) on Unix.
[packaging.md](packaging.md) lists the exact credential entry names and data
files.

Installed desktop builds can check GitHub Releases for signed updates. The check contacts only the release endpoint, never sends server inventory or credentials, and can be skipped while offline. There is no telemetry. See [updates.md](updates.md) for the update model and [security.md](security.md) for the privacy model. Install a newer release manually if an update check fails.

## Uninstall and cleanup

Removing the application files does not remove keyring entries. On Linux
packages and the Windows installer, the documented uninstall command or the
uninstaller runs the cleanup automatically. On macOS and AppImage, run
`--purge-data` first as shown above. `packaging.md` documents the fail-safe
behavior, manual credential-manager commands, and cleanup limitations.
