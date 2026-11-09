# MVP acceptance verification

This document records the Section 25 and Section 28 acceptance run for the MVP
release candidate. The run used the package built from this repository, a
disposable OpenSSH server, and a disposable Laravel fixture. It did not use a
cloud service, a remote agent, or a hosted backend.

## Candidate and environment

- Date: 2026-11-09
- Candidate: the commit that adds this document on `feat/35-acceptance`
- Application version: `0.1.0`
- Host: Fedora Linux 44 x86_64, kernel `7.2.13-200.fc44.x86_64`
- Desktop: X11 / Xvfb, WebKitGTK 2.54.2
- Toolchain: Node.js 24.21.0, Rust 1.99.1, Tauri 2
- SSH fixture: disposable Ubuntu 22.04 container running OpenSSH 8.9p1 and
  systemd, exposed only on `127.0.0.1:2222`
- Package installation: the RPM was installed with `dnf install` in a
  disposable Fedora 44 container, then launched through the installed
  `/usr/bin/serverdesk` binary with a clean XDG data directory and
  WebKitWebDriver. The other Linux artifacts were smoke-tested separately.

### Artifacts built from this repository

| Artifact                          | Size          | SHA-256                                                            |
| --------------------------------- | ------------- | ------------------------------------------------------------------ |
| `ServerDesk_0.1.0_amd64.deb`      | 6,445,952 B   | `ffe2ba86130ab26e4aad72cff9cc8bfd0299c5a2aff1f253803130e597490b37` |
| `ServerDesk-0.1.0-1.x86_64.rpm`   | 6,452,174 B   | `a6dd0a4286d8f4931e2bf3762b19f621b03fcd07aa9a710c05adf2910e2ac66f` |
| `ServerDesk_0.1.0_amd64.AppImage` | 118,503,928 B | `d6e164fce61f18695a4ab27060241f75026a126cdf98d8d2d71df063e442677f` |

The installed package contains `/usr/bin/serverdesk`, a desktop entry, and the
five declared icon sizes. The RPM and DEB both declare `libwebkit2gtk-4.1` and
GTK 3 runtime dependencies.

## Section 25 results

| #   | Criterion                                                                   | Result      | Evidence                                                                                                                                                                                                                                                                                                                                                                                                                                                                             |
| --- | --------------------------------------------------------------------------- | ----------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| 1   | Install and launch on supported Windows, macOS, and Linux builds            | **Partial** | Linux passed end-to-end from an installed RPM. The DEB installed and passed the native smoke test in an Ubuntu 22.04 container, and the AppImage passed the same smoke test. Windows and macOS were not available on this Linux host and were not run.                                                                                                                                                                                                                               |
| 2   | No cloud account or hosted backend required                                 | **Pass**    | The app opened directly into Overview with no sign-in. All workflow operations used the local SQLite database and the SSH fixture. No cloud endpoint or account UI is present.                                                                                                                                                                                                                                                                                                       |
| 3   | Multiple server profiles                                                    | **Pass**    | Three profiles were created, connected, and survived an application restart: password, private key from a file path, and passphrase-protected key material.                                                                                                                                                                                                                                                                                                                          |
| 4   | SSH password and private-key authentication                                 | **Pass**    | Password auth connected with the fixture password. An Ed25519 key supplied as a path connected. A passphrase-protected Ed25519 key pasted as key material connected and required the passphrase. Keyring entries contained the secrets; SQLite contained only references.                                                                                                                                                                                                            |
| 5   | Host keys verified and unexpected changes block connection                  | **Pass**    | The first connection displayed the fixture fingerprint and waited for explicit trust. Rejecting left the profile disconnected and did not save the key. After replacing the server key, the app showed saved and received fingerprints, warned about a man-in-the-middle risk, and closed the connection before authentication; the sshd journal recorded `Connection closed ... [preauth]` and no new accepted authentication. Restoring the original key reconnected successfully. |
| 6   | Connect, disconnect, and reconnect with clear status                        | **Pass**    | The UI showed `Connecting`, host-key verification, `Connected`, `Disconnected`, and `Reconnecting`/`Connection error` states. Disconnect and reconnect both produced the expected row and header state.                                                                                                                                                                                                                                                                              |
| 7   | Dashboard displays real server information with timestamps                  | **Pass**    | The fixture hostname, Ubuntu 22.04, kernel, x86_64 architecture, and uptime were sampled over SSH. CPU, memory, disk, and network cards reported live values with per-group sample timestamps. The fixed `hostname` and `uptime` commands returned stdout and exit code 0.                                                                                                                                                                                                           |
| 8   | Systemd services discovered, actions verified                               | **Pass**    | Discovery returned 61 units from `systemctl`, including `acceptance.service` and `ssh.service`; no placeholder services were added. Start, stop, and restart were each followed by a status re-query and reported `Verified` with the expected active/inactive state. A permission-denied inspect showed the remote `sudo: a password is required` output and the exact scoped sudo rule.                                                                                            |
| 9   | Laravel projects detected and Artisan commands run in the correct directory | **Pass**    | The fixture was detected only when `artisan`, `composer.json`, `bootstrap/`, and `storage/` existed. Laravel `v11.9.0` came from Composer metadata, PHP `8.1.2` from the fixed command, and the queue hint `redis` from `.env`. `artisan about` and `artisan cache:clear` ran with `/home/deploy/projects/laravel-app` as the working directory, which the fixture printed back.                                                                                                     |
| 10  | Interactive terminal supports PTY input, output, and resize                 | **Pass**    | A shell prompt opened in a dedicated PTY. Typed input returned `ACCEPTANCE_TERMINAL_42`. After resizing the window from 1280x800 to 1024x700, `stty size` changed from `19 135` to `16 99`.                                                                                                                                                                                                                                                                                          |
| 11  | SFTP supports browsing, upload, and download                                | **Pass**    | The listing showed files, directories, sizes, permissions, and modification times. An upload's remote content and a download's local content matched byte-for-byte. New folder, rename, directory delete, and file delete were re-read and verified. A real-OpenSSH directory-delete defect found in this run was fixed and retested (see below).                                                                                                                                    |
| 12  | Secrets not stored in plaintext SQLite or application logs                  | **Pass**    | A recursive scan of the entire application-data tree found no fixture password, key passphrase, Laravel app key, database password, private-key block, or log secret in SQLite, WAL, JSON, or WebKit storage. A history query found zero matching rows. The OS Secret Service contained the password and private-key envelopes. The automation driver log contained no secrets.                                                                                                      |
| 13  | Errors and permissions failures reported honestly                           | **Pass**    | Authentication failure, host-key mismatch, sudo denial, missing remote paths, and the pre-fix directory deletion failure were each shown with a distinct message and the remote output where available. The UI did not report success for an unverified state.                                                                                                                                                                                                                       |
| 14  | No agent or publicly exposed database ports required                        | **Pass**    | The fixture contained only sshd and ordinary services; no ServerDesk agent was installed. The application process opened no database listener; its only listening socket was the loopback WebKit inspector used by the test harness. All remote work used port 2222/SSH.                                                                                                                                                                                                             |

