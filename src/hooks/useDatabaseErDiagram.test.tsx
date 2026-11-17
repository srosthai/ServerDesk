import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useDatabaseErDiagram } from "@/hooks/useDatabaseErDiagram";
import { erDiagramTableKey } from "@/features/databases/databaseErDiagramModel";
import { listConstraints, listTables } from "@/services/databaseDiscovery";
import type {
  CommandError,
  DatabaseConstraintSummary,
  DatabaseMetadataValue,
  DatabasePage,
  DatabaseTableSummary,
} from "@/types/contracts";

vi.mock("@/lib/tauri", () => ({
  isTauriRuntime: () => true,
}));

vi.mock("@/services/databaseDiscovery", () => ({
  listTables: vi.fn(),
  listConstraints: vi.fn(),
}));

const unavailable = { status: "unavailable" } as const;

function available<T>(value: T): DatabaseMetadataValue<T> {
  return { status: "available", value };
}

function table(name: string): DatabaseTableSummary {
  return {
    database: "app",
    schema: "public",
    name,
    kind: "table",
    estimatedRows: available(10),
    metadataRefreshedAt: available("2026-11-17T08:00:00Z"),
    owner: unavailable,
    encoding: unavailable,
    collation: unavailable,
    sizeBytes: unavailable,
    engine: unavailable,
  };
}

function constraint(name: string): DatabaseConstraintSummary {
  return {
    name,
    kind: "foreign_key",
    columns: ["user_id"],
    referencedSchema: available("public"),
    referencedTable: available("users"),
    referencedColumns: ["id"],
    onUpdate: available("CASCADE"),
    onDelete: available("CASCADE"),
    checkExpression: unavailable,
    deferrable: unavailable,
    initiallyDeferred: unavailable,
  };
}

function page<T>(items: T[], overrides: Partial<DatabasePage<T>> = {}): DatabasePage<T> {
  return {
    items,
    limit: 500,
    offset: 0,
    hasMore: false,
    total: items.length,
    ...overrides,
  };
}

const users = table("users");
const orders = table("orders");
const audit = table("audit");
const events = table("events");

const usersKey = erDiagramTableKey("app", "public", "users");
const ordersKey = erDiagramTableKey("app", "public", "orders");
const auditKey = erDiagramTableKey("app", "public", "audit");
const eventsKey = erDiagramTableKey("app", "public", "events");

function renderDiagram(query = "") {
  return renderHook(() =>
    useDatabaseErDiagram({
      profileId: "profile-1",
      database: "app",
      schema: "public",
      connected: true,
      query,
      limit: 50,
    }),
  );
}

