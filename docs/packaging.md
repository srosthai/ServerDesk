# Packaging and distribution

ServerDesk ships as native desktop installers built from this repository. The Tauri configuration is split by platform so each operating system receives only its own installable formats.

## Artifacts

| Platform               | Formats                            | Installer                           | Notes                                                                              |
| ---------------------- | ---------------------------------- | ----------------------------------- | ---------------------------------------------------------------------------------- |
| Windows                | NSIS `.exe`                        | Per-user install                    | Preferred format because its uninstall hook can remove Credential Manager entries. |
| macOS                  | `.dmg` containing `ServerDesk.app` | Drag to Applications                | Universal binary for Intel and Apple silicon.                                      |
| Debian/Ubuntu          | `.deb`, `.AppImage`                | Native package or portable AppImage | Built on Ubuntu 22.04 for glibc 2.35 compatibility.                                |
| Fedora/RHEL-compatible | `.rpm`, `.AppImage`                | Native package or portable AppImage | RPM requires RPM-based packaging tools on the build host.                          |

An MSI is intentionally not produced. The NSIS installer is the supported Windows format because it supports the uninstall hook used to remove credentials. Adding an MSI later requires an equivalent WiX custom action before it should be treated as a supported installer.

## Identity and version metadata

- Product name: `ServerDesk`
- Bundle identifier: `com.serverdesk.app`
- Version: `0.1.0`, kept identical in `package.json`, `src-tauri/Cargo.toml`, and `src-tauri/tauri.conf.json`
- Brand source: `assets/brand/serverdesk-logo.png`
- Icon-only crop: `assets/brand/serverdesk-mark-source.png`
- UI-ready sidebar mark: `src/assets/brand/serverdesk-mark.png`

`npm run check:versions` fails when the three version fields disagree, when the product name changes unexpectedly, when the identifier no longer matches `com.serverdesk.app`, when a bundle icon path is missing, when the 32/64/128/256/512 PNG sizes are wrong, or when a brand asset is missing. Release tags must be `v` followed by the package version; the release workflow passes the tag to `scripts/check-versions.mjs --tag`.

The sidebar reads the running application's version from Tauri's `get_app_info` command. Installer metadata and the packaged desktop entry use the same product name and version.

The application icon set is generated from the square, wordmark-free icon source `src-tauri/icons/icon-source.png`:

```bash
npx tauri icon src-tauri/icons/icon-source.png --output src-tauri/icons
```

The source itself is composed from the icon-only crop of the provided logo. The generated and committed icon files are:

- `src-tauri/icons/32x32.png`
- `src-tauri/icons/64x64.png`
- `src-tauri/icons/128x128.png`
- `src-tauri/icons/128x128@2x.png`
- `src-tauri/icons/icon.png`
- `src-tauri/icons/icon.ico`
- `src-tauri/icons/icon.icns`
- `src-tauri/icons/Square30x30Logo.png`
- `src-tauri/icons/Square44x44Logo.png`
- `src-tauri/icons/Square71x71Logo.png`
- `src-tauri/icons/Square89x89Logo.png`
- `src-tauri/icons/Square107x107Logo.png`
- `src-tauri/icons/Square142x142Logo.png`
- `src-tauri/icons/Square150x150Logo.png`
- `src-tauri/icons/Square284x284Logo.png`
- `src-tauri/icons/Square310x310Logo.png`
- `src-tauri/icons/StoreLogo.png`

`src/assets/brand/serverdesk-mark.png` is the UI-ready 256 px sidebar mark. It is not imported by the current UI; integration follows separately. Build output under `src-tauri/target` and `dist` is never committed.

## Local data and credentials

ServerDesk appends the bundle identifier to the operating system data directory:

| Platform | Application data directory                                                         |
| -------- | ---------------------------------------------------------------------------------- |
| Linux    | `$XDG_DATA_HOME/com.serverdesk.app`, otherwise `~/.local/share/com.serverdesk.app` |
| macOS    | `~/Library/Application Support/com.serverdesk.app`                                 |
| Windows  | `%APPDATA%\com.serverdesk.app`                                                     |

The directory contains:

- `serverdesk.sqlite3` and SQLite sidecar files: server metadata, preferences, projects, history, and credential references. On Unix the database files are restricted to the owning user (`0600`).
- `credential-references.json`: the references created by this installation, used only so an uninstall can find orphaned keyring entries. It contains references such as `serverdesk:ssh:<uuid>`, never passwords or private keys.

Secrets live in the operating system credential store:

| Platform | Credential store                                                | Entry identity                                                                  |
| -------- | --------------------------------------------------------------- | ------------------------------------------------------------------------------- |
| Linux    | Secret Service (GNOME Keyring, KWallet, or compatible provider) | service `com.serverdesk.app`, account `serverdesk:ssh:<uuid>`                   |
| macOS    | Keychain                                                        | generic password, service `com.serverdesk.app`, account `serverdesk:ssh:<uuid>` |
| Windows  | Credential Manager                                              | generic credential target `<reference>.com.serverdesk.app`                      |

SQLite stores only the reference. A desktop Linux session therefore needs a running Secret Service provider; a headless or locked-down session without one prevents saving or reading SSH credentials. That is an OS-session limitation, not a database limitation.

## Building locally

Prerequisites are Node.js 20 or newer, stable Rust, and the Tauri dependencies for the host platform. See the README for Fedora packages.

```bash
npm ci
npm run check:versions
npm run tauri build -- --bundles deb,rpm,appimage   # Linux
npm run tauri build -- --bundles nsis               # Windows
npm run tauri build -- --target universal-apple-darwin --bundles app,dmg  # macOS
```

Platform-specific bundle settings live in:

- `src-tauri/tauri.linux.conf.json`
- `src-tauri/tauri.macos.conf.json`
- `src-tauri/tauri.windows.conf.json`

Tauri merges the platform file into `src-tauri/tauri.conf.json` automatically. The Linux file selects DEB, RPM, and AppImage and installs the pre-remove cleanup script. The Windows file selects NSIS and installs the uninstall hooks. The macOS file selects the app bundle and DMG, enables the hardened runtime, and points at the entitlements file.

### Linux package notes

- DEB and RPM package names are derived from `ServerDesk` as `server-desk`.
- The binary is installed as `/usr/bin/serverdesk`.
- The DEB section is `admin`; the RPM release is `1`.
- Runtime dependencies are the standard Tauri WebKitGTK 4.1 stack. `webkit2gtk-4.1` is available on Ubuntu 22.04 and newer, Debian 12 and newer, and Fedora 39 and newer. Older distributions that only package WebKitGTK 4.0 are not supported by this build.
- AppImages need FUSE 2 to run normally. On systems without FUSE, use `./ServerDesk_0.1.0_amd64.AppImage --appimage-extract-and-run`.

## Uninstall and credential cleanup

Removing application files is not enough on any platform: keyring entries live outside the application directory. ServerDesk provides `--purge-data`, which reads the database and `credential-references.json`, deletes every stored credential through the OS credential store, and only then removes the data directory. The operation is fail-safe: if a credential cannot be deleted, the data directory is kept so the cleanup can be retried instead of losing the only record of the remaining entries.

- **Windows NSIS:** the uninstaller runs `serverdesk.exe --purge-data` before deleting program files. On success it also removes `%APPDATA%\com.serverdesk.app` and `%LOCALAPPDATA%\com.serverdesk.app`. If the purge fails, it warns and keeps the local data directory.
- **DEB and RPM:** the pre-remove script runs the purge as each user with a `~/.local/share/com.serverdesk.app` directory. Package upgrades are skipped so data survives upgrades. The script requires an active user D-Bus session for Secret Service; in a session without one, the purge fails safely and leaves the data directory in place.
- **macOS:** DMG installation has no uninstaller hook. Quit ServerDesk, run the packaged binary once with the purge flag, then move the application to Trash:

  ```bash
  "/Applications/ServerDesk.app/Contents/MacOS/serverdesk" --purge-data
  rm -rf "/Applications/ServerDesk.app"
  ```

- **AppImage:** quit ServerDesk and run the AppImage with `--purge-data` before deleting the AppImage file.

