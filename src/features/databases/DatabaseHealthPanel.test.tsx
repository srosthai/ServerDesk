import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { DatabaseHealthPanel } from "@/features/databases/DatabaseHealthPanel";
import { getDatabaseHealth } from "@/services/databaseHealth";
import { useDatabaseHealthStore } from "@/stores/databaseHealthStore";
import type { DatabaseHealthSnapshot } from "@/types/contracts";

vi.mock("@/services/databaseHealth", () => ({
  getDatabaseHealth: vi.fn(),
}));

const NOW = Date.now();

const snapshot: DatabaseHealthSnapshot = {
  profileId: "profile-1",
  engine: "postgresql",
  database: "app",
  version: { version: "16.2.0", raw: "PostgreSQL 16.2" },
  sampledAt: NOW - 30_000,
  latencyMs: { status: "available", value: 14 },
  databaseSizeBytes: { status: "available", value: 1_048_576 },
  tableSizes: {
    status: "available",
    value: {
      items: [
        {
          database: "app",
          schema: "public",
          name: "orders",
          kind: "table",
          sizeBytes: { status: "available", value: 32_768 },
          estimatedRows: { status: "available", value: 8 },
          metadataRefreshedAt: { status: "available", value: new Date(NOW - 60_000).toISOString() },
        },
        {
          database: "app",
          schema: "public",
          name: "users",
          kind: "table",
          sizeBytes: { status: "available", value: 16_384 },
          estimatedRows: { status: "available", value: 42 },
          metadataRefreshedAt: { status: "unavailable" },
        },
      ],
      limit: 12,
      truncated: false,
    },
  },
};

function sample(overrides: Partial<DatabaseHealthSnapshot> = {}): DatabaseHealthSnapshot {
  return { ...snapshot, ...overrides };
}

function renderPanel(overrides: Partial<Parameters<typeof DatabaseHealthPanel>[0]> = {}) {
  return render(
    <DatabaseHealthPanel
      profileId="profile-1"
      profileName="Primary"
      engine="postgresql"
      connected
      selectedDatabase="app"
      {...overrides}
    />,
  );
}

describe("DatabaseHealthPanel", () => {
  beforeEach(() => {
    useDatabaseHealthStore.setState({ entries: {} });
    vi.clearAllMocks();
  });

  it("renders version, latency, sample time, estimates, and table sizes", async () => {
    vi.mocked(getDatabaseHealth).mockResolvedValue(snapshot);
    renderPanel();

    expect(await screen.findByRole("region", { name: "Health indicators" })).toBeInTheDocument();
    const summary = screen.getByRole("region", { name: "Health sample summary" });
    expect(withinText(summary, "16.2.0")).toBe(true);
    expect(withinText(summary, "14 ms")).toBe(true);
    expect(withinText(summary, "Last sample")).toBe(true);
    const sizes = screen.getByRole("region", { name: "Table sizes" });
    expect(withinText(sizes, "public.orders")).toBe(true);
    expect(withinText(sizes, "32 KB")).toBe(true);
    expect(withinText(sizes, "~8 est. rows")).toBe(true);
    expect(screen.getAllByText("Estimated").length).toBeGreaterThan(0);
    expect(screen.getAllByText("Measured").length).toBeGreaterThan(0);
    expect(getDatabaseHealth).toHaveBeenCalledWith("profile-1", 12);
  });

  it("takes a fresh sample from the manual refresh button", async () => {
    const user = userEvent.setup();
    vi.mocked(getDatabaseHealth)
      .mockResolvedValueOnce(snapshot)
      .mockResolvedValueOnce(
        sample({ sampledAt: NOW - 1_000, latencyMs: { status: "available", value: 8 } }),
      );
    renderPanel();

    await screen.findByRole("region", { name: "Health indicators" });
    await user.click(screen.getByRole("button", { name: "Refresh health sample" }));

    await waitFor(() => {
      expect(getDatabaseHealth).toHaveBeenCalledTimes(2);
    });
    const summary = screen.getByRole("region", { name: "Health sample summary" });
    expect(withinText(summary, "8 ms")).toBe(true);
  });

  it("marks an old sample stale and shows unavailable metrics honestly", async () => {
    vi.mocked(getDatabaseHealth).mockResolvedValue(
      sample({
        sampledAt: NOW - 10 * 60_000,
        latencyMs: { status: "unavailable" },
        databaseSizeBytes: { status: "unavailable" },
        tableSizes: { status: "unavailable" },
      }),
    );
    renderPanel();

    expect(await screen.findByText("This health sample is stale")).toBeInTheDocument();
    expect(screen.getByRole("region", { name: "Table sizes" })).toHaveTextContent(
      "Table sizes are unavailable",
    );
    expect(screen.queryByText("No health sample yet")).not.toBeInTheDocument();
    expect(document.body.textContent).not.toContain("0 B");
  });

  it("explains when the explorer selection differs from the active database", async () => {
    vi.mocked(getDatabaseHealth).mockResolvedValue(snapshot);
    renderPanel({ selectedDatabase: "reporting" });

    expect(await screen.findByText("Health follows the active database")).toBeInTheDocument();
    expect(screen.getByText(/This sample is for app/)).toBeInTheDocument();
  });

  it("requires a live connection before sampling", () => {
    renderPanel({ connected: false });

    expect(screen.getByText("Connection required")).toBeInTheDocument();
    expect(getDatabaseHealth).not.toHaveBeenCalled();
  });
});

function withinText(element: HTMLElement, text: string): boolean {
  return (element.textContent ?? "").includes(text);
}