describe("useDatabaseErDiagram", () => {
  beforeEach(() => {
    vi.mocked(listTables).mockReset();
    vi.mocked(listConstraints).mockReset();
    vi.mocked(listTables).mockResolvedValue(page([users, orders], { total: 2 }));
    vi.mocked(listConstraints).mockImplementation((_profileId, ref) =>
      Promise.resolve(
        page(ref.table === "orders" ? [constraint("orders_user_fkey")] : [], { limit: 500 }),
      ),
    );
  });

  it("loads a bounded catalog and only the visible constraint pages", async () => {
    const { result } = renderDiagram();

    expect(result.current.status).toBe("loading");
    await waitFor(() => {
      expect(result.current.status).toBe("ready");
      expect(result.current.constraintLoaded).toBe(2);
    });
    expect(result.current.constraintsLoading).toBe(false);
    expect(result.current.constraints[ordersKey]?.[0]?.name).toBe("orders_user_fkey");
    expect(result.current.constraints[usersKey]).toEqual([]);
    expect(listTables).toHaveBeenCalledWith(
      "profile-1",
      { database: "app", schema: "public" },
      { limit: 500, offset: 0 },
    );
    expect(listConstraints).toHaveBeenCalledTimes(2);
  });

  it("reuses the constraint cache when the table filter changes", async () => {
    vi.mocked(listTables).mockResolvedValue(page([users, orders, audit, events], { total: 4 }));
    const { result, rerender } = renderHook(
      ({ query }: { query: string }) =>
        useDatabaseErDiagram({
          profileId: "profile-1",
          database: "app",
          schema: "public",
          connected: true,
          query,
          limit: 50,
        }),
      { initialProps: { query: "" } },
    );

    await waitFor(() => expect(result.current.constraintLoaded).toBe(4));
    expect(listConstraints).toHaveBeenCalledTimes(4);

    rerender({ query: "orders" });

    await waitFor(() => {
      expect(result.current.constraintTotal).toBe(1);
      expect(result.current.constraintLoaded).toBe(1);
    });
    expect(result.current.constraints[ordersKey]?.[0]?.name).toBe("orders_user_fkey");
    expect(listConstraints).toHaveBeenCalledTimes(4);

    rerender({ query: "events" });
    await waitFor(() => {
      expect(result.current.constraintTotal).toBe(1);
      expect(result.current.constraintLoaded).toBe(1);
      expect(result.current.constraints[eventsKey]).toEqual([]);
    });
    expect(listConstraints).toHaveBeenCalledTimes(4);
    expect(listTables).toHaveBeenCalledTimes(1);
  });

  it("keeps table nodes visible when one constraint page is denied", async () => {
    vi.mocked(listConstraints).mockImplementation((_profileId, ref) => {
      if (ref.table === "orders") {
        return Promise.reject({
          code: "permission",
          message: "Constraint metadata denied.",
        } satisfies CommandError);
      }
      return Promise.resolve(page([]));
    });
    const { result } = renderDiagram();

    await waitFor(() => {
      expect(result.current.status).toBe("ready");
      expect(result.current.constraintLoaded).toBe(2);
    });
    expect(result.current.constraintsLoading).toBe(false);
    expect(result.current.constraintFailures).toEqual([
      {
        key: ordersKey,
        table: "public.orders",
        error: { code: "permission", message: "Constraint metadata denied." },
      },
    ]);
  });

  it("reports a catalog permission failure before any graph is built", async () => {
    vi.mocked(listTables).mockRejectedValue({
      code: "permission",
      message: "Table metadata denied.",
    } satisfies CommandError);
    const { result } = renderDiagram();

    await waitFor(() => {
      expect(result.current.status).toBe("error");
    });
    expect(result.current.error).toEqual({
      code: "permission",
      message: "Table metadata denied.",
    });
    expect(result.current.tables).toEqual([]);
    expect(listConstraints).not.toHaveBeenCalled();
  });

  it("reloads the catalog and clears stale constraint entries", async () => {
    const { result } = renderDiagram();
    await waitFor(() => expect(result.current.constraintLoaded).toBe(2));

    act(() => {
      result.current.reload();
    });

    expect(result.current.status).toBe("loading");
    await waitFor(() => expect(result.current.constraintLoaded).toBe(2));
    expect(listTables).toHaveBeenCalledTimes(2);
    expect(listConstraints).toHaveBeenCalledTimes(4);
  });

  it("does not read metadata without a live connection or a database", async () => {
    const { result } = renderHook(() =>
      useDatabaseErDiagram({
        profileId: "profile-1",
        database: null,
        schema: null,
        connected: true,
      }),
    );
    expect(result.current.status).toBe("idle");
    expect(listTables).not.toHaveBeenCalled();

    const disconnected = renderHook(() =>
      useDatabaseErDiagram({
        profileId: "profile-1",
        database: "app",
        schema: "public",
        connected: false,
      }),
    );
    expect(disconnected.result.current.status).toBe("idle");
    expect(listTables).not.toHaveBeenCalled();
  });

  it("reports a truncated catalog page without dropping loaded tables", async () => {
    vi.mocked(listTables).mockResolvedValue(page([users, orders], { hasMore: true, total: 900 }));
    const { result } = renderDiagram();
    await waitFor(() => expect(result.current.status).toBe("ready"));
    expect(result.current.catalogTruncated).toBe(true);
    expect(result.current.totalTables).toBe(900);
    expect(result.current.tables).toHaveLength(2);
  });

  it("uses the encoded table key for constraints with punctuation in names", async () => {
    const odd = table("user::audit/log");
    const oddKey = erDiagramTableKey("app", "public", "user::audit/log");
    vi.mocked(listTables).mockResolvedValue(page([odd], { total: 1 }));
    vi.mocked(listConstraints).mockResolvedValue(page([]));
    const { result } = renderDiagram();
    await waitFor(() => expect(result.current.constraintLoaded).toBe(1));
    expect(result.current.constraints[oddKey]).toEqual([]);
    expect(auditKey).not.toBe(oddKey);
  });
});
