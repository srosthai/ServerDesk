import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it } from "vitest";
import { ServersPage } from "@/features/servers/ServersPage";
import { useServerStore } from "@/stores/serverStore";

describe("ServersPage", () => {
  beforeEach(() => {
    useServerStore.setState({
      servers: [],
      status: "idle",
      error: null,
    });
  });

  it("creates, searches, and deletes a server profile", async () => {
    const user = userEvent.setup();
    render(<ServersPage />);

    expect(await screen.findByText("No server profiles yet")).toBeInTheDocument();
    await user.click(screen.getAllByRole("button", { name: "Add server" })[0]);

    await user.type(screen.getByLabelText("Name"), "Production");
    await user.type(screen.getByLabelText("Host"), "198.51.100.24");
    await user.type(screen.getByLabelText("Username"), "deploy");
    await user.click(screen.getByRole("button", { name: "Save server" }));

    expect(await screen.findByText("Production")).toBeInTheDocument();
    expect(screen.getByText("198.51.100.24:22")).toBeInTheDocument();

    await user.type(screen.getByLabelText("Search servers"), "staging");
    expect(await screen.findByText("No servers match")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Clear filters" }));
    expect(await screen.findByText("Production")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Delete Production" }));
    expect(screen.getByText("Delete Production?")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Delete server" }));

    await waitFor(() => {
      expect(screen.queryByText("Production")).not.toBeInTheDocument();
    });
  });
});
