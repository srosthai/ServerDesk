# ServerDesk

ServerDesk is a local-first desktop application for managing Linux VPS servers over SSH. It connects directly from your computer to your servers and has no hosted account, cloud backend, or server-side agent.

## Features

- Saved SSH profiles with password or private-key authentication and explicit host-key verification.
- A live server overview with system identity, CPU load, memory, disk, and network samples.
- Multiple interactive terminal tabs per server.
- A remote file browser with verified upload, download, rename, and delete operations.
- Systemd service discovery with verified start, stop, and restart actions.
- Laravel project detection, fixed Artisan actions, and a read-only queue and Supervisor view.
- Bounded log tails and a local, sanitized operation history.
- Local SQLite metadata with secrets kept in the operating system credential store.

Detailed behavior is documented in [docs/features.md](docs/features.md).

## Supported platforms

- **Linux (x86_64):** DEB and AppImage on Ubuntu 22.04+ and Debian 12+; RPM and AppImage on Fedora 39+.
- **macOS:** 10.15 or newer, as a universal DMG for Intel and Apple silicon.
- **Windows:** 64-bit Windows 10 or newer, using a per-user NSIS installer.

Release builds are attached to the repository's GitHub Releases page. If no release is published yet, build from source with [docs/development.md](docs/development.md) and [docs/packaging.md](docs/packaging.md).

## Install

Download the installer for your platform and follow [docs/install.md](docs/install.md) for package commands, first-run setup, and uninstall notes. Linux/macOS and Windows also support verified command-line install scripts described in [docs/install.md](docs/install.md#command-line-install).

## Development setup

Requirements: Node.js 20 or newer, stable Rust, and the Tauri desktop dependencies for your platform. Platform packages are listed in [docs/development.md](docs/development.md).

```bash
npm install
npm run tauri dev
```

Run the frontend without the desktop shell:

```bash
npm run dev
```

## Build installers

```bash
npm run check
npm run tauri build -- --bundles deb,rpm,appimage   # Linux
npm run tauri build -- --bundles nsis               # Windows
npm run tauri build -- --target universal-apple-darwin --bundles app,dmg  # macOS
```

Artifacts are written to `src-tauri/target/release/bundle`. Do not commit `dist/`, `src-tauri/target/`, or installer output.

## Documentation

User guide:

- [Install and first run](docs/install.md)
- [SSH account and scoped sudo](docs/ssh-setup.md)
- [Host key verification and recovery](docs/host-keys.md)
- [Features](docs/features.md)
- [Troubleshooting](docs/troubleshooting.md)
- [Security and privacy](docs/security.md)

Contributor reference:

- [Development](docs/development.md)
- [Packaging and distribution](docs/packaging.md)
- [Reliability model](docs/reliability.md)
- [Acceptance verification](docs/acceptance.md)
