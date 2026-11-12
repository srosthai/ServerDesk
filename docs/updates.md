# Updates and release signing

ServerDesk can check for, download, verify, and install signed updates from GitHub Releases. The updater is separate from the Windows Authenticode signature and Apple notarization: it uses a Tauri minisign keypair that is embedded in the application as a public key.

The feature ships in the first updater-capable release. A `v0.1.0` installation predates the updater and cannot self-update; those users must install the first updater-capable build manually.

## Platform support

| Platform                  | In-app update | Notes                                                                                                  |
| ------------------------- | ------------- | ------------------------------------------------------------------------------------------------------ |
| Linux AppImage            | Yes           | Downloads the signed AppImage artifact, verifies it, replaces the running AppImage, then relaunches.   |
| Linux DEB or RPM          | No            | Detected but not replaced. ServerDesk links to the release page so the package manager can install it. |
| Windows NSIS              | Yes           | Downloads the NSIS installer and launches it. The app may exit during installation.                    |
| macOS `.app` from the DMG | Yes           | Downloads the universal app archive and replaces the app bundle. Requires a signed/notarized build.    |
| Browser preview           | No            | Update checks are disabled.                                                                            |
| Tauri development build   | No            | Update checks are disabled, even when a release endpoint is configured.                                |

Linux DEB/RPM installs still show available versions and release notes. They do not download or execute the update; the release page link opens the package download instead.

## How a check works

1. An installed build checks `https://github.com/srosthai/ServerDesk/releases/latest/download/latest.json`.
2. A startup check runs after the window opens. It is skipped in development and browser previews, skipped while offline, and throttled to at most once every six hours. The last successful check time is stored in browser storage on the local device.
3. The Settings page has a manual **Check for updates** action that bypasses the six-hour throttle. It is disabled in development and browser previews.
4. When the manifest announces a newer version, the topbar shows an unobtrusive version badge. Selecting it opens Settings.
5. The Settings panel shows the installed version, available version, release notes, download progress, and install/restart state.
6. **Download and install** uses Tauri's `downloadAndInstall`. The updater verifies the artifact's minisign signature with the embedded public key before replacing anything. A missing or invalid signature is rejected. `requireSignedVersion` is enabled, so the version recorded in the signed trusted comment must also match the version announced by the manifest, preventing a tampered manifest from pointing a newer version at an older signed release.
7. On success, ServerDesk calls `relaunch()` and starts the new build. Relaunch happens only after installation completes. If relaunch fails, the panel says the update installed but a manual restart is required.

The update endpoint returns a manifest for the latest **published** release. GitHub's `releases/latest` does not resolve draft or prerelease releases. Do not publish a release that is missing `latest.json` as the latest release.

## Signing key storage

The production minisign keypair lives outside the repository:

- Private key: `/home/dev/.config/serverdesk-updater/serverdesk-updater.key`
- Public key: `/home/dev/.config/serverdesk-updater/serverdesk-updater.key.pub`

The public key is embedded in `src-tauri/tauri.conf.json` under `plugins.updater.pubkey`. The private key is never committed, copied into `src-tauri/`, printed in logs, or attached to a release. The updater configuration also enables `requireSignedVersion`, which rejects an artifact whose signed trusted comment does not name the version announced by the manifest.

The private key file should be mode `0600` and backed up in an offline secret store. The GitHub Actions repository secret `TAURI_SIGNING_PRIVATE_KEY` contains the private key content, not a path:

```text
TAURI_SIGNING_PRIVATE_KEY = contents of serverdesk-updater.key
```

The production key has no password. If a password-protected key is used, set `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` as an additional repository secret. Never put either value in a workflow file, shell history, pull request, or issue.

For a local signed build, pass the private key path to the Tauri CLI without writing it into the repository:

```bash
TAURI_SIGNING_PRIVATE_KEY=/home/dev/.config/serverdesk-updater/serverdesk-updater.key \
  npm run tauri build -- --bundles appimage
```

