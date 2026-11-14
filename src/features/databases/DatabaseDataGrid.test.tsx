import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { ComponentProps } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { DatabaseDataGrid } from "@/features/databases/DatabaseDataGrid";
import { columnWidth } from "@/features/databases/databaseDataGridModel";
import {
  cancelTableRows,
  createDatabaseRowRequestId,
  readTableRows,
} from "@/services/databaseRows";
import { useDatabaseRowStore } from "@/stores/databaseRowStore";
import { useDatabaseWorkspaceStore } from "@/stores/databaseWorkspaceStore";
import type {
  DatabaseProfile,
  DatabaseRowCell,
  DatabaseRowColumn,
  DatabaseRowIdentity,
  DatabaseRowOptions,
  DatabaseRowReadResult,
  DatabaseRowRecord,
} from "@/types/contracts";

vi.mock("@/services/databaseRows", () => ({
  readTableRows: vi.fn(),
  cancelTableRows: vi.fn(),
  createDatabaseRowRequestId: vi.fn(),
}));

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
  sslMode: "required",
  createdAt: 1,
  updatedAt: 1,
  lastUsedAt: null,
};

const columns: DatabaseRowColumn[] = [
  {
    name: "id",
    dataType: "bigint",
    kind: "integer",
    nullable: false,
    ordinalPosition: 1,
    primaryKey: true,
    identity: "by_default",
    generated: false,
    sortable: true,
    filterable: true,
    searchable: false,
  },
  {
    name: "name",
    dataType: "text",
    kind: "text",
    nullable: true,
    ordinalPosition: 2,
    primaryKey: false,
    identity: "none",
    generated: false,
    sortable: true,
    filterable: true,
    searchable: true,
  },
  {
    name: "active",
    dataType: "boolean",
    kind: "boolean",
    nullable: false,
    ordinalPosition: 3,
    primaryKey: false,
    identity: "none",
    generated: false,
    sortable: true,
    filterable: true,
    searchable: false,
  },
  {
    name: "balance",
    dataType: "numeric",
    kind: "decimal",
    nullable: true,
    ordinalPosition: 4,
    primaryKey: false,
    identity: "none",
    generated: false,
    sortable: true,
    filterable: true,
    searchable: false,
  },
  {
    name: "created_at",
    dataType: "timestamp with time zone",
    kind: "temporal",
    nullable: false,
    ordinalPosition: 5,
    primaryKey: false,
    identity: "none",
    generated: false,
    sortable: true,
    filterable: true,
    searchable: false,
  },
  {
    name: "metadata",
    dataType: "jsonb",
    kind: "json",
    nullable: true,
    ordinalPosition: 6,
    primaryKey: false,
    identity: "none",
    generated: false,
    sortable: false,
    filterable: false,
    searchable: false,
  },
  {
    name: "avatar",
    dataType: "bytea",
    kind: "binary",
    nullable: true,
    ordinalPosition: 7,
    primaryKey: false,
    identity: "none",
    generated: false,
    sortable: false,
    filterable: false,
    searchable: false,
  },
  {
    name: "notes",
    dataType: "text",
    kind: "text",
    nullable: true,
    ordinalPosition: 8,
    primaryKey: false,
    identity: "none",
    generated: false,
    sortable: false,
    filterable: true,
    searchable: true,
  },
];

function record(
  id: string,
  name: string | null,
  active: string,
  balance: string | null,
  createdAt: string,
  metadata: string | null,
  avatar: string | null,
  notes: string | null,
): DatabaseRowRecord {
  const cells: DatabaseRowCell[] = [
    { value: id, truncated: false },
    { value: name, truncated: false },
    { value: active, truncated: false },
    { value: balance, truncated: false },
    { value: createdAt, truncated: false },
    { value: metadata, truncated: false },
    { value: avatar, truncated: false },
    { value: notes, truncated: false },
  ];
  return {
    cells,
    identity: [{ column: "id", value: id }],
  };
}

type Fixture = {
  columns: DatabaseRowColumn[];
  rows: DatabaseRowRecord[];
  identity: DatabaseRowIdentity;
  editable: boolean;
  readOnlyReasons: DatabaseRowReadResult["readOnlyReasons"];
  estimatedRows: number | null;
  exactRows?: number;
  hasMore: boolean;
};

