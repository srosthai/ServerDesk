import type {
  CapabilityFlags,
  DatabaseAction,
  DatabaseEngine,
  DatabaseMetadataValue,
  DatabaseSessionActionKind,
  DatabaseSessionSummary,
} from "@/types/contracts";
import { engineLabel, type StructureDecision } from "@/features/databases/databaseStructureModel";

export const SESSION_POLL_INTERVAL_MS = 5_000;
export const SESSION_POLL_MAX_TICKS = 60;
export const SESSION_PAGE_LIMIT = 200;

export type SessionCapabilities = {
  browse: StructureDecision;
  cancel: StructureDecision;
  terminate: StructureDecision;
};

function enabled(): StructureDecision {
  return { enabled: true, reason: null };
}

function disabled(reason: string): StructureDecision {
  return { enabled: false, reason };
}

function unsupportedReason(
  label: string,
  engine: DatabaseEngine,
  version: string | null | undefined,
): string {
  return `${label} is not supported by ${engineLabel(engine)} ${version ?? "on this server"}.`;
}

/** Derives session listing and action availability from the capability document. */
export function sessionCapabilitiesFor(input: {
  engine: DatabaseEngine;
  version?: string | null;
  connected: boolean;
  readOnly: boolean;
  actions?: DatabaseAction[];
  flags?: CapabilityFlags;
}): SessionCapabilities {
  const { actions, flags } = input;
  const actionSet = actions ? new Set(actions) : null;
  const connectionReason = !input.connected
    ? "Connect this database profile before listing sessions or running queries."
    : null;

  function decide(
    action: DatabaseAction,
    label: string,
    readOnlyBlocks: boolean,
  ): StructureDecision {
    if (connectionReason) {
      return disabled(connectionReason);
    }
    if (readOnlyBlocks && input.readOnly) {
      return disabled(
        "This database profile is read-only. Cancelling queries and terminating sessions require a writable administrative connection.",
      );
    }
    if (actionSet && !actionSet.has(action)) {
      return disabled(unsupportedReason(label, input.engine, input.version));
    }
    if (!actionSet) {
      if (!flags) {
        return disabled(
          "Capability details are unavailable for this connection. Reconnect to refresh them before using session tools.",
        );
      }
      if (action === "browse_sessions" && !flags.sessions) {
        return disabled(unsupportedReason(label, input.engine, input.version));
      }
      if (action === "cancel_query" && !flags.cancelQuery) {
        return disabled(unsupportedReason(label, input.engine, input.version));
      }
      if (action === "kill_session" && !flags.sessions) {
        return disabled(unsupportedReason(label, input.engine, input.version));
      }
    }
    return enabled();
  }

  return {
    browse: decide("browse_sessions", "Session monitoring", false),
    cancel: decide("cancel_query", "Query cancellation", true),
    terminate: decide("kill_session", "Session termination", true),
  };
}

export function sessionActionLabel(action: DatabaseSessionActionKind): string {
  return action === "cancel_query" ? "Cancel query" : "Terminate session";
}

export function sessionConfirmationPhrase(
  action: DatabaseSessionActionKind,
  sessionId: number,
): string {
  return action === "cancel_query" ? `cancel query ${sessionId}` : `terminate session ${sessionId}`;
}

export function sessionActionCapability(action: DatabaseSessionActionKind): "cancel" | "terminate" {
  return action === "cancel_query" ? "cancel" : "terminate";
}

/** Renders an unsupported field explicitly instead of guessing a value. */
export function metadataText(
  value: DatabaseMetadataValue<string> | DatabaseMetadataValue<number>,
): string {
  return value.status === "available" ? String(value.value) : "Unavailable";
}

export function sessionHostLabel(session: DatabaseSessionSummary): string {
  return metadataText(session.host);
}

export function sessionDatabaseLabel(session: DatabaseSessionSummary): string {
  return metadataText(session.database);
}

export function sessionApplicationLabel(session: DatabaseSessionSummary): string {
  return metadataText(session.applicationName);
}

export function sessionStateLabel(session: DatabaseSessionSummary): string {
  if (session.state.status === "available") {
    return session.state.value;
  }
  return metadataText(session.command);
}

export function sessionWaitEventLabel(session: DatabaseSessionSummary): string {
  return metadataText(session.waitEvent);
}

export function sessionBackendTypeLabel(session: DatabaseSessionSummary): string {
  return metadataText(session.backendType);
}

export function sessionConnectedAtLabel(session: DatabaseSessionSummary): string {
  return metadataText(session.connectedAt);
}

export function sessionQueryText(session: DatabaseSessionSummary): string {
  if (session.query.status === "available") {
    return session.query.value.text;
  }
  if (session.state.status === "available" && session.state.value !== "active") {
    return "No running query";
  }
  return "Unavailable";
}

export function sessionElapsedLabel(session: DatabaseSessionSummary): string {
  if (session.query.status !== "available") {
    return "Unavailable";
  }
  const elapsed = session.query.value.elapsedMs;
  if (elapsed.status !== "available") {
    return "Unavailable";
  }
  return formatSessionDuration(elapsed.value);
}

export function formatSessionDuration(milliseconds: number): string {
  if (!Number.isFinite(milliseconds) || milliseconds < 0) {
    return "Unavailable";
  }
  if (milliseconds < 1_000) {
    return `${Math.round(milliseconds)} ms`;
  }
  const seconds = milliseconds / 1_000;
  if (seconds < 60) {
    return `${seconds.toFixed(1)} s`;
  }
  const wholeSeconds = Math.floor(seconds);
  const minutes = Math.floor(wholeSeconds / 60);
  const remainder = wholeSeconds % 60;
  return `${minutes}m ${remainder}s`;
}

/** The auto-refresh loop stops after a bounded number of ticks. */
export function sessionPollingExhausted(tick: number): boolean {
  return tick >= SESSION_POLL_MAX_TICKS;
}
