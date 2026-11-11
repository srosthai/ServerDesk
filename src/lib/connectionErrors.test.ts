import { describe, expect, it } from "vitest";
import { describeConnectionError } from "@/lib/connectionErrors";
import type { ErrorCode } from "@/types/contracts";

describe("describeConnectionError", () => {
  it("uses distinct titles for the supported connection failure codes", () => {
    const cases: Array<[ErrorCode, string]> = [
      ["authentication", "Authentication failed"],
      ["host_key", "Host key mismatch"],
      ["timeout", "Connection timed out"],
      ["dns", "Hostname could not be resolved"],
      ["network", "Network connection lost"],
      ["permission", "Permission denied"],
      ["remote", "Remote check failed"],
      ["cancelled", "Connection cancelled"],
      ["credential", "Credential unavailable"],
      ["credential_unavailable", "Credential store unavailable"],
      ["credential_locked", "Credential store locked"],
      ["credential_missing", "Saved credential missing"],
      ["credential_invalid", "Saved credential invalid"],
    ];

    for (const [code, title] of cases) {
      expect(describeConnectionError({ code }).title).toBe(title);
    }

    const titles = cases.map(([code]) => describeConnectionError({ code }).title);
    expect(new Set(titles).size).toBe(cases.length);
  });

  it("falls back to a safe generic presentation for unknown codes", () => {
    const presentation = describeConnectionError({ code: "unexpected" as ErrorCode });

    expect(presentation.title).toBe("Connection failed");
    expect(presentation.action.length).toBeGreaterThan(0);
  });
});