let fixture: Fixture;

function resetFixture() {
  fixture = {
    columns,
    rows: [
      record(
        "1",
        "ann",
        "true",
        "10.50",
        "2026-02-01 12:00:00+00",
        '{"ok":true}',
        "AAECQUJD",
        "short note",
      ),
      record("2", null, "false", "0.00", "2026-02-02 09:30:00+00", '{"nested":{"a":1}}', null, ""),
    ],
    identity: { kind: "primary_key", columns: ["id"], stable: true },
    editable: true,
    readOnlyReasons: [],
    estimatedRows: 12_000,
    hasMore: false,
  };
}

function resultFor(options: DatabaseRowOptions): DatabaseRowReadResult {
  const allNames = fixture.columns.map((column) => column.name);
  const visibleNames = options.visibleColumns ?? allNames;
  const positions = visibleNames
    .map((name) => allNames.indexOf(name))
    .filter((index) => index >= 0);
  const rows = fixture.rows.map((row) => ({
    identity: row.identity,
    cells: positions.map((index) => row.cells[index] ?? { value: null, truncated: false }),
  }));
  return {
    engine: "postgresql",
    table: { database: "app", schema: "public", name: "users" },
    columns: fixture.columns,
    visibleColumns: visibleNames,
    identity: fixture.identity,
    editable: fixture.editable,
    readOnlyReasons: fixture.readOnlyReasons,
    rows,
    page: {
      limit: options.limit ?? 100,
      offset: options.offset ?? 0,
      returned: rows.length,
      hasMore: fixture.hasMore,
    },
    counts: {
      estimatedTableRows:
        fixture.estimatedRows === null
          ? { status: "unavailable" }
          : { status: "available", value: fixture.estimatedRows },
      ...(options.includeExactCount
        ? {
            exactRows: {
              value: fixture.exactRows ?? 125,
              filtered: (options.filters?.length ?? 0) > 0 || Boolean(options.search),
            },
          }
        : {}),
    },
  };
}

function lastOptions(): DatabaseRowOptions {
  const calls = vi.mocked(readTableRows).mock.calls;
  return calls[calls.length - 1][2] ?? {};
}

function renderGrid(overrides: Partial<ComponentProps<typeof DatabaseDataGrid>> = {}) {
  return render(
    <DatabaseDataGrid
      profileId={profile.id}
      profileName={profile.name}
      database="app"
      schema="public"
      table="users"
      connected
      readOnly={false}
      onConnect={vi.fn()}
      {...overrides}
    />,
  );
}

