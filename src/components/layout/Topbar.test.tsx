import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import { beforeEach, describe, expect, it } from "vitest";
import { Topbar } from "@/components/layout/Topbar";
import type { UpdateInfo } from "@/lib/updates";
import { useConnectionStore } from "@/stores/connectionStore";
import { usePreferencesStore } from "@/stores/preferencesStore";
import { useServerStore } from "@/stores/serverStore";
import { useUpdateStore } from "@/stores/updateStore";

const updateInfo: UpdateInfo = {
  version: "0.1.1",
  currentVersion: "0.1.0",
  notes: "",
  date: null,
};

function renderTopbar() {
  return render(
    <MemoryRouter initialEntries={["/"]}>
      <Topbar
        sidebarCollapsed={false}
        onOpenNavigation={() => undefined}
        onToggleSidebar={() => undefined}
      />
      <Routes>
        <Route path="/" element={<div>Overview screen</div>} />
        <Route path="/settings" element={<div>Settings screen</div>} />
      </Routes>
    </MemoryRouter>,
  );
}

describe("Topbar update badge", () => {
  beforeEach(() => {
    useServerStore.setState({ servers: [], status: "ready", error: null });
    useConnectionStore.setState({ connections: {}, busy: {}, error: null });
    usePreferencesStore.setState({
      preferences: {
        theme: "system",
        monitoringIntervalSeconds: 30,
        lastActiveServerId: null,
      },
      status: "ready",
      error: null,
    });
    useUpdateStore.setState({
      status: "idle",
      available: null,
      progress: null,
      error: null,
    });
  });

  it("does not show an update action when no update is available", () => {
    renderTopbar();
    expect(screen.queryByRole("button", { name: /available/i })).not.toBeInTheDocument();
  });

  it("shows the available version and opens Settings", async () => {
    const user = userEvent.setup();
    useUpdateStore.setState({ status: "available", available: updateInfo });
    renderTopbar();

    const badge = screen.getByRole("button", {
      name: "Update v0.1.1 available. Open Settings to install it.",
    });
    expect(badge).toHaveTextContent("v0.1.1 available");

    await user.click(badge);

    expect(screen.getByText("Settings screen")).toBeInTheDocument();
  });
});
