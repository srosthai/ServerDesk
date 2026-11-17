import { beforeEach, describe, expect, it, vi } from "vitest";
import { useDatabaseSessionStore } from "@/stores/databaseSessionStore";
import { listDatabaseSessions, runDatabaseSessionAction } from "@/services/databaseSessions";
import type { DatabaseSessionActionResult, DatabaseSessionSummary } from "@/types/contracts";

vi.mock("@/services/databaseSessions", () => ({
  listDatabaseSessions: vi.fn(),
  runDatabaseSessionAction: vi.fn(),
}));

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

const page = {
  items: [session],
  limit: 200,
  offset: 0,
  hasMore: false,
  total: 1,
};

const result: DatabaseSessionActionResult = {
  action: "terminate_session",
  engine: "postgresql",
  sessionId: 501,
  user: "app",
  database: { status: "available", value: "app" },
  requestAccepted: true,
  verified: true,
  verification: "Session 501 is no longer present. Termination verified.",
  durationMs: 4,
};

describe("databaseSessionStore", () => {
  beforeEach(() => {
    useDatabaseSessionStore.setState({ entries: {} });
    vi.mocked(listDatabaseSessions).mockReset();
    vi.mocked(runDatabaseSessionAction).mockReset();
  });

  it("loads sessions and keeps a bounded page", async () => {
    vi.mocked(listDatabaseSessions).mockResolvedValue(page);

    await useDatabaseSessionStore.getState().fetchSessions("profile-1");

    expect(listDatabaseSessions).toHaveBeenCalledWith(
      "profile-1",
      { limit: 200, offset: 0 },
      false,
    );
    expect(useDatabaseSessionStore.getState().entries["profile-1"]).toMatchObject({
      status: "ready",
      sessions: [session],
      runningOnly: false,
      hasMore: false,
      error: null,
    });
  });

  it("requests running-only sessions without clearing the current table", async () => {
    vi.mocked(listDatabaseSessions).mockResolvedValue(page);
    await useDatabaseSessionStore.getState().fetchSessions("profile-1");
    vi.mocked(listDatabaseSessions).mockResolvedValue({ ...page, items: [session] });

    await useDatabaseSessionStore.getState().setRunningOnly("profile-1", true);

    expect(listDatabaseSessions).toHaveBeenLastCalledWith(
      "profile-1",
      { limit: 200, offset: 0 },
      true,
    );
    expect(useDatabaseSessionStore.getState().entries["profile-1"]?.runningOnly).toBe(true);
  });

  it("keeps a typed permission error and preserves the last visible sessions", async () => {
    vi.mocked(listDatabaseSessions).mockResolvedValue(page);
    await useDatabaseSessionStore.getState().fetchSessions("profile-1");
    vi.mocked(listDatabaseSessions).mockRejectedValue({
      code: "permission",
      message: "The database account does not have permission for this operation.",
    });

    await useDatabaseSessionStore.getState().refresh("profile-1");

    expect(useDatabaseSessionStore.getState().entries["profile-1"]).toMatchObject({
      status: "error",
      sessions: [session],
      error: "The database account does not have permission for this operation.",
    });
  });

  it("refreshes the list after a verified action", async () => {
    vi.mocked(listDatabaseSessions).mockResolvedValue(page);
    vi.mocked(runDatabaseSessionAction).mockResolvedValue(result);
    await useDatabaseSessionStore.getState().fetchSessions("profile-1");

    const action = await useDatabaseSessionStore.getState().executeAction("profile-1", {
      sessionId: 501,
      action: "terminate_session",
      expectedUser: "app",
      expectedDatabase: "app",
      confirmation: "terminate session 501",
    });

    expect(action.verified).toBe(true);
    expect(runDatabaseSessionAction).toHaveBeenCalledWith("profile-1", {
      sessionId: 501,
      action: "terminate_session",
      expectedUser: "app",
      expectedDatabase: "app",
      confirmation: "terminate session 501",
    });
    expect(listDatabaseSessions).toHaveBeenCalledTimes(2);
    expect(useDatabaseSessionStore.getState().entries["profile-1"]?.lastResult).toEqual(result);
  });

  it("keeps an unverified result for the panel to disclose", async () => {
    vi.mocked(listDatabaseSessions).mockResolvedValue(page);
    vi.mocked(runDatabaseSessionAction).mockResolvedValue({
      ...result,
      verified: false,
      verification: "Session 501 is still present.",
      verificationError: "The session was still present when re-read.",
    });
    await useDatabaseSessionStore.getState().fetchSessions("profile-1");

    await useDatabaseSessionStore.getState().executeAction("profile-1", {
      sessionId: 501,
      action: "terminate_session",
      expectedUser: "app",
      confirmation: "terminate session 501",
    });

    expect(useDatabaseSessionStore.getState().entries["profile-1"]?.lastResult).toMatchObject({
      verified: false,
      verificationError: "The session was still present when re-read.",
    });
  });

  it("stores a permission denial without losing the dialog error", async () => {
    vi.mocked(listDatabaseSessions).mockResolvedValue(page);
    vi.mocked(runDatabaseSessionAction).mockRejectedValue({
      code: "permission",
      message: "The database account does not have permission for this operation.",
    });
    await useDatabaseSessionStore.getState().fetchSessions("profile-1");

    await expect(
      useDatabaseSessionStore.getState().executeAction("profile-1", {
        sessionId: 501,
        action: "terminate_session",
        expectedUser: "app",
        confirmation: "terminate session 501",
      }),
    ).rejects.toMatchObject({ code: "permission" });

    expect(useDatabaseSessionStore.getState().entries["profile-1"]).toMatchObject({
      actionStatus: "idle",
      actionError: "The database account does not have permission for this operation.",
      lastResult: null,
    });
  });

  it("resets one profile without clearing other connections", async () => {
    vi.mocked(listDatabaseSessions).mockResolvedValue(page);
    await useDatabaseSessionStore.getState().fetchSessions("profile-1");
    await useDatabaseSessionStore.getState().fetchSessions("profile-2");

    useDatabaseSessionStore.getState().reset("profile-1");

    expect(useDatabaseSessionStore.getState().entries["profile-1"]).toBeUndefined();
    expect(useDatabaseSessionStore.getState().entries["profile-2"]).toBeDefined();
  });
});