describe("DatabaseDataGrid", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    Object.defineProperty(window, "__TAURI_INTERNALS__", {
      configurable: true,
      value: {},
    });
    resetFixture();
    useDatabaseWorkspaceStore.setState({ tabs: {}, order: [], activeTabId: null });
    useDatabaseWorkspaceStore.getState().openTab(profile);
    useDatabaseRowStore.setState({ entries: {} });
    let requestId = 0;
    vi.mocked(createDatabaseRowRequestId).mockImplementation(() => {
      requestId += 1;
      return `row-request-${requestId}`;
    });
    vi.mocked(readTableRows).mockImplementation((_profileId, _table, options) =>
      Promise.resolve(resultFor(options ?? {})),
    );
    vi.mocked(cancelTableRows).mockResolvedValue({
      requestId: "row-request-1",
      cancelled: true,
    });
  });

  it("loads one bounded page and renders typed values, NULL, and empty strings", async () => {
    renderGrid();

    expect(await screen.findByRole("grid", { name: "Table data for users" })).toBeInTheDocument();
    expect(lastOptions()).toMatchObject({
      limit: 100,
      offset: 0,
      sort: [],
      filters: [],
      search: null,
      includeExactCount: false,
      visibleColumns: undefined,
    });

    expect(screen.getByText("ann")).toBeInTheDocument();
    expect(screen.getAllByText("NULL").length).toBeGreaterThan(0);
    expect(screen.getByText('"" empty')).toBeInTheDocument();
    expect(screen.getByText("true")).toBeInTheDocument();
    expect(screen.getByText("false")).toBeInTheDocument();
    expect(screen.getByText("~12,000 est. rows")).toBeInTheDocument();
    expect(screen.getByText(/Sampled /)).toBeInTheDocument();
    expect(screen.getByText("Binary · 6 B")).toBeInTheDocument();
    expect(screen.queryByText("12,000 exact")).not.toBeInTheDocument();
  });

  it("pages forward and back on the server without loading the table", async () => {
    const user = userEvent.setup();
    fixture.hasMore = true;
    renderGrid();

    await screen.findByRole("grid", { name: "Table data for users" });
    await user.click(screen.getByRole("button", { name: "Next page" }));

    await waitFor(() => {
      expect(lastOptions().offset).toBe(100);
    });
    expect(screen.getByText(/Rows 101–102/)).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Previous page" }));
    await waitFor(() => {
      expect(lastOptions().offset).toBe(0);
    });
  });

  it("changes page size and resets the offset to the first page", async () => {
    const user = userEvent.setup();
    renderGrid();

    await screen.findByRole("grid", { name: "Table data for users" });
    await user.click(screen.getByRole("combobox", { name: "Rows per page" }));
    await user.click(await screen.findByRole("option", { name: "25" }));

    await waitFor(() => {
      expect(lastOptions()).toMatchObject({ limit: 25, offset: 0 });
    });
  });

  it("sorts on the server and resets a deep page to a safe offset", async () => {
    const user = userEvent.setup();
    fixture.hasMore = true;
    renderGrid();

    await screen.findByRole("grid", { name: "Table data for users" });
    await user.click(screen.getByRole("button", { name: "Next page" }));
    await waitFor(() => expect(lastOptions().offset).toBe(100));

    await user.click(screen.getByRole("button", { name: "Sort by name" }));
    await waitFor(() => {
      expect(lastOptions()).toMatchObject({
        offset: 0,
        sort: [{ column: "name", direction: "asc" }],
      });
    });
    expect(screen.getByRole("columnheader", { name: /name/ })).toHaveAttribute(
      "aria-sort",
      "ascending",
    );

    await user.click(screen.getByRole("button", { name: "Sort by name" }));
    await waitFor(() => {
      expect(lastOptions().sort).toEqual([{ column: "name", direction: "desc" }]);
    });
    expect(screen.getByRole("columnheader", { name: /name/ })).toHaveAttribute(
      "aria-sort",
      "descending",
    );
  });

  it("applies a typed server-side filter and resets to the first page", async () => {
    const user = userEvent.setup();
    fixture.hasMore = true;
    fixture.rows = [
      record(
        "1",
        "ann",
        "true",
        "10.50",
        "2026-02-01 12:00:00+00",
        '{"ok":true}',
        "AAECQUJD",
        "short note",
      ),
    ];
    renderGrid();

    await screen.findByRole("grid", { name: "Table data for users" });
    await user.click(screen.getByRole("button", { name: "Next page" }));
    await waitFor(() => expect(lastOptions().offset).toBe(100));

    await user.click(screen.getByRole("button", { name: "Table filters" }));
    await user.click(screen.getByRole("button", { name: "Add filter" }));
    await user.click(screen.getByRole("combobox", { name: "Filter column 1" }));
    await user.click(await screen.findByRole("option", { name: "name" }));
    await user.click(screen.getByRole("combobox", { name: "Filter operator 1" }));
    await user.click(await screen.findByRole("option", { name: "contains" }));
    await user.type(screen.getByLabelText("Filter value 1"), "ann");
    await user.click(screen.getByRole("button", { name: "Apply filters" }));

    await waitFor(() => {
      expect(lastOptions()).toMatchObject({
        offset: 0,
        filters: [
          {
            column: "name",
            operator: "contains",
            value: { kind: "text", value: "ann" },
          },
        ],
      });
    });
    expect(screen.getByText('name contains "ann"')).toBeInTheDocument();
  });

  it("rejects an invalid typed filter value without sending it", async () => {
    const user = userEvent.setup();
    renderGrid();

    await screen.findByRole("grid", { name: "Table data for users" });
    const callsBefore = vi.mocked(readTableRows).mock.calls.length;
    await user.click(screen.getByRole("button", { name: "Table filters" }));
    await user.click(screen.getByRole("button", { name: "Add filter" }));
    await user.click(screen.getByRole("combobox", { name: "Filter operator 1" }));
    await user.click(await screen.findByRole("option", { name: ">" }));
    await user.type(screen.getByLabelText("Filter value 1"), "not-a-number");
    await user.click(screen.getByRole("button", { name: "Apply filters" }));

    expect(await screen.findByRole("alert")).toHaveTextContent("id: Enter a whole number.");
    expect(vi.mocked(readTableRows).mock.calls).toHaveLength(callsBefore);
  });

  it("applies search and exact count options on the server", async () => {
    const user = userEvent.setup();
    renderGrid();

    await screen.findByRole("grid", { name: "Table data for users" });
    await user.type(screen.getByLabelText("Search text columns"), "ann");
    await user.click(screen.getByRole("button", { name: "Apply search" }));
    await waitFor(() => {
      expect(lastOptions()).toMatchObject({
        offset: 0,
        search: { query: "ann", columns: [] },
      });
    });

    await user.click(screen.getByLabelText("Include exact row count"));
    await waitFor(() => {
      expect(lastOptions().includeExactCount).toBe(true);
    });
    expect(await screen.findByText("125 exact filtered")).toBeInTheDocument();
  });

  it("hides columns and adjusts widths without fetching hidden values", async () => {
    const user = userEvent.setup();
    renderGrid();

    await screen.findByRole("grid", { name: "Table data for users" });
    await user.click(screen.getByRole("button", { name: "Choose columns and widths" }));
    const dialog = await screen.findByRole("dialog", { name: "Columns and widths" });
    await user.click(within(dialog).getByLabelText("Show column notes"));
    const width = within(dialog).getByLabelText("Width for name");
    fireEvent.change(width, { target: { value: "240" } });
    await user.click(within(dialog).getByRole("button", { name: "Apply columns" }));

    await waitFor(() => {
      expect(lastOptions()).toMatchObject({
        offset: 0,
        visibleColumns: ["id", "name", "active", "balance", "created_at", "metadata", "avatar"],
      });
    });
    expect(screen.queryByRole("button", { name: "Sort by notes" })).not.toBeInTheDocument();
    const state = useDatabaseWorkspaceStore.getState();
    const tableKey = Object.keys(state.tabs[profile.id].dataGrid)[0];
    expect(state.tabs[profile.id].dataGrid[tableKey].columnWidths.name).toBe(240);
  });

  it("resizes a column from the keyboard and keeps it scoped to this tab", async () => {
    const user = userEvent.setup();
    renderGrid();

    await screen.findByRole("grid", { name: "Table data for users" });
    const handle = screen.getByRole("separator", { name: "Resize name column" });
    const nameColumn = columns.find((column) => column.name === "name");
    expect(nameColumn).toBeDefined();
    if (!nameColumn) {
      return;
    }
    handle.focus();
    await user.keyboard("{ArrowRight}");

    const state = useDatabaseWorkspaceStore.getState();
    const tableKey = Object.keys(state.tabs[profile.id].dataGrid)[0];
    expect(state.tabs[profile.id].dataGrid[tableKey].columnWidths.name).toBe(
      columnWidth(nameColumn, {}) + 16,
    );
  });

  it("shows only the preview in the grid and the full value in the safe viewer", async () => {
    const user = userEvent.setup();
    const longNotes = "x".repeat(600);
    const longJson = JSON.stringify({ message: longNotes, nested: { ok: true } });
    fixture.rows = [
      record(
        "1",
        "ann",
        "true",
        "10.50",
        "2026-02-01 12:00:00+00",
        longJson,
        "AAECQUJD",
        longNotes,
      ),
    ];
    renderGrid();

    const grid = await screen.findByRole("grid", { name: "Table data for users" });
    expect(within(grid).queryByText(longNotes)).not.toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "View notes value in row 1" }));
    const textDialog = await screen.findByRole("dialog", { name: "notes value" });
    expect(within(textDialog).getByLabelText("Full value for notes")).toHaveTextContent(longNotes);

    const closeButtons = within(textDialog).getAllByRole("button", { name: "Close" });
    await user.click(closeButtons[closeButtons.length - 1]);
    await user.click(screen.getByRole("button", { name: "View metadata value in row 1" }));
    const jsonDialog = await screen.findByRole("dialog", { name: "metadata value" });
    expect(within(jsonDialog).getByText("Valid JSON")).toBeInTheDocument();
    expect(within(jsonDialog).getByLabelText("Full value for metadata")).toHaveTextContent(
      '"message": "',
    );
  });

  it("warns that a server-truncated value is incomplete instead of truncating silently", async () => {
    const user = userEvent.setup();
    fixture.rows = [
      {
        identity: [{ column: "id", value: "1" }],
        cells: [
          { value: "1", truncated: false },
          { value: "ann", truncated: false },
          { value: "true", truncated: false },
          { value: "10.50", truncated: false },
          { value: "2026-02-01 12:00:00+00", truncated: false },
          { value: '{"a":', truncated: true },
          { value: null, truncated: false },
          { value: "x".repeat(4_096), truncated: true },
        ],
      },
    ];
    renderGrid();

    await screen.findByRole("grid", { name: "Table data for users" });
    await user.click(screen.getByRole("button", { name: "View notes value in row 1" }));
    const dialog = await screen.findByRole("dialog", { name: "notes value" });
    expect(within(dialog).getByText("Truncated by server")).toBeInTheDocument();
    expect(within(dialog).getByRole("note")).toHaveTextContent("remainder was not read");
  });

  it("shows the empty state and can recover from a past-the-end page", async () => {
    const user = userEvent.setup();
    fixture.rows = [];
    renderGrid();

    expect(await screen.findByText("This table has no rows")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Table filters" }));
    await user.click(screen.getByRole("button", { name: "Add filter" }));
    await user.click(screen.getByRole("combobox", { name: "Filter column 1" }));
    await user.click(await screen.findByRole("option", { name: "name" }));
    await user.click(screen.getByRole("button", { name: "Apply filters" }));

    expect(await screen.findByText("No rows match")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Clear filters" }));
    expect(await screen.findByText("This table has no rows")).toBeInTheDocument();
  });

  it("shows an error state with a retry action", async () => {
    const user = userEvent.setup();
    vi.mocked(readTableRows).mockRejectedValueOnce({
      code: "query",
      message: "The database rejected the row read.",
    });
    renderGrid();

    expect(await screen.findByText("Could not read table rows")).toBeInTheDocument();
    expect(screen.getByText("The database rejected the row read.")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Try again" }));
    expect(await screen.findByRole("grid", { name: "Table data for users" })).toBeInTheDocument();
  });

  it("does not read rows while disconnected", async () => {
    renderGrid({ connected: false });

    expect(screen.getByText("Connection required")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Connect Primary" })).toBeInTheDocument();
    expect(readTableRows).not.toHaveBeenCalled();
  });

  it("disables row editing when a table has no stable identity", async () => {
    fixture.identity = { kind: "none", columns: [], stable: false };
    fixture.editable = false;
    fixture.readOnlyReasons = ["no_stable_identity"];
    renderGrid();

    await screen.findByRole("grid", { name: "Table data for users" });
    expect(screen.getByRole("note", { name: "Row editing unavailable" })).toHaveTextContent(
      "no primary key or complete unique index",
    );
    expect(screen.getByRole("button", { name: "Edit row" })).toBeDisabled();
    expect(screen.getByText("No stable identity")).toBeInTheDocument();
  });

  it("supports keyboard navigation and column selection", async () => {
    const user = userEvent.setup();
    renderGrid();

    const grid = await screen.findByRole("grid", { name: "Table data for users" });
    grid.focus();
    await user.keyboard("{ArrowRight}{ArrowDown}{ArrowUp}");

    const activeId = grid.getAttribute("aria-activedescendant");
    expect(activeId).toBeTruthy();
    const activeCell = activeId ? document.getElementById(activeId) : null;
    expect(activeCell).not.toBeNull();
    expect(activeCell).toHaveAttribute("role", "gridcell");
    expect(activeCell).toHaveAttribute("aria-selected", "true");

    const nameHeader = screen.getByRole("columnheader", { name: /name/ });
    expect(nameHeader).toHaveAttribute("aria-selected", "true");

    await user.keyboard("{Enter}");
    expect(await screen.findByRole("dialog", { name: "name value" })).toBeInTheDocument();
  });
});
