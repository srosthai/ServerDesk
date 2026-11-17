import { beforeEach, describe, expect, it, vi } from "vitest";
import { getDatabaseHealth } from "@/services/databaseHealth";
import { useDatabaseHealthStore } from "@/stores/databaseHealthStore";
import type { DatabaseHealthSnapshot } from "@/types/contracts";

vi.mock("@/services/databaseHealth", () => ({
  getDatabaseHealth: vi.fn(),
}));

const snapshot: DatabaseHealthSnapshot = {
  profileId: "profile-1",
  engine: "postgresql",
  database: "app",
  version: { version: "16.2.0", raw: "PostgreSQL 16.2" },
  sampledAt: 1_700_000_000_000,
  latencyMs: { status: "available", value: 12 },
  databaseSizeBytes: { status: "available", value: 8_192 },
  tableSizes: {
    status: "available",
    value: { items: [], limit: 12, truncated: false },
  },
};

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

describe("database health store", () => {
  beforeEach(() => {
    useDatabaseHealthStore.setState({ entries: {} });
    vi.clearAllMocks();
  });

  it("loads a manual sample", async () => {
    vi.mocked(getDatabaseHealth).mockResolvedValue(snapshot);

    await useDatabaseHealthStore.getState().fetchHealth("profile-1");

    expect(getDatabaseHealth).toHaveBeenCalledWith("profile-1", 12);
    const entry = useDatabaseHealthStore.getState().entries["profile-1"];
    expect(entry.status).toBe("ready");
    expect(entry.snapshot).toEqual(snapshot);
    expect(entry.error).toBeNull();
  });

  it("keeps the previous sample visible during a refresh", async () => {
    useDatabaseHealthStore.setState({
      entries: {
        "profile-1": {
          status: "ready",
          snapshot,
          refreshing: false,
          error: null,
          lastRequestedAt: 1,
          requestSequence: 1,
        },
      },
    });
    const pending = deferred<DatabaseHealthSnapshot>();
    vi.mocked(getDatabaseHealth).mockReturnValue(pending.promise);

    const refresh = useDatabaseHealthStore.getState().refresh("profile-1");

    expect(useDatabaseHealthStore.getState().entries["profile-1"]).toMatchObject({
      status: "ready",
      refreshing: true,
      snapshot,
      error: null,
    });

    pending.resolve({ ...snapshot, sampledAt: snapshot.sampledAt + 60_000 });
    await refresh;

    expect(useDatabaseHealthStore.getState().entries["profile-1"]).toMatchObject({
      status: "ready",
      refreshing: false,
      error: null,
    });
    expect(useDatabaseHealthStore.getState().entries["profile-1"].snapshot?.sampledAt).toBe(
      snapshot.sampledAt + 60_000,
    );
  });

  it("keeps the stale sample and reports a refresh failure", async () => {
    useDatabaseHealthStore.setState({
      entries: {
        "profile-1": {
          status: "ready",
          snapshot,
          refreshing: false,
          error: null,
          lastRequestedAt: 1,
          requestSequence: 1,
        },
      },
    });
    vi.mocked(getDatabaseHealth).mockRejectedValue({
      code: "connection",
      message: "The database connection is not active.",
    });

    await useDatabaseHealthStore.getState().refresh("profile-1");

    const entry = useDatabaseHealthStore.getState().entries["profile-1"];
    expect(entry.status).toBe("ready");
    expect(entry.snapshot).toEqual(snapshot);
    expect(entry.refreshing).toBe(false);
    expect(entry.error).toBe("The database connection is not active.");
  });

  it("shows an initial failure without inventing a sample", async () => {
    vi.mocked(getDatabaseHealth).mockRejectedValue({
      code: "timeout",
      message: "The database did not respond.",
    });

    await useDatabaseHealthStore.getState().fetchHealth("profile-1");

    const entry = useDatabaseHealthStore.getState().entries["profile-1"];
    expect(entry.status).toBe("error");
    expect(entry.snapshot).toBeNull();
    expect(entry.error).toBe("The database did not respond.");
  });
});
