import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { DatabaseExplorerTree } from "@/features/databases/DatabaseExplorerTree";
import type { DatabaseExplorerSelection } from "@/features/databases/databaseExplorerTreeModel";
import { loadExplorerNode, refreshExplorerNode } from "@/services/databaseDiscovery";
import { useDatabaseExplorerStore } from "@/stores/databaseExplorerStore";
import type {
  DatabaseCapabilities,
  DatabaseConnectionSnapshot,
  DatabaseExplorerNode,
  DatabaseExplorerNodePayload,
  DatabasePage,
  DatabaseProfile,
  DatabaseSchemaSummary,
  DatabaseSequenceSummary,
  DatabaseSummary,
  DatabaseTableSummary,
  DatabaseViewSummary,
} from "@/types/contracts";

vi.mock("@/services/databaseDiscovery", () => ({
  loadExplorerNode: vi.fn(),
  refreshExplorerNode: vi.fn(),
}));

const unavailable = { status: "unavailable" } as const;
const metadata = {
  owner: unavailable,
  encoding: unavailable,
  collation: unavailable,
  sizeBytes: unavailable,
  engine: unavailable,
};

const capabilities: DatabaseCapabilities = {
  engine: "postgresql",
  version: "16.2.0",
  flags: {
    schemas: true,
    sequences: true,
    materializedViews: true,
    routines: true,
    triggers: true,
    roles: true,
    sessions: true,
    transactions: true,
    savepoints: true,
    cancelQuery: true,
    explain: true,
    checkConstraints: true,
    generatedColumns: true,
    identityColumns: true,
    columnReorder: false,
    databaseRename: true,
    indexRename: true,
    queryTimeout: true,
  },
  actions: [],
};

const primaryProfile: DatabaseProfile = {
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
  readOnly: true,
  sslMode: "required",
  createdAt: 1,
  updatedAt: 1,
  lastUsedAt: 1,
};

const analyticsProfile: DatabaseProfile = {
  ...primaryProfile,
  id: "profile-2",
  name: "Analytics",
  databaseName: "warehouse",
};

const primarySnapshot: DatabaseConnectionSnapshot = {
  profileId: primaryProfile.id,
  serverId: "server-1",
  name: primaryProfile.name,
  engine: primaryProfile.engine,
  state: "connected",
  readOnly: true,
  databaseName: "app",
  version: { version: "16.2.0", raw: "PostgreSQL 16.2" },
  capabilities,
  changedAt: 10,
};

const analyticsSnapshot: DatabaseConnectionSnapshot = {
  ...primarySnapshot,
  profileId: analyticsProfile.id,
  name: analyticsProfile.name,
  databaseName: "warehouse",
};

const appDatabase: DatabaseSummary = {
  name: "app",
  system: false,
  ...metadata,
};

const analyticsDatabase: DatabaseSummary = {
  name: "analytics",
  system: false,
  ...metadata,
};

const publicSchema: DatabaseSchemaSummary = {
  name: "public",
  system: false,
  ...metadata,
};

const usersTable: DatabaseTableSummary = {
  database: "app",
  schema: "public",
  name: "users",
  kind: "table",
  estimatedRows: { status: "available", value: 250 },
  metadataRefreshedAt: unavailable,
  ...metadata,
};

const ordersTable: DatabaseTableSummary = {
  ...usersTable,
  name: "orders",
};

const activeUsersView: DatabaseViewSummary = {
  database: "app",
  schema: "public",
  name: "active_users",
  kind: "view",
  ...metadata,
};

const usersSequence: DatabaseSequenceSummary = {
  database: "app",
  schema: "public",
  name: "users_id_seq",
  dataType: { status: "available", value: "bigint" },
  startValue: { status: "available", value: "1" },
  increment: { status: "available", value: "1" },
  ...metadata,
};

function page<T>(items: T[]): DatabasePage<T> {
  return { items, limit: 100, offset: 0, hasMore: false, total: items.length };
}

const databasesPayload: DatabaseExplorerNodePayload = {
  nodeKind: "databases",
  payload: page([appDatabase, analyticsDatabase]),
};

