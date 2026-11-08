import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import App from "@/App";

describe("application shell", () => {
  it("renders the overview and navigates between routes", async () => {
    const user = userEvent.setup();
    render(<App />);

    expect(screen.getByRole("heading", { name: "Overview" })).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Skip to content" })).toBeInTheDocument();

    await user.click(screen.getByRole("link", { name: "Servers" }));

    expect(await screen.findByRole("heading", { name: "Servers" })).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Servers" })).toHaveAttribute("aria-current", "page");
    expect(screen.getByText("No server profiles yet")).toBeInTheDocument();
  });

  it("opens settings from navigation", async () => {
    const user = userEvent.setup();
    render(<App />);

    await user.click(screen.getByRole("link", { name: "Settings" }));

    expect(await screen.findByRole("heading", { name: "Settings" })).toBeInTheDocument();
    expect(screen.getAllByText("Web preview").length).toBeGreaterThan(0);
  });
});