`--purge-data` writes a short result to standard error and exits non-zero if cleanup failed. Windows GUI builds have no console, so the exit status is what the NSIS hook uses.

The manual fallback for a credential whose reference was lost is the platform credential manager:

```bash
# Linux, with an unlocked Secret Service session
secret-tool search --all service com.serverdesk.app
# macOS
security find-generic-password -s com.serverdesk.app
# Windows
cmdkey /list | findstr com.serverdesk.app
```

## macOS signing and notarization

Unsigned macOS builds are useful for local testing but are blocked by Gatekeeper on other machines. A distributable DMG requires an Apple Developer Program membership, a Developer ID Application certificate, hardened runtime, and notarization.

Set these repository secrets for the release workflow:

| Secret                       | Purpose                                                                                |
| ---------------------------- | -------------------------------------------------------------------------------------- |
| `APPLE_CERTIFICATE`          | Base64-encoded `.p12` Developer ID Application certificate.                            |
| `APPLE_CERTIFICATE_PASSWORD` | Password used when exporting the `.p12`.                                               |
| `APPLE_SIGNING_IDENTITY`     | Full identity name, for example `Developer ID Application: Example Corp (TEAMID1234)`. |
| `APPLE_ID`                   | Apple ID used for notarization.                                                        |
| `APPLE_PASSWORD`             | App-specific password for that Apple ID, not the account password.                     |
| `APPLE_TEAM_ID`              | Apple Developer team identifier.                                                       |

For an App Store Connect API key instead of an Apple ID password, set `APPLE_API_ISSUER`, `APPLE_API_KEY`, and `APPLE_API_KEY_PATH`. Do not commit certificates, API keys, or passwords.

Tauri signs both the universal `.app` and the DMG when the signing identity and notarization variables are present. The hardened runtime is enabled in `tauri.macos.conf.json`, and `packaging/macos/entitlements.plist` grants the JavaScript JIT entitlement required by WKWebView. The app is not sandboxed and does not need App Sandbox network entitlements.

Local signing outline:

```bash
security import DeveloperID.p12 -k ~/Library/Keychains/login.keychain-db \
  -P "$P12_PASSWORD" -T /usr/bin/codesign
npm run tauri build -- --target universal-apple-darwin --bundles app,dmg
```

Verify a signed and notarized build:

```bash
codesign --verify --deep --strict --verbose=2 "ServerDesk.app"
spctl --assess --type execute --verbose=4 "ServerDesk.app"
xcrun stapler validate "ServerDesk_0.1.0_universal.dmg"
```

Without the secrets, the release workflow still builds an unsigned universal DMG. It is suitable for internal testing only and must not be presented as a normal end-user download. The workflow unsets empty signing variables before calling Tauri so a missing certificate is not treated as an empty-import request.

Windows Authenticode signing is not configured. Unsigned NSIS installers trigger SmartScreen warnings. Add a signing command and certificate secrets before calling the Windows artifact production-ready.

## Release workflow

`.github/workflows/release.yml` runs on tags matching `v*` and can be started manually with `workflow_dispatch`.

1. Bump the version in `package.json`, `src-tauri/Cargo.toml`, and `src-tauri/tauri.conf.json` together.
2. Run `npm run check` and `npm run check:versions`.
3. Merge the change through `dev`, `production`, and `main`.
4. Create and push a tag such as `v0.1.0` from `main`.
5. The workflow builds:
   - Ubuntu 22.04: DEB, RPM, and AppImage, then smoke-tests the release binary in Xvfb against a clean data directory.
   - macOS 15 Intel: universal `.app` and DMG, signed and notarized when the Apple secrets exist.
   - Windows: NSIS installer.
6. The publish job downloads all artifacts and creates a draft GitHub release. Review the artifacts, confirm signing status, and publish the draft manually.

The workflow uses `GITHUB_TOKEN` only to create the draft release. It does not publish automatically and it does not store build artifacts in the repository.

## Verification

### Native smoke test

The release binary supports an internal smoke-test mode. It starts the real Tauri application, creates the main window, opens SQLite in the resolved app-data directory, applies migrations, prints the schema version, and closes the window after two seconds:

