# Development

How to build, test, and navigate the ServerDesk repository. User installation
is covered in [install.md](install.md); packaging and release behavior in
[packaging.md](packaging.md).

## Requirements

- Node.js 20 or newer (CI uses Node 22)
- Rust stable with `cargo` and `rustfmt`
- The Tauri 2 desktop dependencies for your platform

### Linux

Debian and Ubuntu:

```bash
sudo apt update
sudo apt install -y libwebkit2gtk-4.1-dev build-essential curl wget file \
  libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev
```

Fedora:

```bash
sudo dnf install -y webkit2gtk4.1-devel gtk3-devel libsoup3-devel javascriptcoregtk4.1-devel \
  librsvg2-devel libappindicator-gtk3-devel openssl-devel pkgconf-pkg-config gcc gcc-c++ make
```

### macOS

Install the Xcode Command Line Tools:

```bash
xcode-select --install
```

For a universal build, add both Rust targets:

```bash
rustup target add aarch64-apple-darwin x86_64-apple-darwin
```

### Windows

Install the Microsoft C++ Build Tools with the **Desktop development with C++**
workload, the WebView2 runtime, and the MSVC Rust toolchain:

```bash
rustup default stable-x86_64-pc-windows-msvc
```

### Rust

Install Rust with [rustup](https://rustup.rs/) if it is not already present:

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

## First checkout

```bash
git clone https://github.com/srosthai/ServerDesk.git
cd ServerDesk
git checkout dev
npm ci
```

Use `npm ci` to match `package-lock.json`, as CI does. `npm install` is fine
while changing dependencies.

## Run the application

```bash
npm run tauri dev
```

This starts Vite on port 1420 and launches the Tauri desktop shell. Run the
frontend alone in a browser with:

```bash
npm run dev
```

The browser preview has no Tauri commands, so server, credential, terminal, and
filesystem features are limited. Use the desktop shell for end-to-end work.

## Checks

Run the same checks as CI before opening a pull request:

```bash
npm run check
npm test
cargo fmt --manifest-path src-tauri/Cargo.toml --check
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml
```

`npm run check` runs version metadata validation, type checking, ESLint,
Prettier, and the production build. It includes Markdown formatting, so README
and `docs/` changes must pass `prettier --check`. Format them with:

```bash
npm run format
```

CI is defined in `.github/workflows/ci.yml`. It runs a frontend job and a Rust
job on Ubuntu for pushes and pull requests to `dev`, `production`, and `main`.
The release workflow in `.github/workflows/release.yml` builds Linux, macOS, and
Windows artifacts.

## Testing

### Frontend

`npm test` runs Vitest with jsdom and Testing Library. Tests live beside the
code as `*.test.ts` or `*.test.tsx`. They cover routing and page behavior,
connection and host-key dialogs, theming and preferences, terminal tab
lifecycle, file selection and transfer queues, service and project views, log
presentation, history filters, error presentation, and store cleanup.

Useful focused commands:

```bash
npm test -- src/features/servers
npm run test:watch
```

### Rust

Rust unit tests live in the modules under `src-tauri/src/`. Integration tests
live in `src-tauri/tests/`:

| Suite                   | Coverage                                                                                      |
| ----------------------- | --------------------------------------------------------------------------------------------- |
| `ssh_authentication.rs` | Password and key authentication, host-key trust, failure mapping, disconnect cleanup          |
| `remote_commands.rs`    | Fixed commands, exit status, output, timeouts, lost sessions                                  |
| `terminal_sessions.rs`  | PTY setup, ANSI output, input, resize, multiple sessions, remote termination                  |
| `files.rs`              | SFTP listing, upload/download, recursion, cancellation, overwrite, rollback, traversal blocks |
| `service_actions.rs`    | Scoped sudo commands, permission denial, verified state, confirmations                        |
| `artisan.rs`            | Fixed Artisan actions, working directory, output redaction, failures                          |
| `projects.rs`           | Project detection, path validation, metadata parsing                                          |
| `queue_supervisor.rs`   | Read-only Supervisor and queue worker detection                                               |
| `logs.rs`               | Bounded tails, allowlists, rotation, permissions, redaction                                   |
| `history.rs`            | Sanitized history, filters, retention, clear behavior                                         |
| `storage.rs`            | Migrations, SQLite constraints, credential references, file modes                             |
| `security.rs`           | Injection attempts, secret scanning, redaction regression coverage                            |

Most SSH integration suites start an in-process `russh` server, so they do not
need a real VPS, network access, or a running `sshd`. The SFTP suite uses an
in-process SFTP subsystem and a local fixture tree.

One credential test is ignored by default because it needs an unlocked OS
credential store:

```bash
cargo test --manifest-path src-tauri/Cargo.toml os_credential_store_round_trip -- --ignored
```

Run it on a desktop session with Secret Service, Keychain, or Credential Manager
available.

## Architecture overview

ServerDesk has two processes in one desktop application:

- a React renderer (webview) for presentation and local UI state;
- a Rust core for SSH, SFTP, SQLite, the OS credential store, and Tauri
  commands.

There is no Node server, hosted API, or background agent. Tauri IPC is the only
boundary between the renderer and the Rust core.

### Frontend

- `src/app/` contains the hash router, application shell, and navigation.
- `src/features/` contains feature modules: dashboard, servers, projects,
  monitoring, services, files, terminal, logs, history, security, and settings.
- `src/services/` contains typed wrappers around Tauri commands. Components do
  not call `invoke` directly.
- `src/stores/` contains Zustand stores for connection, server, host-key,
  project, service, file, terminal, log, history, queue, and preference state.
- `src/types/contracts.ts` defines the shared command and event contracts.
- `src/hooks/useTauriEvent` subscribes to backend events and removes the
  listener on unmount.
- `src/components/ui/` contains shared shadcn/ui primitives, and
  `src/styles/globals.css` contains the Tailwind theme tokens.

The renderer keeps only view state and the latest non-secret samples. Secrets
are submitted to Tauri commands and are never stored in frontend state beyond
the input field.

### Backend

- `src-tauri/src/commands/` is the Tauri command boundary: argument validation,
  confirmation checks, state lookup, and serialization.
- `src-tauri/src/core/` contains domain logic. Every dynamic remote command line
  is built in `core/command.rs`; `core/redaction.rs` is the single redaction
  implementation.
- `src-tauri/src/core/ssh/` owns the connection manager, `russh` transport,
  host-key checks, private-key loading, and terminal channels.
- `src-tauri/src/storage/` owns SQLite access, migrations, and record types.
  Schema files live in `migrations/` and are embedded at compile time.
- `src-tauri/tests/` contains the integration suites listed above.

Connections are managed per server profile. A healthy session is reused, and a
reused session is checked with a fixed read-only `true` command before it is
reported as connected. Terminals and SFTP channels are opened on the existing
SSH connection; no separate listener or agent is installed.

### Data flow

1. A page calls a function in `src/services/`.
2. The service invokes a typed Tauri command.
3. The command validates input, checks confirmation where required, and uses a
   core module or the connection manager.
4. Core logic builds a fixed command or SFTP request, runs it over the managed
   SSH session, and applies output bounds and redaction.
5. The command returns a typed result or a structured `AppError`. The frontend
   maps the error code to a user-facing title and action.
6. Mutating operations record a bounded, sanitized history row after the result
   is known.

Events such as `connection-state-changed`, `terminal-event`, and
`file-transfer-event` update stores without polling the backend.

### Security invariants

Changes to these areas need the security tests and a review of
[security.md](security.md):

- Components must use `src/services/`, not `invoke` directly.
- User-influenced paths must pass `normalize_remote_path` and be quoted as one
  argument. Dynamic command construction belongs in `core/command.rs`.
- Artisan action names, systemd verbs, and fixed command names come from closed
  enums, never caller-provided text.
- Systemd units must pass `validate_service_unit_name`.
- Backend confirmation is required for destructive file, service, Artisan,
  history, host-key, and profile operations.
- Secrets and passphrases must never be logged, included in errors, or written
  to SQLite/history. Use the central redactor for remote text that can reach the
  UI or history.
- New remote commands need output bounds, timeouts, and tests for failure and
  permission paths.
- New migrations must be appended; do not edit an applied migration.
- Keep the version identical in `package.json`, `src-tauri/Cargo.toml`, and
  `src-tauri/tauri.conf.json`.

## UI system

The interface uses Tailwind CSS with shadcn/ui primitives and IBM Plex
typography. Design tokens and theme values live in `src/styles/globals.css`.
Shared primitives live in `src/components/ui/`. The application uses a hash
router because the packaged webview loads local files.

## Project layout

```text
src/                  React frontend
src/app/              Router, shell, and navigation
src/components/ui/    Shared UI primitives
src/features/         Feature modules
src/services/         Typed Tauri command wrappers
src/stores/           Zustand state stores
src/styles/           Design tokens and global styles
src-tauri/            Rust backend and Tauri configuration
src-tauri/src/        Application entry points and Rust modules
src-tauri/tests/      Rust integration tests
migrations/           SQLite schema migrations
```

## Documentation

User and administrator guides live in `docs/`:

- [install.md](install.md)
- [ssh-setup.md](ssh-setup.md)
- [host-keys.md](host-keys.md)
- [features.md](features.md)
- [troubleshooting.md](troubleshooting.md)
- [security.md](security.md)
- [packaging.md](packaging.md)
- [reliability.md](reliability.md)
- [acceptance.md](acceptance.md)

Keep claims aligned with the implementation. If a behavior changes, update the
relevant guide and the acceptance record when it affects the verified workflow.
