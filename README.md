# ServerDesk

ServerDesk is a local-first desktop application for managing Linux VPS servers over SSH. It is built with Tauri 2, React, TypeScript, and Rust.

The app runs on your computer and connects directly to your servers. It does not require a hosted account, a cloud backend, or a server-side agent.

## Requirements

- Node.js 20 or newer
- Rust stable with `cargo`
- Tauri desktop dependencies for your platform

Fedora:

```bash
sudo dnf install -y webkit2gtk4.1-devel gtk3-devel libsoup3-devel javascriptcoregtk4.1-devel \
  librsvg2-devel libappindicator-gtk3-devel openssl-devel pkgconf-pkg-config gcc gcc-c++ make
```

Install Rust with [rustup](https://rustup.rs/):

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

## Development

```bash
npm install
npm run tauri dev
```

Run the frontend without the desktop shell:

```bash
npm run dev
```

## Checks

```bash
npm run typecheck
npm run lint
npm run format:check
npm run build
npm test
cargo check --manifest-path src-tauri/Cargo.toml
cargo test --manifest-path src-tauri/Cargo.toml
```

GitHub Actions runs the frontend and Rust checks on pushes and pull requests to `dev`, `production`, and `main`.

## Credentials

SSH passwords and private key material are stored in the operating system credential store through the Rust keyring integration. SQLite stores only a credential reference. Secret input is available in the desktop build, not the browser preview.

Private key authentication supports OpenSSH, PEM (RSA, ECDSA, Ed25519), and PuTTY PPK keys, including passphrase-protected keys. The passphrase is stored with the key inside the credential store entry, so SQLite still holds only the credential reference. Key material can also be supplied as a local file path. Authentication failures return the distinct `authentication` error code; unreadable keys and incorrect passphrases return `credential`. Credentials, key material, and passphrases are redacted from debug output and are never written to logs.

## Connections

A Rust connection manager owns one SSH session per server profile, reuses healthy sessions, rejects duplicate concurrent connects, and emits `connection-state-changed` events to the frontend. Connection states are `disconnected`, `connecting`, `host-key-verification`, `connected`, `reconnecting`, and `error`.

Host keys are checked against the local `known_hosts` records before authentication. An unknown fingerprint opens a blocking verification dialog with the key type and fingerprint; the key is trusted only after explicit acceptance. A changed fingerprint blocks the connection with a typed `host_key` error and is never saved automatically.

The add/edit server form includes a connection test. It reports visible progress, keeps non-secret field values after a failed test, and uses the same host-key verification flow before sending credentials. Auth failures, host-key mismatches, timeouts, DNS failures, and network loss keep distinct error codes and UI labels. Tests against an unsaved profile trust an accepted key only for that test; saving and connecting verifies the key again.

## Remote commands

The Overview page provides the first end-to-end vertical slice: add a profile, connect, verify the host key, authenticate, run fixed `hostname` and `uptime` commands, inspect real stdout, stderr, the exit code, and the connection state, then disconnect and release the session. The Rust command path accepts a closed `RemoteCommand` enum and maps it to fixed command templates, so user text is never concatenated into a shell command. A non-zero exit status is returned as a structured result rather than lost as a transport error.

`cargo test --manifest-path src-tauri/Cargo.toml --test remote_commands` starts an in-process `russh` server and exercises the real client transport, host-key verification, password authentication, command execution, failure output, and disconnect cleanup.

## System identity

The Overview page samples server identity over the authenticated session. Hostname, distribution, and version come from fixed read-only templates (`hostname` and `cat /etc/os-release`), kernel and architecture come from `uname -srm`, and uptime comes from `cat /proc/uptime`. Parsing is per field: if `/etc/os-release` is missing or a distribution does not publish `VERSION_ID`, only those values are shown as unavailable.

Every identity command has a 16 KiB output bound and uses the connection manager's command timeout. The typed `get_system_identity` Tauri command returns a `SystemIdentity` sample with a local sample timestamp, duration, and connection state. The frontend calls it through `src/services/identity.ts` and stores per-server samples in `useSystemIdentityStore`.

## Monitoring snapshots

The Overview page collects lightweight, snapshot-based monitoring over the authenticated session. CPU load averages come from `/proc/loadavg`, the logical CPU count from `nproc`, memory from `/proc/meminfo`, filesystem usage from `df -Pk`, and interface counters from `/proc/net/dev`. Every command is a fixed read-only template with a 32 KiB output bound and the connection manager's command timeout.

Each metric group carries its own sample timestamp and an explicit status: `available`, `unsupported`, or `unavailable`. A transport failure aborts the refresh; the frontend keeps the last successful snapshot, marks it stale, and keeps showing its original sample time. Polling uses the monitoring interval from Settings, clamped to a five-second minimum, and stops whenever no server is selected or the session is disconnected. Manual refresh always remains available while connected.

## Server overview

The default Overview workspace is the server health dashboard. It shows the selected server's connection state, hostname, distribution, kernel, architecture, and uptime alongside CPU load, memory, disk, and network cards. Every metric carries a unit and a sample time, and the dashboard summary reports whether the latest data is live, stale, or unavailable.

Manual refresh updates identity and monitoring in place without a page reload. Monitoring polling uses the interval configured in Settings, clamped to a five-second minimum, and only runs while the selected session is connected. Identity refreshes on demand. A disconnected server never shows live values: the dashboard either shows an explicit unavailable state or retains the last successful sample with its original timestamp and a visible stale marker.

## Command contracts

React calls typed Tauri commands through `src/services/`. Contract types live in `src/types/contracts.ts`, and every command returns a structured `AppError` when it fails. Components do not call `invoke` directly. Tauri event subscriptions use `useTauriEvent`, which cleans up its listener on unmount.

## UI system

The interface uses Tailwind CSS with shadcn/ui primitives and IBM Plex typography. Design tokens and theme values live in `src/styles/globals.css`. Shared primitives live in `src/components/ui/`.

## Project layout

```text
src/                  React frontend
src/components/ui/    Shared UI primitives
src/features/         Feature modules
src/styles/           Design tokens and global styles
src-tauri/            Rust backend and Tauri configuration
src-tauri/src/        Application entry points and Rust modules
```

## Status

Early scaffold. Feature work is tracked in the repository issues and organized by milestone.
