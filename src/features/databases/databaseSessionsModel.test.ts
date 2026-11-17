import { describe, expect, it } from "vitest";
import {
  SESSION_POLL_MAX_TICKS,
  formatSessionDuration,
  sessionCapabilitiesFor,
  sessionConfirmationPhrase,
  sessionElapsedLabel,
  sessionPollingExhausted,
  sessionQueryText,
  sessionStateLabel,
} from "@/features/databases/databaseSessionsModel";
import type { CapabilityFlags, DatabaseSessionSummary } from "@/types/contracts";

const flags: CapabilityFlags = {
  schemas: true,
  sequences: true,
  materializedViews: true,
  routines: true,
  routineEdit: true,
  triggers: true,
  triggerEdit: true,
  viewManagement: true,
  roles: true,
  sessions: true,
  transactions: true,
  savepoints: true,
  cancelQuery: true,
  explain: true,
  checkConstraints: true,
  generatedColumns: true,
  identityColumns: true,
  autoIncrementColumns: false,
  columnTypeChange: true,
  columnRename: true,
  tableRename: true,
  tableDuplicate: true,
  columnReorder: false,
  databaseRename: true,
  indexRename: true,
  partialIndexes: true,
  queryTimeout: true,
};

const session: DatabaseSessionSummary = {
  id: 501,
  user: "app",
  host: { status: "available", value: "10.0.0.5" },
  database: { status: "available", value: "app" },
  applicationName: { status: "available", value: "psql" },
  command: { status: "unavailable" },
  state: { status: "available", value: "active" },
  waitEvent: { status: "unavailable" },
  backendType: { status: "unavailable" },
  connectedAt: { status: "available", value: "2026-11-17T08:00:00Z" },
  isCurrent: false,
  query: {
    status: "available",
    value: {
      text: "SELECT pg_sleep(30)",
      redacted: false,
      truncated: false,
      startedAt: { status: "available", value: "2026-11-17T08:05:00Z" },
      elapsedMs: { status: "available", value: 30_000 },
    },
  },
};

describe("databaseSessionsModel", () => {
  it("builds exact confirmation phrases per action and session", () => {
    expect(sessionConfirmationPhrase("cancel_query", 501)).toBe("cancel query 501");
    expect(sessionConfirmationPhrase("terminate_session", 501)).toBe("terminate session 501");
  });

  it("disables actions on read-only profiles while keeping listing available", () => {
    const capabilities = sessionCapabilitiesFor({
      engine: "postgresql",
      version: "16.2.0",
      connected: true,
      readOnly: true,
      flags,
    });

    expect(capabilities.browse.enabled).toBe(true);
    expect(capabilities.cancel.enabled).toBe(false);
    expect(capabilities.terminate.enabled).toBe(false);
    expect(capabilities.terminate.reason).toContain("read-only");
  });

  it("marks unsupported session actions unavailable", () => {
    const capabilities = sessionCapabilitiesFor({
      engine: "postgresql",
      version: "9.1.0",
      connected: true,
      readOnly: false,
      flags: { ...flags, sessions: false, cancelQuery: false },
    });

    expect(capabilities.browse.enabled).toBe(false);
    expect(capabilities.browse.reason).toContain("Session monitoring");
    expect(capabilities.cancel.enabled).toBe(false);
    expect(capabilities.terminate.enabled).toBe(false);
  });

  it("formats elapsed time and unsupported values explicitly", () => {
    expect(formatSessionDuration(850)).toBe("850 ms");
    expect(formatSessionDuration(2_500)).toBe("2.5 s");
    expect(formatSessionDuration(65_000)).toBe("1m 5s");
    expect(sessionElapsedLabel(session)).toBe("30.0 s");

    const idle = {
      ...session,
      state: { status: "available" as const, value: "idle" },
      query: { status: "unavailable" as const },
    };
    expect(sessionElapsedLabel(idle)).toBe("Unavailable");
    expect(sessionQueryText(idle)).toBe("No running query");
    expect(sessionStateLabel(idle)).toBe("idle");
  });

  it("bounds the polling loop", () => {
    expect(sessionPollingExhausted(0)).toBe(false);
    expect(sessionPollingExhausted(SESSION_POLL_MAX_TICKS - 1)).toBe(false);
    expect(sessionPollingExhausted(SESSION_POLL_MAX_TICKS)).toBe(true);
  });
});