const schemasPayload: DatabaseExplorerNodePayload = {
  nodeKind: "schemas",
  payload: page([publicSchema]),
};

const tablesPayload: DatabaseExplorerNodePayload = {
  nodeKind: "tables",
  payload: page([usersTable, ordersTable]),
};

const viewsPayload: DatabaseExplorerNodePayload = {
  nodeKind: "views",
  payload: page([activeUsersView]),
};

const sequencesPayload: DatabaseExplorerNodePayload = {
  nodeKind: "sequences",
  payload: page([usersSequence]),
};

function payloadForNode(node: DatabaseExplorerNode): DatabaseExplorerNodePayload {
  switch (node.kind) {
    case "databases":
      return databasesPayload;
    case "schemas":
      return schemasPayload;
    case "tables":
      return tablesPayload;
    case "views":
      return viewsPayload;
    case "sequences":
      return sequencesPayload;
    default:
      return {
        nodeKind: "objectCounts",
        payload: {
          tables: { status: "available", value: 2 },
          views: { status: "available", value: 1 },
          materializedViews: unavailable,
          sequences: { status: "available", value: 1 },
        },
      };
  }
}

function enableTauriRuntime() {
  Object.defineProperty(window, "__TAURI_INTERNALS__", {
    configurable: true,
    value: {},
  });
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

type HarnessProps = {
  profiles?: DatabaseProfile[];
  snapshots?: Record<string, DatabaseConnectionSnapshot>;
  onConnectProfile?: (profile: DatabaseProfile) => void;
};

function Harness({
  profiles = [primaryProfile],
  snapshots = { [primaryProfile.id]: primarySnapshot },
  onConnectProfile = vi.fn(),
}: HarnessProps) {
  const [selectedProfileId, setSelectedProfileId] = useState<string | null>(
    profiles[0]?.id ?? null,
  );
  const [selection, setSelection] = useState<DatabaseExplorerSelection | null>(null);

  return (
    <DatabaseExplorerTree
      profiles={profiles}
      profileStatus="ready"
      snapshots={snapshots}
      selectedProfileId={selectedProfileId}
      selection={selection}
      onSelectProfile={setSelectedProfileId}
      onSelectionChange={setSelection}
      onConnectProfile={onConnectProfile}
      onAddConnection={vi.fn()}
      onRetryProfiles={vi.fn()}
    />
  );
}

async function expandPrimary(user: ReturnType<typeof userEvent.setup>) {
  await user.click(screen.getByRole("treeitem", { name: /^Primary/ }));
  await screen.findByRole("treeitem", { name: /^app/ });
}

describe("DatabaseExplorerTree", () => {
  beforeEach(() => {
    enableTauriRuntime();
    vi.clearAllMocks();
    useDatabaseExplorerStore.setState({
      nodes: {},
      error: null,
      commandError: null,
    });
    vi.mocked(loadExplorerNode).mockImplementation(async (_profileId, node) =>
      payloadForNode(node),
    );
    vi.mocked(refreshExplorerNode).mockImplementation(async (_profileId, node) =>
      payloadForNode(node),
    );
  });

  it("lazily loads only the node that was expanded", async () => {
    const user = userEvent.setup();
    render(<Harness />);

    expect(loadExplorerNode).not.toHaveBeenCalled();

    await expandPrimary(user);
    expect(loadExplorerNode).toHaveBeenCalledTimes(1);
    expect(loadExplorerNode).toHaveBeenNthCalledWith(
      1,
      primaryProfile.id,
      { kind: "databases" },
      undefined,
    );

    await user.click(screen.getByRole("treeitem", { name: /^app/ }));
    expect(loadExplorerNode).toHaveBeenCalledTimes(1);
    await user.click(screen.getByRole("treeitem", { name: /^Schemas/ }));

    expect(loadExplorerNode).toHaveBeenCalledTimes(2);
    expect(loadExplorerNode).toHaveBeenNthCalledWith(
      2,
      primaryProfile.id,
      { kind: "schemas" },
      undefined,
    );

    await screen.findByRole("treeitem", { name: /^public/ });
    await user.click(screen.getByRole("treeitem", { name: /^public/ }));
    await user.click(screen.getByRole("treeitem", { name: /^Tables/ }));

    expect(loadExplorerNode).toHaveBeenCalledTimes(3);
    expect(loadExplorerNode).toHaveBeenNthCalledWith(
      3,
      primaryProfile.id,
      { kind: "tables", database: "app", schema: "public" },
      undefined,
    );
    expect(await screen.findByRole("treeitem", { name: /^users/ })).toBeInTheDocument();
  });

  it("refreshes one node in place and keeps the selected database selected", async () => {
    const user = userEvent.setup();
    render(<Harness />);

    await expandPrimary(user);
    const databaseRow = screen.getByRole("treeitem", { name: /^app/ });
    await user.click(databaseRow);
    expect(databaseRow).toHaveAttribute("aria-selected", "true");

    const refreshedPayload: DatabaseExplorerNodePayload = {
      nodeKind: "databases",
      payload: page([appDatabase, { ...analyticsDatabase, name: "analytics_archive" }]),
    };
    vi.mocked(refreshExplorerNode).mockResolvedValueOnce(refreshedPayload);

    await user.click(screen.getByRole("button", { name: "Refresh Primary" }));

    await waitFor(() => {
      expect(refreshExplorerNode).toHaveBeenCalledWith(
        primaryProfile.id,
        { kind: "databases" },
        expect.objectContaining({ offset: 0 }),
      );
    });
    expect(await screen.findByRole("treeitem", { name: /^analytics_archive/ })).toBeInTheDocument();
    expect(screen.getByRole("treeitem", { name: /^app/ })).toHaveAttribute("aria-selected", "true");
  });

  it("refreshes every loaded node with the tree action", async () => {
    const user = userEvent.setup();
    render(<Harness />);

    await expandPrimary(user);
    await user.click(screen.getByRole("treeitem", { name: /^app/ }));
    await user.click(screen.getByRole("treeitem", { name: /^Schemas/ }));
    await screen.findByRole("treeitem", { name: /^public/ });

    await user.click(screen.getByRole("button", { name: "Refresh entire tree" }));

    await waitFor(() => {
      const nodes = vi.mocked(refreshExplorerNode).mock.calls.map((call) => call[1]);
      expect(nodes).toEqual(expect.arrayContaining([{ kind: "databases" }, { kind: "schemas" }]));
    });
  });

  it("renders an empty node state without hiding the connection", async () => {
    const user = userEvent.setup();
    vi.mocked(loadExplorerNode).mockResolvedValueOnce({
      nodeKind: "databases",
      payload: page([]),
    });
    render(<Harness />);

    await user.click(screen.getByRole("treeitem", { name: /^Primary/ }));

    expect(await screen.findByText("No databases found.")).toBeInTheDocument();
    expect(screen.getByRole("treeitem", { name: /^Primary/ })).toBeInTheDocument();
  });

  it("shows a permission error in context and retries only that node", async () => {
    const user = userEvent.setup();
    vi.mocked(loadExplorerNode)
      .mockRejectedValueOnce({
        code: "permission",
        message: "The database account lacks USAGE on schema app.",
        details: "PostgreSQL SQLSTATE 42501.",
      })
      .mockResolvedValueOnce(databasesPayload);
    render(<Harness />);

    await user.click(screen.getByRole("treeitem", { name: /^Primary/ }));

    expect(await screen.findByText("Permission denied")).toBeInTheDocument();
    expect(screen.getByText("The database account lacks USAGE on schema app.")).toBeInTheDocument();
    expect(screen.getByRole("treeitem", { name: /^Primary/ })).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Try again" }));

    expect(await screen.findByRole("treeitem", { name: /^app/ })).toBeInTheDocument();
  });

  it("shows a disconnected node state without calling discovery", async () => {
    const user = userEvent.setup();
    const onConnectProfile = vi.fn();
    render(
      <Harness
        snapshots={{
          [primaryProfile.id]: {
            ...primarySnapshot,
            state: "disconnected",
          },
        }}
        onConnectProfile={onConnectProfile}
      />,
    );

    await user.click(screen.getByRole("treeitem", { name: /^Primary/ }));

    expect(screen.getByText("Connection required")).toBeInTheDocument();
    expect(screen.getByText(/Connect Primary over SSH to browse databases/)).toBeInTheDocument();
    expect(loadExplorerNode).not.toHaveBeenCalled();

    await user.click(screen.getByRole("button", { name: "Connect Primary" }));
    expect(onConnectProfile).toHaveBeenCalledWith(primaryProfile);
  });

  it("searches loaded branches and keeps their ancestors visible", async () => {
    const user = userEvent.setup();
    render(<Harness />);

    await expandPrimary(user);
    await user.type(
      screen.getByRole("searchbox", { name: "Search database explorer" }),
      "analytics",
    );

    expect(await screen.findByRole("treeitem", { name: /^analytics/ })).toBeInTheDocument();
    expect(screen.queryByRole("treeitem", { name: /^app/ })).not.toBeInTheDocument();
    expect(screen.getByRole("treeitem", { name: /^Primary/ })).toBeInTheDocument();

    await user.clear(screen.getByRole("searchbox", { name: "Search database explorer" }));
    expect(await screen.findByRole("treeitem", { name: /^app/ })).toBeInTheDocument();
  });

  it("navigates the tree with the keyboard and selects with Enter", async () => {
    const user = userEvent.setup();
    render(
      <Harness
        profiles={[primaryProfile, analyticsProfile]}
        snapshots={{
          [primaryProfile.id]: primarySnapshot,
          [analyticsProfile.id]: analyticsSnapshot,
        }}
      />,
    );

    const primaryRow = screen.getByRole("treeitem", { name: /^Primary/ });
    const analyticsRow = screen.getByRole("treeitem", { name: /^Analytics/ });
    act(() => primaryRow.focus());

    await user.keyboard("{ArrowDown}");
    await waitFor(() => expect(analyticsRow).toHaveFocus());

    await user.keyboard("{Home}");
    await waitFor(() => expect(primaryRow).toHaveFocus());

    await user.keyboard("{ArrowRight}");
    const databaseRow = await screen.findByRole("treeitem", { name: /^app/ });
    expect(loadExplorerNode).toHaveBeenCalledWith(
      primaryProfile.id,
      { kind: "databases" },
      undefined,
    );

    await user.keyboard("{ArrowRight}");
    await waitFor(() => expect(databaseRow).toHaveFocus());

    await user.keyboard("{ArrowLeft}");
    await waitFor(() => expect(primaryRow).toHaveFocus());

    await user.keyboard("{ArrowLeft}");
    await waitFor(() => expect(primaryRow).toHaveAttribute("aria-expanded", "false"));

    await user.keyboard("{Enter}");
    await waitFor(() => expect(primaryRow).toHaveAttribute("aria-selected", "true"));
  });

  it("virtualizes a large node so the whole result set is not rendered", async () => {
    const user = userEvent.setup();
    const largePayload: DatabaseExplorerNodePayload = {
      nodeKind: "databases",
      payload: page(
        Array.from({ length: 300 }, (_, index) => ({
          ...appDatabase,
          name: `db-${String(index).padStart(3, "0")}`,
        })),
      ),
    };
    vi.mocked(loadExplorerNode).mockResolvedValueOnce(largePayload);
    render(<Harness />);

    await user.click(screen.getByRole("treeitem", { name: /^Primary/ }));
    expect(await screen.findByRole("treeitem", { name: /^db-000/ })).toBeInTheDocument();

    const rendered = within(screen.getByRole("tree")).getAllByRole("treeitem");
    expect(rendered.length).toBeLessThan(100);
    expect(screen.queryByRole("treeitem", { name: /^db-299/ })).not.toBeInTheDocument();
  });

  it("shows a loading skeleton while a node is pending", async () => {
    const user = userEvent.setup();
    const pending = deferred<DatabaseExplorerNodePayload>();
    vi.mocked(loadExplorerNode).mockReturnValueOnce(pending.promise);
    render(<Harness />);

    await user.click(screen.getByRole("treeitem", { name: /^Primary/ }));

    expect(screen.getByRole("status", { name: "Loading databases" })).toBeInTheDocument();

    pending.resolve(databasesPayload);
    expect(await screen.findByRole("treeitem", { name: /^app/ })).toBeInTheDocument();
  });
});
