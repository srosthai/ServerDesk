# ServerDesk

ServerDesk is a local-first desktop application for managing Linux VPS servers over SSH. It connects directly from your computer to your servers and has no hosted account, cloud backend, or server-side agent.

## Features

- Saved SSH profiles with password or private-key authentication and explicit host-key verification.
- A live server overview with system identity, CPU load, memory, disk, and network samples.
- Multiple interactive terminal tabs per server.
- A remote file browser with verified upload, download, rename, and delete operations.
- Systemd service discovery with verified start, stop, and restart actions.
- Laravel project detection, fixed Artisan actions, and project dashboards.
- Bounded log tails and a local, sanitized operation history.
- Local SQLite metadata with secrets kept in the operating system credential store.

Detailed behavior is documented in [docs/features.md](docs/features.md).

## Supported platforms

- **Linux:** DEB and AppImage on Ubuntu 22.04+ and Debian 12+; RPM and AppImage on Fedora 39+.
- **macOS:** 10.15 or newer, as a universal DMG for Intel and Apple silicon.
- **Windows:** 10 or newer, using a per-user NSIS installer.

Download the latest installers from the repository's Releases page. Packaging, signing, data locations, uninstall cleanup, and known limitations are covered in [docs/packaging.md](docs/packaging.md).

## Development setup

Requirements: Node.js 20 or newer, stable Rust, and the Tauri desktop dependencies for your platform.

Fedora:

```bash
sudo dnf install -y webkit2gtk4.1-devel gtk3-devel libsoup3-devel javascriptcoregtk4.1-devel \
  librsvg2-devel libappindicator-gtk3-devel openssl-devel pkgconf-pkg-config gcc gcc-c++ make
```

Install Rust with [rustup](https://rustup.rs/), then:

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

- [Features](docs/features.md)
- [Development](docs/development.md)
- [Packaging and distribution](docs/packaging.md)
- [Acceptance verification](docs/acceptance.md)
- [Security model](docs/security.md)
- [Reliability model](docs/reliability.md)
