import { describe, expect, it } from "vitest";
import { getCommandErrorMessage, isCommandError } from "@/lib/errors";

describe("command errors", () => {
  it("recognizes structured errors", () => {
    const error = { code: "timeout", message: "Connection timed out" };

    expect(isCommandError(error)).toBe(true);
    expect(getCommandErrorMessage(error)).toBe("Connection timed out");
  });

  it("recognizes string errors", () => {
    expect(getCommandErrorMessage("Network unreachable")).toBe("Network unreachable");
  });

  it("falls back for unknown errors", () => {
    expect(isCommandError(new Error("boom"))).toBe(false);
    expect(getCommandErrorMessage(new Error("boom"))).toBe("The operation failed.");
  });
});