`bundle.createUpdaterArtifacts` is enabled in `src-tauri/tauri.conf.json`, so a normal bundle build without a signing key fails instead of producing unsigned updater artifacts. Keep that behavior: an unsigned updater artifact must never be published.

### Rotation

The public key is embedded in every installed build. Replacing the keypair therefore does not affect existing installations until they install a build containing the new public key.

1. Generate a new keypair into the offline secret store.
2. Keep the old private key available.
3. Publish a transitional release signed with the **old** key whose build embeds the **new** public key.
4. After the transitional release is adopted, sign subsequent releases with the new key and update the GitHub secret.
5. Retain the old private key while old installs may still need a signed transitional build. Retire it only after the supported upgrade window ends.

If the private key is compromised, publish a security advisory and a manually installable transitional build signed with the compromised key only long enough to tell users to update. Installed apps that never receive the transitional build cannot be forced to trust a new key; they require a manual download. There is no remote revocation channel.

## `latest.json`

`latest.json` is generated by `scripts/latest-json.mjs` from the signed release assets. The workflow runs it before checksums and release upload:

```bash
node scripts/latest-json.mjs \
  --version 0.1.1 \
  --notes-file release-notes.txt \
  --pub-date 2026-11-12T12:00:00Z \
  --asset-dir release-assets \
  --output release-assets/latest.json
```

The generator is deterministic: the platform order is fixed, the publication date and notes are explicit inputs, and no timestamps or random values are generated. It also decodes each `.sig` and rejects signatures whose signed trusted comment does not record the release version, because `requireSignedVersion` is enabled. The workflow uses the tag commit time for `--pub-date` and the annotated tag message (or a release-page link) for `--notes-file`.

Required platform entries:

- `linux-x86_64` — the signed AppImage (`*.AppImage`) and its `.AppImage.sig` file. If a build emits a compressed `*.AppImage.tar.gz`, the generator prefers it and reads its `.sig`.
- `windows-x86_64` — the NSIS installer (`*-setup.exe`) and its `.sig` file.
- `darwin-x86_64` — the universal macOS app archive (`*.app.tar.gz`) and its `.sig` file.
- `darwin-aarch64` — the same universal macOS archive and signature.

The Tauri updater first looks for a bundle-specific key such as `linux-x86_64-appimage` and then falls back to the four keys above. ServerDesk publishes the four common keys so the same manifest works for the supported bundle types.

Example shape (signatures and URLs abbreviated):

```json
{
  "version": "0.1.1",
  "notes": "ServerDesk 0.1.1",
  "pub_date": "2026-11-12T12:00:00Z",
  "platforms": {
    "darwin-aarch64": {
      "url": "https://github.com/srosthai/ServerDesk/releases/download/v0.1.1/ServerDesk_0.1.1_universal.app.tar.gz",
      "signature": "…"
    },
    "darwin-x86_64": {
      "url": "https://github.com/srosthai/ServerDesk/releases/download/v0.1.1/ServerDesk_0.1.1_universal.app.tar.gz",
      "signature": "…"
    },
    "linux-x86_64": {
      "url": "https://github.com/srosthai/ServerDesk/releases/download/v0.1.1/ServerDesk_0.1.1_amd64.AppImage",
      "signature": "…"
    },
    "windows-x86_64": {
      "url": "https://github.com/srosthai/ServerDesk/releases/download/v0.1.1/ServerDesk_0.1.1_x64-setup.exe",
      "signature": "…"
    }
  }
}
```

The stable endpoint is:

```text
https://github.com/srosthai/ServerDesk/releases/latest/download/latest.json
```

Verify the manifest after publishing:

```bash
curl -fsSL https://github.com/srosthai/ServerDesk/releases/latest/download/latest.json
```

It must return the same version as the latest published, non-draft, non-prerelease release and every platform URL must return an artifact with a matching `.sig` file. Do not point the endpoint at a draft or prerelease.

## Release workflow

`.github/workflows/release.yml` passes `TAURI_SIGNING_PRIVATE_KEY` to the Linux, macOS, and Windows build jobs. Tauri writes the updater artifacts and `.sig` files next to the normal installers. The upload steps include:

