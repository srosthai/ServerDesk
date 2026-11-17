import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { DatabaseSessionsPanel } from "@/features/databases/DatabaseSessionsPanel";
import { listDatabaseSessions, runDatabaseSessionAction } from "@/services/databaseSessions";
import { useDatabaseSessionStore } from "@/stores/databaseSessionStore";
import type {
  CapabilityFlags,
  DatabasePage,
  DatabaseSessionActionResult,
  DatabaseSessionSummary,
} from "@/types/contracts";

vi.mock("@/services/databaseSessions", () => ({
  listDatabaseSessions: vi.fn(),
  runDatabaseSessionAction: vi.fn(),
}));

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

const running: DatabaseSessionSummary = {
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

const idle: DatabaseSessionSummary = {
  id: 502,
  user: "reporting",
  host: { status: "unavailable" },
  database: { status: "available", value: "app" },
  applicationName: { status: "unavailable" },
  command: { status: "unavailable" },
  state: { status: "available", value: "idle" },
  waitEvent: { status: "unavailable" },
  backendType: { status: "unavailable" },
  connectedAt: { status: "unavailable" },
  isCurrent: false,
  query: { status: "unavailable" },
};

const current: DatabaseSessionSummary = {
  ...running,
  id: 503,
  user: "serverdesk",
  isCurrent: true,
  applicationName: { status: "available", value: "serverdesk" },
  query: {
    status: "available",
    value: {
      text: "SELECT 1",
      redacted: false,
      truncated: false,
      startedAt: { status: "available", value: "2026-11-17T08:05:00Z" },
      elapsedMs: { status: "available", value: 5 },
    },
  },
};

const redacted: DatabaseSessionSummary = {
  ...running,
  id: 504,
  user: "admin",
  applicationName: { status: "available", value: "psql-admin" },
  query: {
    status: "available",
    value: {
      text: "ALTER ROLE reporting WITH PASSWORD '<redacted>'",
      redacted: true,
      truncated: false,
      startedAt: { status: "available", value: "2026-11-17T08:05:00Z" },
      elapsedMs: { status: "available", value: 1_200 },
    },
  },
};

const page: DatabasePage<DatabaseSessionSummary> = {
  items: [running, idle, current, redacted],
  limit: 200,
  offset: 0,
  hasMore: false,
  total: 4,
};

const verifiedResult: DatabaseSessionActionResult = {
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

function renderPanel(readOnly = false) {
  return render(
    <DatabaseSessionsPanel
      profileId="profile-1"
      profileName="Primary"
      engine="postgresql"
      version="16.2.0"
      connected
      readOnly={readOnly}
      flags={flags}
    />,
  );
}

function rowFor(sessionId: number): HTMLElement {
  const row = screen.getByText(String(sessionId)).closest("tr");
  if (!row) {
    throw new Error(`row ${sessionId} not found`);
  }
  return row;
}

describe("DatabaseSessionsPanel", () => {
  beforeEach(() => {
    useDatabaseSessionStore.setState({ entries: {} });
    vi.mocked(listDatabaseSessions).mockReset();
    vi.mocked(runDatabaseSessionAction).mockReset();
    vi.mocked(listDatabaseSessions).mockResolvedValue(page);
  });

  it("renders sessions, elapsed time, unsupported fields, and redaction", async () => {
    renderPanel();

    await waitFor(() => expect(screen.getByText("SELECT pg_sleep(30)")).toBeInTheDocument());
    expect(screen.getByText("30.0 s")).toBeInTheDocument();
    expect(screen.getByText("psql")).toBeInTheDocument();
    expect(screen.getByText("No running query")).toBeInTheDocument();
    expect(screen.getAllByText("Unavailable").length).toBeGreaterThan(0);
    expect(screen.getByText("Current")).toBeInTheDocument();
    expect(screen.getByText("Redacted")).toBeInTheDocument();
    expect(screen.getByText(/ALTER ROLE reporting/)).toBeInTheDocument();
  });

  it("keeps the table visible while a refresh is in flight", async () => {
    const user = userEvent.setup();
    renderPanel();
    await waitFor(() => expect(screen.getByText("SELECT pg_sleep(30)")).toBeInTheDocument());

    let resolveRefresh: (value: DatabasePage<DatabaseSessionSummary>) => void = () => {};
    vi.mocked(listDatabaseSessions).mockImplementation(
      () =>
        new Promise<DatabasePage<DatabaseSessionSummary>>((resolve) => {
          resolveRefresh = resolve;
        }),
    );

    await user.click(screen.getByRole("button", { name: "Refresh" }));

    expect(screen.getByRole("button", { name: "Refreshing..." })).toBeDisabled();
    expect(screen.getByText("SELECT pg_sleep(30)")).toBeInTheDocument();

    resolveRefresh(page);
    await waitFor(() => expect(screen.getByRole("button", { name: "Refresh" })).toBeEnabled());
  });

  it("disables cancellation and termination on a read-only profile", async () => {
    renderPanel(true);

    await waitFor(() => expect(screen.getByText("SELECT pg_sleep(30)")).toBeInTheDocument());
    expect(screen.getByText("Read-only profile")).toBeInTheDocument();
    const row = rowFor(501);
    expect(within(row).getByRole("button", { name: "Cancel query" })).toBeDisabled();
    expect(within(row).getByRole("button", { name: "Terminate session" })).toBeDisabled();
  });

  it("requires the exact confirmation phrase before terminating", async () => {
    const user = userEvent.setup();
    vi.mocked(runDatabaseSessionAction).mockResolvedValue(verifiedResult);
    renderPanel();
    await waitFor(() => expect(screen.getByText("SELECT pg_sleep(30)")).toBeInTheDocument());

    await user.click(within(rowFor(501)).getByRole("button", { name: "Terminate session" }));
    const dialog = screen.getByRole("dialog");
    const confirm = within(dialog).getByRole("button", { name: "Terminate session" });
    expect(confirm).toBeDisabled();

    await user.type(
      within(dialog).getByLabelText(/Type terminate session 501/),
      "terminate session 501",
    );
    expect(confirm).toBeEnabled();
    await user.click(confirm);

    await waitFor(() =>
      expect(runDatabaseSessionAction).toHaveBeenCalledWith("profile-1", {
        sessionId: 501,
        action: "terminate_session",
        expectedUser: "app",
        expectedDatabase: "app",
        confirmation: "terminate session 501",
      }),
    );
    expect(await screen.findByText("Terminate session verified")).toBeInTheDocument();
  });

  it("shows a permission denial without claiming success", async () => {
    const user = userEvent.setup();
    vi.mocked(runDatabaseSessionAction).mockRejectedValue({
      code: "permission",
      message: "The database account does not have permission for this operation.",
    });
    renderPanel();
    await waitFor(() => expect(screen.getByText("SELECT pg_sleep(30)")).toBeInTheDocument());

    await user.click(within(rowFor(501)).getByRole("button", { name: "Cancel query" }));
    const dialog = screen.getByRole("dialog");
    await user.type(within(dialog).getByLabelText(/Type cancel query 501/), "cancel query 501");
    await user.click(within(dialog).getByRole("button", { name: "Cancel query" }));

    expect(await screen.findByText("The action did not complete")).toBeInTheDocument();
    expect(
      screen.getByText("The database account does not have permission for this operation."),
    ).toBeInTheDocument();
    expect(screen.queryByText("Cancel query verified")).not.toBeInTheDocument();
  });

  it("exposes a bounded auto-refresh control", async () => {
    const user = userEvent.setup();
    renderPanel();
    await waitFor(() => expect(screen.getByText("SELECT pg_sleep(30)")).toBeInTheDocument());

    await user.click(screen.getByRole("button", { name: "Auto-refresh off" }));

    expect(screen.getByRole("button", { name: "Auto-refresh on" })).toHaveAttribute(
      "aria-pressed",
      "true",
    );
    expect(screen.getByText(/stops after 60 refreshes/)).toBeInTheDocument();
  });
});
