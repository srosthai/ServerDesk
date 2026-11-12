import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { UpdateSettingsSection } from "@/features/settings/UpdateSettingsSection";
import { isTauriRuntime } from "@/lib/tauri";
import { createUpdateProgress, type UpdateInfo } from "@/lib/updates";
import {
  checkForUpdate,
  downloadAndInstallUpdate,
  getUpdateRuntimeInfo,
  openReleasePage,
} from "@/services/updates";
import { useUpdateStore } from "@/stores/updateStore";

vi.mock("@/lib/tauri", () => ({
  isTauriRuntime: vi.fn(() => false),
}));

vi.mock("@/services/updates", () => ({
  getUpdateRuntimeInfo: vi.fn(),
  checkForUpdate: vi.fn(),
  downloadAndInstallUpdate: vi.fn(),
  relaunchApplication: vi.fn(),
  openReleasePage: vi.fn(),
}));

const updateInfo: UpdateInfo = {
  version: "0.1.1",
  currentVersion: "0.1.0",
  notes: "Fixes a terminal layout issue.\nAdds update checks.",
  date: "2026-11-12T12:00:00Z",
};

function setState(partial: Partial<ReturnType<typeof useUpdateStore.getState>>) {
  useUpdateStore.setState(partial);
}

describe("UpdateSettingsSection", () => {
  beforeEach(() => {
    vi.mocked(isTauriRuntime).mockReturnValue(false);
    vi.mocked(getUpdateRuntimeInfo).mockResolvedValue({
      currentVersion: "0.1.0",
      bundleType: "appimage",
      selfUpdateSupported: true,
      manualPackageUpdate: false,
    });
    vi.mocked(checkForUpdate).mockResolvedValue(null);
    vi.mocked(downloadAndInstallUpdate).mockResolvedValue();
    vi.mocked(openReleasePage).mockResolvedValue();
    useUpdateStore.setState({
      status: "disabled",
      currentVersion: "0.1.0",
      bundleType: null,
      selfUpdateSupported: null,
      available: null,
      progress: null,
      error: null,
      lastCheckedAt: null,
    });
  });

  it("shows the current version and why checks are disabled in a browser preview", () => {
    render(<UpdateSettingsSection />);

    expect(screen.getByText("Updates")).toBeInTheDocument();
    expect(screen.getByText("v0.1.0")).toBeInTheDocument();
    expect(screen.getByText("Not checked yet")).toBeInTheDocument();
    expect(screen.getByText("Update checks are disabled here")).toBeInTheDocument();
    expect(
      screen.getByText("Update checks run in the installed desktop app, not the browser preview."),
    ).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Check for updates" })).toBeDisabled();
  });

  it("explains that development builds skip update checks", () => {
    vi.mocked(isTauriRuntime).mockReturnValue(true);
    render(<UpdateSettingsSection />);

    expect(screen.getByText(/development builds/)).toBeInTheDocument();
  });

  it("shows an available version, release notes, and manual check action", () => {
    setState({
      status: "available",
      bundleType: "appimage",
      selfUpdateSupported: true,
      available: updateInfo,
      lastCheckedAt: Date.now() - 60_000,
    });
    render(<UpdateSettingsSection />);

    expect(screen.getByText("ServerDesk v0.1.1 is available")).toBeInTheDocument();
    expect(screen.getByText("Installed v0.1.0 · Released 2026-11-12")).toBeInTheDocument();
    expect(screen.getByText(/Fixes a terminal layout issue/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Download and install/ })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Check for updates" })).toBeInTheDocument();
  });

  it("shows download progress and then install progress", () => {
    setState({
      status: "downloading",
      bundleType: "appimage",
      selfUpdateSupported: true,
      available: updateInfo,
      progress: createUpdateProgress(512 * 1024, 2 * 1024 * 1024),
    });
    const { rerender } = render(<UpdateSettingsSection />);

    expect(screen.getByText("Downloading update… 25%")).toBeInTheDocument();
    expect(screen.getByText("512 KB of 2 MB")).toBeInTheDocument();
    expect(screen.getByRole("progressbar", { name: "Update download progress" })).toHaveAttribute(
      "aria-valuenow",
      "25",
    );

    setState({
      status: "installing",
      progress: createUpdateProgress(2 * 1024 * 1024, 2 * 1024 * 1024),
    });
    rerender(<UpdateSettingsSection />);

    expect(screen.getByText("Installing update…")).toBeInTheDocument();
    expect(screen.getByRole("progressbar", { name: "Update download progress" })).toHaveAttribute(
      "aria-valuenow",
      "100",
    );
    expect(screen.queryByRole("button", { name: /Download and install/ })).not.toBeInTheDocument();
  });

  it("shows the up-to-date state", () => {
    setState({ status: "up-to-date" });
    render(<UpdateSettingsSection />);

    expect(screen.getByText("ServerDesk is up to date")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /Download and install/ })).not.toBeInTheDocument();
  });

  it("shows clear signature, network, and permission errors without claiming success", () => {
    setState({
      status: "error",
      error: {
        kind: "signature",
        message: "minisign signature verification failed",
        details: null,
      },
    });
    const { rerender } = render(<UpdateSettingsSection />);

    expect(screen.getByText("The update could not be verified")).toBeInTheDocument();
    expect(screen.getByText(/Nothing was installed/)).toBeInTheDocument();
    expect(screen.queryByText("ServerDesk is up to date")).not.toBeInTheDocument();

    setState({
      status: "error",
      error: { kind: "network", message: "failed to fetch", details: null },
    });
    rerender(<UpdateSettingsSection />);
    expect(screen.getByText("The update server could not be reached")).toBeInTheDocument();

    setState({
      status: "error",
      error: { kind: "permission", message: "capability denied", details: null },
    });
    rerender(<UpdateSettingsSection />);
    expect(screen.getByText("The update is not permitted")).toBeInTheDocument();
  });

  it("offers a download page instead of in-place install for DEB and RPM packages", async () => {
    const user = userEvent.setup();
    setState({
      status: "available",
      bundleType: "deb",
      selfUpdateSupported: false,
      available: updateInfo,
    });
    render(<UpdateSettingsSection />);

    expect(screen.getByText("Manual update required")).toBeInTheDocument();
    expect(screen.getByText(/managed by the system package manager/)).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /Download and install/ })).not.toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: /Open download page/ }));
    await waitFor(() => expect(openReleasePage).toHaveBeenCalledTimes(1));
  });

  it("does not claim success while installing", () => {
    setState({
      status: "downloading",
      bundleType: "appimage",
      selfUpdateSupported: true,
      available: updateInfo,
      progress: createUpdateProgress(10, 100),
    });
    render(<UpdateSettingsSection />);

    expect(screen.queryByText("Update installed")).not.toBeInTheDocument();
    expect(screen.queryByText("ServerDesk is up to date")).not.toBeInTheDocument();
  });

  it("shows a restart warning when installation completed but relaunch failed", () => {
    setState({
      status: "installed",
      error: {
        kind: "relaunch",
        message: "Could not restart",
        details: null,
      },
    });
    render(<UpdateSettingsSection />);

    expect(screen.getByText("Restart required")).toBeInTheDocument();
    expect(screen.getByText(/Quit and open ServerDesk again/)).toBeInTheDocument();
  });
});
