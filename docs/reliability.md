# Reliability model

This document records the reliability audit: what was already in place, which
gaps were fixed, and which limitations remain accepted. The typed
`AppError`/`ErrorCode` taxonomy and the existing design system are unchanged.

## Error categories

Every failure crosses the Tauri boundary as one structured error with a stable
`code`, a user-facing message, and optional bounded details.

| Code             | Meaning                                                                 |
| ---------------- | ----------------------------------------------------------------------- |
| `authentication` | The SSH server rejected the username, password, or key.                 |
| `host_key`       | The host key is unknown, changed, or failed a trusted-key comparison.   |
| `timeout`        | A connection attempt, command, or request exceeded its time bound.      |
| `dns`            | The hostname could not be resolved.                                     |
| `network`        | The socket or SSH session was lost while an operation was running.      |
| `permission`     | The SSH account or SFTP server denied the operation.                    |
| `remote`         | The remote command or SFTP request failed after it was sent.            |
| `cancelled`      | A user-requested cancellation stopped the operation.                    |
| `connection`     | No authenticated session exists for the requested operation.            |
| `conflict`       | An overwrite, duplicate destination, or existing path blocked the work. |
| `validation`     | Local validation rejected the request before any remote operation.      |
| `not_found`      | A saved profile, project, credential, or remote path does not exist.    |
| `credential`     | A saved credential or private key could not be loaded or decrypted.     |
| `storage`        | The local database or filesystem operation failed.                      |
| `internal`       | A local invariant failed; no remote retry is attempted automatically.   |

A command running without a managed session now returns `connection`, not
`not_found`, so the UI does not describe network loss as a missing profile. A
command timeout on a healthy session stays `timeout`; the same timeout on a
session whose health check fails becomes `network` and moves the connection to
`error`.

## Timeouts and cancellation

- Connection attempts, discovery checks, commands, terminal opens, SFTP
  opens, terminal writes, terminal closes, and SFTP requests each have a time
  bound.
- Session health checks and every session close in the connection manager are
  time-bounded, so a reconnect cannot hang on a stale handle.
- Reusing an existing session runs the fixed read-only `true` command before
  reporting `connected`.
- File uploads and downloads are cancellable at chunk boundaries. Cancellation
  keeps the sibling partial file, returns status `cancelled`, and reports
  `mayBeIncomplete` plus the partial path.
- Closing a terminal sends an SSH channel close and reports a defined exit
  reason. Disconnecting a server closes its terminal channels before the SSH
  session.
- A managed connection attempt is cancelled by disconnecting the server; the
  late session is closed rather than published.

Long-running read-only commands are bounded by timeouts but do not expose a
per-command cancel control. Closing an SSH channel does not prove that a remote
process stopped, so ServerDesk does not claim safe cancellation for commands
that can change remote state.

## Reconnect and duplicate safety

ServerDesk never replays a command automatically. A timed-out or interrupted
command returns an error or an `unknown` history result with guidance to verify
the remote state before retrying. The UI retry controls are explicit user
actions. Service discovery and monitoring refreshes are read-only and may run
again without changing remote state.

A second file transfer to the same server and destination is rejected while the
first is still running. This prevents a retry from writing the same destination
twice when the previous backend task has not finished. The retry uses a new
transfer identifier and a new partial path; no transfer is resumed silently.

## Transfer completeness

- Uploads and downloads write to a sibling `.serverdesk-*.part` file.
- An existing destination is moved to a backup before an overwrite. The backup
  is deleted only after the new destination is verified.
- After an upload rename, ServerDesk re-reads the destination and requires the
  file type and size to match the source. A mismatch removes the unverified
  file and restores the backup when one existed.
- Downloads flush and check the local partial file size before the rename.
- Any cancellation or failure returns the bytes/files transferred, directories
  created, warnings, partial paths, and `mayBeIncomplete=true`.
- A content checksum is not computed; the verification is type and byte count.

## Verified results

- Service actions re-query the unit after the command. History records
  `success` only when the expected state is actually observed, `failure` when
  the observed state contradicts the action, and `unknown` when the state could
  not be verified.
- Create, rename, and delete re-read the remote path. The UI uses warning
  language such as "Delete not verified" instead of claiming success when the
  re-read fails.
- Fixed remote commands report success only for exit code 0; a missing exit
  status is not success.
- File listings and log tails carry explicit typed statuses for missing,
  unreadable, unsupported, truncated, or rotated data.
- Artisan actions are verified by the remote exit status. ServerDesk does not
  claim a deeper filesystem or cache effect that it did not observe.

## Failure-path tests

The Rust suites exercise authentication and host-key failures, DNS/network/
timeout mapping, command timeouts without retry, lost-session network errors,
health-check reuse, upload and download cancellation, interrupted transfers,
duplicate destination rejection, post-rename verification with rollback,
partial recursive delete, service permission denials and verification,
Artisan failures, log permission/rotation failures, terminal cleanup, and
disconnect cleanup. Frontend tests cover distinct error presentations and the
unverified-mutation warning.

## Accepted risks

- Transfer progress and partial paths are in memory. A crash can leave partial
  files without a visible retry record; no transfer resumes automatically.
- Upload verification checks type and size, not content. A malicious SFTP
  server can return plausible metadata for different content.
- Artisan actions that change caches or workers are verified by exit status,
  not by inspecting application internals.
- A connection that dies while idle is reported on the next operation or
  keepalive failure; there is no separate network-loss monitor.
- Long-running commands have timeouts but no safe per-command cancellation.
