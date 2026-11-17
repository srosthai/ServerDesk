import type { ComponentProps } from "react";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import DatabaseErDiagramView from "@/features/databases/DatabaseErDiagramView";
import {
  erDiagramTableKey,
  type ErDiagramGraph,
  type ErDiagramTable,
} from "@/features/databases/databaseErDiagramModel";
import { objectRowId } from "@/features/databases/databaseExplorerTreeModel";
import { useDatabaseErDiagram } from "@/hooks/useDatabaseErDiagram";
import type {
  DatabaseConstraintSummary,
  DatabaseMetadataValue,
  DatabaseProfile,
  DatabaseTableSummary,
} from "@/types/contracts";

vi.mock("@/hooks/useDatabaseErDiagram", () => ({
  useDatabaseErDiagram: vi.fn(),
}));

vi.mock("@/features/databases/DatabaseErDiagramCanvas", () => ({
  default: function MockCanvas({
    graph,
    onSelectTable,
  }: {
    graph: ErDiagramGraph;
    onSelectTable: (table: ErDiagramTable) => void;
  }) {
    return (
      <div data-testid="mock-canvas">
        <span>{graph.tables.length} nodes</span>
        <button type="button" onClick={() => onSelectTable(graph.tables[0])}>
          Mock select first table
        </button>
      </div>
    );
  },
}));

const reload = vi.fn();
const onSelectTable = vi.fn();
const onConnect = vi.fn();

function available<T>(value: T): DatabaseMetadataValue<T> {
  return { status: "available", value };
}

const unavailable = { status: "unavailable" } as const;

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

function foreignKey(
  name: string,
  columns: string[],
  referencedTable: string,
  referencedColumns: string[],
): DatabaseConstraintSummary {
  return {
    name,
    kind: "foreign_key",
    columns,
    referencedSchema: available("public"),
    referencedTable: available(referencedTable),
    referencedColumns,
    onUpdate: available("CASCADE"),
    onDelete: available("CASCADE"),
    checkExpression: unavailable,
    deferrable: unavailable,
    initiallyDeferred: unavailable,
  };
}

const users = table("users");
const orders = table("orders");
const usersKey = erDiagramTableKey("app", "public", "users");
const ordersKey = erDiagramTableKey("app", "public", "orders");

const profile: DatabaseProfile = {
  id: "profile-1",
  serverId: "server-1",
  name: "Primary",
  engine: "postgresql",
  host: "127.0.0.1",
  port: 5432,
  socket: null,
  databaseName: "app",
  username: "app",
  hasCredential: true,
  readOnly: false,
  sslMode: null,
  createdAt: 1,
  updatedAt: 1,
  lastUsedAt: null,
};

function readyState(overrides: Partial<ReturnType<typeof useDatabaseErDiagram>> = {}) {
  return {
    status: "ready" as const,
    error: null,
    tables: [users, orders],
    totalTables: 2,
    catalogTruncated: false,
    constraints: {
      [usersKey]: [],
      [ordersKey]: [foreignKey("orders_user_fkey", ["user_id"], "users", ["id"])],
    },
    constraintFailures: [],
    constraintLoaded: 2,
    constraintTotal: 2,
    constraintsLoading: false,
    updatedAt: 1_700_000_000_000,
    reload,
    ...overrides,
  };
}

function renderView(overrides: Partial<ComponentProps<typeof DatabaseErDiagramView>> = {}) {
  return render(
    <DatabaseErDiagramView
      profile={profile}
      profileName="Primary"
      engine="postgresql"
      connected
      database="app"
      schema="public"
      selectedTable={null}
      onSelectTable={onSelectTable}
      onConnect={onConnect}
      {...overrides}
    />,
  );
}

