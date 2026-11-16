import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";
import { useDatabaseOperationTaskSync } from "@/hooks/useDatabaseOperationTaskSync";
import { useDatabaseBackupStore } from "@/stores/databaseBackupStore";
import { useDatabaseCsvStore } from "@/stores/databaseCsvStore";
import { useDatabaseDumpStore } from "@/stores/databaseDumpStore";
import { useDatabaseWorkspaceStore } from "@/stores/databaseWorkspaceStore";
import type { DatabaseCsvExportResult, DatabaseProfile } from "@/types/contracts";

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
  lastUsedAt: 1_700_000_000,
};

function exportResult(): DatabaseCsvExportResult {
  return {
    status: "completed",
    engine: "postgresql",
    table: { database: "app", schema: "public", name: "users" },
    destinationPath: "/tmp/users.csv",
    rowsExported: 12,
    columns: ["name"],
    bytesWritten: 2048,
    encoding: "utf8",
    delimiter: "comma",
    includeHeader: true,
    warnings: [],
    mayBeIncomplete: false,
    valuesTruncated: false,
    durationMs: 1500,
  };
}

describe("useDatabaseOperationTaskSync", () => {
  beforeEach(() => {
    useDatabaseWorkspaceStore.setState({ tabs: {}, order: [], activeTabId: null });
    useDatabaseWorkspaceStore.getState().openTab(profile);
    useDatabaseCsvStore.setState({
      preview: { status: "idle" },
      transfer: { direction: "export", status: "idle", cancelling: false },
    });
    useDatabaseDumpStore.setState({
      tools: { status: "idle" },
      transfer: { direction: "export", status: "idle", cancelling: false },
    });
    useDatabaseBackupStore.setState({
      tools: { status: "idle" },
      preview: { status: "idle" },
      transfer: { direction: "backup", status: "idle", cancelling: false },
    });
  });

  it("finishes a background task after its wizard has closed", async () => {
    useDatabaseWorkspaceStore.getState().addTask(profile.id, {
      id: "csv-1",
      label: "CSV table export · users",
      status: "running",
      detail: "Preparing the CSV export",
      startedAt: 1,
    });
    useDatabaseCsvStore.setState({
      transfer: {
        transferId: "csv-1",
        profileId: profile.id,
        direction: "export",
        status: "running",
        cancelling: false,
        startedAt: 1,
      },
    });

    renderHook(() => useDatabaseOperationTaskSync());
    act(() => {
      useDatabaseCsvStore.setState({
        transfer: {
          transferId: "csv-1",
          profileId: profile.id,
          direction: "export",
          status: "completed",
          exportResult: exportResult(),
          cancelling: false,
          startedAt: 1,
        },
      });
    });

    await waitFor(() => {
      const task = useDatabaseWorkspaceStore.getState().tabs[profile.id]?.tasks[0];
      expect(task?.status).toBe("succeeded");
      expect(task?.detail).toContain("12 rows");
      expect(task?.detail).toContain("2.0 KB");
    });
  });
});