- Linux: DEB, RPM, AppImage, a compressed `*.AppImage.tar.gz` when produced, and all `.sig` files.
- macOS: DMG, `*.app.tar.gz`, and all `.sig` files.
- Windows: NSIS `*-setup.exe` and its `.sig` file.

The publish job copies those files into one flat directory, generates `latest.json`, adds `SHA256SUMS`, and creates a draft GitHub release. A maintainer reviews and publishes the draft. Publishing is what makes `releases/latest` resolve to it.

Release checklist:

1. Bump the version together in `package.json`, `src-tauri/Cargo.toml`, and `src-tauri/tauri.conf.json`.
2. Run `npm run check`, `npm test`, and the Rust checks.
3. Confirm the `TAURI_SIGNING_PRIVATE_KEY` repository secret is present and current.
4. Merge through `dev`, `production`, and `main`.
5. Tag `main` with `v<version>` and push the tag.
6. Wait for all platform builds; confirm the `.sig` files and `latest.json` are in the draft release.
7. Publish the draft as the latest release, not as a prerelease.
8. Verify the endpoint and install the update from the previous version.

## Platform caveats

### Linux

Only AppImage installs can replace themselves. The updater needs a writable AppImage path and uses the `APPIMAGE` environment variable that the AppImage runtime sets. If FUSE 2 is unavailable, the AppImage must be run with `--appimage-extract-and-run`, which is not a self-updatable installation.

DEB and RPM installs are detected through Tauri's bundle type. They show the available version and link to the release page. Update with the package manager:

```bash
sudo apt install ./ServerDesk_0.1.1_amd64.deb
# or
sudo dnf install ./ServerDesk-0.1.1-1.x86_64.rpm
```

### Windows

The updater uses the per-user NSIS installer. Windows may show a SmartScreen or User Account Control prompt, and the app can exit while the installer runs. The updater signature does not replace Authenticode; unsigned installers still trigger SmartScreen warnings. See [packaging.md](packaging.md#macos-signing-and-notarization) for signing configuration.

### macOS

The updater downloads and replaces the `.app` bundle. Gatekeeper reliability requires a Developer ID-signed and notarized app with the hardened runtime. An unsigned or unnotarized build may update but can be blocked on the next launch. The same universal archive is listed under both macOS architecture keys.

## Failures and rollback

The updater distinguishes offline, network, signature, permission, unsupported installation, install, and relaunch failures. The Settings panel keeps the current installation intact and shows a retry action for check and download failures. A signature failure means the artifact was rejected before installation. If installation succeeded but relaunch failed, quit and reopen ServerDesk.

There is no automatic rollback. The Tauri updater installs newer versions and does not use the manifest to downgrade an installation. The database schema may have migrations that an older build cannot reverse. To roll back:

1. Keep the previous installers from the GitHub release.
2. Quit ServerDesk.
3. Reinstall the older AppImage, NSIS installer, DMG, DEB, or RPM manually.
4. Restore a backup of the application data if the newer build migrated the database and the older build cannot open it.

AppImage replacement may keep the old file in a temporary location during the update, but that is not a supported rollback mechanism. The safe rollback path is a manual reinstall from a retained release artifact.

## Verification limits

A full live self-update test requires two published versions. The `v0.1.0` installation predates the updater and cannot install `v0.1.1` in place. Release QA for `v0.1.1` must:

1. Install a signed `v0.1.1` build.
2. Publish or stage a `v0.1.2` release with the same endpoint.
3. Confirm the `v0.1.1` build detects, downloads, verifies, installs, and relaunches into `v0.1.2`.
4. Repeat with a deliberately corrupted `.sig` or an artifact signed by a different key and confirm the update is rejected.
5. Repeat with the network disabled and confirm the current installation still starts and works.

Local and CI verification can confirm that signing produces the expected `.sig` and updater artifacts and that the manifest generator accepts them. It cannot prove the published `latest.json` endpoint or the final install on every platform.