describe("DatabaseErDiagramView", () => {
  beforeEach(() => {
    vi.mocked(useDatabaseErDiagram).mockReset();
    onSelectTable.mockReset();
    onConnect.mockReset();
    reload.mockReset();
    vi.mocked(useDatabaseErDiagram).mockReturnValue(readyState());
  });

  it("asks for a connection before reading relationship metadata", async () => {
    const user = userEvent.setup();
    vi.mocked(useDatabaseErDiagram).mockReturnValue(readyState({ status: "idle", tables: [] }));
    renderView({ connected: false });

    expect(screen.getByText("Connection required")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Connect" }));
    expect(onConnect).toHaveBeenCalledTimes(1);
  });

  it("asks for a database or schema when no scope is selected", () => {
    renderView({ database: null, schema: null });
    expect(screen.getByText("Select a database or schema")).toBeInTheDocument();
  });

  it("shows a loading state while the catalog is read", () => {
    vi.mocked(useDatabaseErDiagram).mockReturnValue(
      readyState({ status: "loading", tables: [], constraints: {}, constraintLoaded: 0 }),
    );
    renderView();
    expect(screen.getByLabelText("Loading ER diagram")).toBeInTheDocument();
    expect(
      screen.getByText("Reading table names and relationship metadata from the catalog."),
    ).toBeInTheDocument();
  });

  it("keeps permission failures actionable and retryable", async () => {
    const user = userEvent.setup();
    vi.mocked(useDatabaseErDiagram).mockReturnValue(
      readyState({
        status: "error",
        error: { code: "permission", message: "Permission denied for table metadata." },
        tables: [],
      }),
    );
    renderView();

    expect(screen.getByText("Relationship metadata is not available")).toBeInTheDocument();
    expect(screen.getByText(/Permission denied for table metadata/)).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Try again" }));
    expect(reload).toHaveBeenCalledTimes(1);
  });

  it("renders an explicit empty schema state", () => {
    vi.mocked(useDatabaseErDiagram).mockReturnValue(
      readyState({ tables: [], totalTables: 0, constraints: {} }),
    );
    renderView();
    expect(screen.getByText("No tables in this schema")).toBeInTheDocument();
  });

  it("renders the no-FK state without hiding isolated tables", async () => {
    vi.mocked(useDatabaseErDiagram).mockReturnValue(
      readyState({
        constraints: { [usersKey]: [], [ordersKey]: [] },
      }),
    );
    renderView();

    expect(screen.getByText("No foreign-key relationships found")).toBeInTheDocument();
    expect(await screen.findByTestId("mock-canvas")).toHaveTextContent("2 nodes");
    expect(screen.getByText(/Tables without foreign keys are still shown/)).toBeInTheDocument();
  });

  it("selects a table through the graph and opens the structure selection", async () => {
    const user = userEvent.setup();
    renderView();

    await user.click(await screen.findByRole("button", { name: "Mock select first table" }));
    expect(onSelectTable).toHaveBeenCalledTimes(1);
    expect(onSelectTable).toHaveBeenCalledWith({
      id: objectRowId("profile-1", "app", "public", "tables", "orders"),
      profileId: "profile-1",
      kind: "table",
      label: "orders",
      engine: "postgresql",
      database: "app",
      schema: "public",
      objectName: "orders",
    });
  });

  it("offers a keyboard-accessible table index with the same selection target", async () => {
    const user = userEvent.setup();
    renderView();

    await user.click(screen.getByText("Table and relationship index"));
    await user.click(screen.getByRole("button", { name: "Open structure for public.users" }));
    expect(onSelectTable).toHaveBeenCalledWith(
      expect.objectContaining({
        id: objectRowId("profile-1", "app", "public", "tables", "users"),
        objectName: "users",
      }),
    );
    expect(
      screen.getByText("user_id → id · orders_user_fkey · on delete CASCADE"),
    ).toBeInTheDocument();
  });

  it("filters visible tables and explains when nothing matches", async () => {
    const user = userEvent.setup();
    renderView();

    const filter = screen.getByRole("textbox", { name: "Filter diagram tables" });
    await user.type(filter, "orders");
    await waitFor(() => {
      expect(vi.mocked(useDatabaseErDiagram).mock.calls.at(-1)?.[0].query).toBe("orders");
    });
    expect(await screen.findByTestId("mock-canvas")).toHaveTextContent("1 nodes");

    await user.clear(filter);
    await user.type(filter, "does-not-exist");
    expect(screen.getByText("No tables match this filter")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Clear filter" }));
    expect(filter).toHaveValue("");
  });

  it("waits for constraint metadata before mounting the graph", () => {
    vi.mocked(useDatabaseErDiagram).mockReturnValue(
      readyState({ constraintsLoading: true, constraintLoaded: 1, constraintTotal: 2 }),
    );
    renderView();
    expect(screen.queryByTestId("mock-canvas")).not.toBeInTheDocument();
    expect(screen.queryByText("No foreign-key relationships found")).not.toBeInTheDocument();
    expect(
      screen.getByText("Loading relationship metadata for 1 of 2 visible tables."),
    ).toBeInTheDocument();
  });

  it("discloses partial constraints and large schema limits", async () => {
    vi.mocked(useDatabaseErDiagram).mockReturnValue(
      readyState({
        constraintFailures: [
          {
            key: ordersKey,
            table: "public.orders",
            error: { code: "permission", message: "Constraint metadata denied." },
          },
        ],
        constraintLoaded: 2,
      }),
    );
    renderView();

    expect(screen.getByText("Relationships may be incomplete")).toBeInTheDocument();
    expect(screen.getByText(/Constraint metadata denied/)).toBeInTheDocument();
  });

  it("shows the table limit warning when a large schema is narrowed", () => {
    const manyTables = Array.from({ length: 51 }, (_, index) => table(`table_${index}`));
    vi.mocked(useDatabaseErDiagram).mockReturnValue(
      readyState({
        tables: manyTables,
        totalTables: manyTables.length,
        constraints: {},
      }),
    );
    renderView();

    expect(screen.getByText("Diagram table limit reached")).toBeInTheDocument();
  });

  it("warns when relationships exceed the rendering limit", async () => {
    const manyRelationships = Array.from({ length: 301 }, (_, index) =>
      foreignKey(`orders_user_fkey_${index}`, ["user_id"], "users", ["id"]),
    );
    vi.mocked(useDatabaseErDiagram).mockReturnValue(
      readyState({
        constraints: { [usersKey]: [], [ordersKey]: manyRelationships },
      }),
    );
    renderView();
    expect(screen.getByText("Relationship limit reached")).toBeInTheDocument();
    await screen.findByTestId("mock-canvas");
  });
});
