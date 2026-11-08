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
cargo check --manifest-path src-tauri/Cargo.toml
```

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