```bash
./src-tauri/target/release/serverdesk --version
# ServerDesk 0.1.0

smoke_dir="$(mktemp -d)"
XDG_DATA_HOME="$smoke_dir" SERVERDESK_SMOKE_TEST=1 \
  ./src-tauri/target/release/serverdesk
test -f "$smoke_dir/com.serverdesk.app/serverdesk.sqlite3"
sqlite3 "$smoke_dir/com.serverdesk.app/serverdesk.sqlite3" 'PRAGMA user_version;'
```

Expected output includes `SERVERDESK_SMOKE_WINDOW_PRESENT=1` and schema version `4`. Use `xvfb-run -a` on a headless Linux host.

### Package content checks

```bash
dpkg-deb -c target/release/bundle/deb/*.deb
dpkg-deb -I target/release/bundle/deb/*.deb
rpm -qlp target/release/bundle/rpm/*.rpm
rpm -qip target/release/bundle/rpm/*.rpm
```

Check that the package installs `/usr/bin/serverdesk`, a desktop entry, and the icon, and that the version matches `0.1.0`.

### Credential cleanup check

Create a saved server with a password through the desktop app, then quit it and run:

```bash
/usr/bin/serverdesk --purge-data
test ! -e "${XDG_DATA_HOME:-$HOME/.local/share}/com.serverdesk.app"
secret-tool search --all service com.serverdesk.app
```

`secret-tool search` must return no ServerDesk entries. Repeat for an indexed entry left without a matching database row to cover the orphan case.

## Tested distributions and known limitations

Tested locally on Fedora Linux 44 x86_64:

- DEB, RPM, and AppImage packages build from this repository with Tauri 2.
- The release binary launches with a clean `XDG_DATA_HOME`, creates `com.serverdesk.app/serverdesk.sqlite3`, reaches database schema version 4, reports the main window as present, and exits with status 0 through the smoke-test hook. The SQLite file is mode `0600`.
- The RPM package was extracted to a temporary root and its `/usr/bin/serverdesk` binary passed the same smoke test with schema version 4.
- The AppImage was run with `--appimage-extract-and-run` and passed the same smoke test with schema version 4.
- Package metadata, version `0.1.0`, desktop entry contents, and icon files were inspected. The packaged 32x32 and 512x512 icon hashes match `src-tauri/icons/32x32.png` and `src-tauri/icons/icon.png`.
- The ignored OS credential-store round-trip test passed against the local Secret Service.
- `serverdesk --purge-data` removed two real Secret Service entries (one referenced by SQLite and one referenced only by the credential index), deleted the app-data directory, and exited 0. A second test forced a credential deletion failure and confirmed that the database and index are kept for a retry.

The DEB artifact is built by the same Tauri bundler on Linux. Installing it with `dpkg` was not possible on the Fedora test host without a Debian-based environment; CI builds it on Ubuntu 22.04 and runs the release smoke test there.

Not executed locally:

- Windows NSIS installation and uninstallation. The workflow builds it on `windows-latest`.
- macOS universal build, code signing, and notarization. The workflow builds it on `macos-15-intel`; signing requires the repository secrets above.
- A full desktop install/uninstall on Debian/Ubuntu, including the package manager scriptlets. The package contents and scriptlets were inspected, but the end-to-end dpkg/rpm transaction was not run.

Known limitations:

- Linux package uninstall cannot reach a user's Secret Service when that user has no active D-Bus session. The cleanup is fail-safe and documented; the user can rerun `serverdesk --purge-data` after logging in.
- AppImage runs without a system installer, so it cannot hook the host uninstaller. Users must run `--purge-data` before deleting the file.
- macOS DMG has no uninstall hook; the documented purge step is required before deleting the app.
- MSI is not shipped. Only NSIS installers include automatic Windows credential cleanup.
- The macOS `.app` is not signed for App Store distribution and has no sandbox entitlements.
- No auto-updater is configured. Releases are manual downloads.
- AppImages built on Ubuntu 22.04 require glibc 2.35 or newer and FUSE 2 (or the extract-and-run flag).
- Windows builds are unsigned and may show SmartScreen warnings.
