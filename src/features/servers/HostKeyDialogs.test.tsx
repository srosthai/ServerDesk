import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { HostKeyChangedDialog, HostKeyVerificationDialog } from "@/features/servers/HostKeyDialogs";

describe("host key dialogs", () => {
  it("shows the key type and fingerprint before authentication can proceed", async () => {
    const user = userEvent.setup();
    const onTrust = vi.fn();
    const onReject = vi.fn();

    render(
      <HostKeyVerificationDialog
        serverName="Production"
        host="198.51.100.24:22"
        hostKey={{ keyType: "ssh-ed25519", fingerprint: "SHA256:trustedfingerprint" }}
        pending={false}
        onTrust={onTrust}
        onReject={onReject}
      />,
    );

    expect(screen.getByRole("heading", { name: "Verify SSH host key" })).toBeInTheDocument();
    expect(screen.getByText("ssh-ed25519")).toBeInTheDocument();
    expect(screen.getByText("SHA256:trustedfingerprint")).toBeInTheDocument();
    expect(
      screen.getByText(/Authentication is not attempted until you accept the key/i),
    ).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Trust and connect" }));
    expect(onTrust).toHaveBeenCalledTimes(1);

    await user.click(screen.getByRole("button", { name: "Reject key" }));
    expect(onReject).toHaveBeenCalledTimes(1);
  });

  it("blocks a changed key without offering a trust action", async () => {
    const user = userEvent.setup();
    const onDismiss = vi.fn();
    const onManageKeys = vi.fn();

    render(
      <HostKeyChangedDialog
        serverName="Production"
        host="198.51.100.24:22"
        hostKey={{
          keyType: "ssh-ed25519",
          fingerprint: "SHA256:received",
          expectedFingerprint: "SHA256:expected",
        }}
        onDismiss={onDismiss}
        onManageKeys={onManageKeys}
      />,
    );

    expect(screen.getByRole("heading", { name: "SSH host key changed" })).toBeInTheDocument();
    expect(screen.getByText("SHA256:expected")).toBeInTheDocument();
    expect(screen.getByText("SHA256:received")).toBeInTheDocument();
    expect(
      screen.getByText(/ServerDesk did not change the saved fingerprint/i),
    ).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /Trust and connect/i })).not.toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Manage trusted keys" }));
    expect(onManageKeys).toHaveBeenCalledTimes(1);

    await user.click(screen.getByRole("button", { name: "Dismiss warning" }));
    expect(onDismiss).toHaveBeenCalledTimes(1);
  });
});