Result summary: **13 pass, 0 fail, 1 partial** (cross-platform installation).
The partial result is the absence of Windows and macOS hosts, not a Linux
failure.

## Section 28 workflow

The installed Linux build completed the full definition-of-done workflow:

1. Install the RPM and launch `/usr/bin/serverdesk` with a clean data directory.
2. Add a profile, verify and trust the host key, authenticate, and connect.
3. Inspect real identity and monitoring samples.
4. Discover systemd units and start, stop, and restart a unit with verified
   results.
5. Detect a Laravel project and run fixed Artisan actions in its directory.
6. Open an interactive terminal, send input, receive output, and resize the PTY.
7. Browse, upload, download, create, rename, and delete remote files over SFTP.
8. Read a bounded, redacted Laravel log tail.
9. Review sanitized operation history.
10. Change preferences and confirm they survive a restart.
11. Disconnect, reconnect, and restart the application with profiles, projects,
    history, and preferences intact.

## Defects found and fixed

1. **Directory deletion failed against real OpenSSH SFTP.** OpenSSH returns
   synthetic `.` and `..` entries from `readdir`. The recursive delete walker
   validated every entry name and therefore rejected every real directory with
   "The remote directory contains an entry name ServerDesk cannot delete
   safely." The transport now filters these synthetic entries before they reach
   listing, transfer, or delete code. The in-process SFTP fixture now emits `.`
   and `..` so the regression is covered by `cargo test --test files`.
2. **Create-folder history used the parent path.** The history row now records
   the created folder path rather than the parent directory.

No critical security or data-loss defect remains open. The only data-integrity
issue found was corrected and its regression test passes.

## Automated checks

| Check                                                                            | Result                                                                                                          |
| -------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------- |
| `npm run check` (version metadata, typecheck, lint, format, production build)    | Pass                                                                                                            |
| `npm test`                                                                       | 42 files, 251 tests passed                                                                                      |
| `cargo fmt --check`                                                              | Pass                                                                                                            |
| `cargo clippy --all-targets -- -D warnings`                                      | Pass                                                                                                            |
| `cargo test`                                                                     | 294 passed, 0 failed, 1 ignored                                                                                 |
| RPM install and full UI acceptance workflow                                      | 91/91 automated acceptance checks passed                                                                        |
| Installed binary smoke test (`--version`, window present, schema 4, 0600 SQLite) | Pass                                                                                                            |
| DEB install and smoke test in Ubuntu 22.04 container                             | Install and smoke pass; local Fedora-built binary emitted a `GLIBC_2.39` weak-version warning (see limitations) |
| AppImage `--version` and smoke test                                              | Pass                                                                                                            |

The Rust total is 200 library tests plus 94 integration tests. The one ignored
test is the environment-gated OS credential-store round trip.

## Platform coverage and limitations

- **Linux:** full install-and-workflow acceptance passed on Fedora 44 with the
  RPM. The DEB and AppImage were smoke-tested; the full UI workflow was not
  repeated from those two formats.
- **Windows and macOS:** not run. This host is Linux-only. The repository's
  release workflow builds those artifacts on their native runners, but no
  acceptance result is claimed here.
- **DEB glibc note:** a package built on Fedora 44 requires a newer glibc than
  Ubuntu 22.04 provides. It installed and completed the smoke test with a
  `GLIBC_2.39` weak-version warning. Release DEBs are built on Ubuntu 22.04 by
  `.github/workflows/release.yml`; that native build was not part of this local
  run. Do not treat a Fedora-built DEB as an Ubuntu 22.04 release artifact.
- **Test harness:** WebDriver ports 4444/4445 and the SSH fixture port 2222 are
  test-only. They are not opened by the application in normal use.
- **Accepted product limitations:** see `docs/security.md`,
  `docs/reliability.md`, and `docs/packaging.md`. They are unchanged by this
  run and remain non-blocking for the Linux MVP.
