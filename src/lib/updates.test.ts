import { beforeEach, describe, expect, it } from "vitest";
import {
  classifyUpdateError,
  clearLastUpdateCheck,
  compareVersions,
  createUpdateProgress,
  decideUpdateCheck,
  formatBytes,
  formatLastChecked,
  formatVersionLabel,
  getUpdateErrorCopy,
  isNewerVersion,
  readLastUpdateCheck,
  UPDATE_CHECK_INTERVAL_MS,
  UPDATE_LAST_CHECK_KEY,
  writeLastUpdateCheck,
} from "@/lib/updates";

const environment = {
  runtime: true,
  development: false,
  online: true,
  lastCheckedAt: null,
  now: 1_000_000,
  trigger: "startup" as const,
};

describe("update versions", () => {
  it("compares semantic versions", () => {
    expect(compareVersions("0.1.1", "0.1.0")).toBeGreaterThan(0);
    expect(compareVersions("1.0.0", "0.99.99")).toBeGreaterThan(0);
    expect(compareVersions("0.1.0", "0.1.0")).toBe(0);
    expect(compareVersions("0.1.0", "0.1.1")).toBeLessThan(0);
    expect(compareVersions("1.0.0-beta.2", "1.0.0-beta.1")).toBeGreaterThan(0);
    expect(compareVersions("1.0.0", "1.0.0-rc.1")).toBeGreaterThan(0);
  });

  it("treats malformed versions defensively", () => {
    expect(compareVersions("not-a-version", "0.1.0")).toBe(0);
    expect(isNewerVersion("0.1.1", "0.1.0")).toBe(true);
    expect(isNewerVersion("0.1.0", "0.1.0")).toBe(false);
    expect(isNewerVersion("custom-build", "0.1.0")).toBe(true);
  });

  it("formats version labels", () => {
    expect(formatVersionLabel("0.1.1")).toBe("v0.1.1");
    expect(formatVersionLabel("v0.1.1")).toBe("v0.1.1");
    expect(formatVersionLabel("")).toBe("Unknown version");
  });
});

describe("update check policy", () => {
  it("never checks in web previews or development builds", () => {
    expect(decideUpdateCheck({ ...environment, runtime: false })).toBe("disabled");
    expect(decideUpdateCheck({ ...environment, development: true })).toBe("disabled");
  });

  it("skips startup checks while offline", () => {
    expect(decideUpdateCheck({ ...environment, online: false })).toBe("offline");
  });

  it("throttles startup checks but always allows manual checks", () => {
    const recent = {
      ...environment,
      lastCheckedAt: environment.now - UPDATE_CHECK_INTERVAL_MS + 1,
    };

    expect(decideUpdateCheck(recent)).toBe("recent");
    expect(decideUpdateCheck({ ...recent, trigger: "manual" })).toBe("allowed");
    expect(
      decideUpdateCheck({
        ...environment,
        lastCheckedAt: environment.now - UPDATE_CHECK_INTERVAL_MS,
      }),
    ).toBe("allowed");
  });
});

describe("update progress and labels", () => {
  it("computes bounded progress", () => {
    expect(createUpdateProgress(50, 200)).toEqual({
      downloadedBytes: 50,
      totalBytes: 200,
      percent: 25,
    });
    expect(createUpdateProgress(500, 200).percent).toBe(100);
    expect(createUpdateProgress(-5, null)).toEqual({
      downloadedBytes: 0,
      totalBytes: null,
      percent: null,
    });
  });

  it("formats byte counts", () => {
    expect(formatBytes(0)).toBe("0 B");
    expect(formatBytes(512)).toBe("512 B");
    expect(formatBytes(1536)).toBe("1.5 KB");
    expect(formatBytes(20 * 1024 * 1024)).toBe("20 MB");
  });

  it("formats the last check time without locale-dependent output", () => {
    const now = Date.parse("2026-11-12T12:00:00Z");

    expect(formatLastChecked(null, now)).toBe("Not checked yet");
    expect(formatLastChecked(now - 30_000, now)).toBe("Checked just now");
    expect(formatLastChecked(now - 5 * 60_000, now)).toBe("Checked 5 min ago");
    expect(formatLastChecked(now - 3 * 60 * 60_000, now)).toBe("Checked 3 hr ago");
    expect(formatLastChecked(now - 3 * 24 * 60 * 60_000, now)).toBe("Checked on 2026-11-09");
  });
});

describe("update errors", () => {
  it.each([
    ["Network error: failed to fetch", "network"],
    ["The signature is invalid", "signature"],
    ["minisign signature verification failed", "signature"],
    ["Permission denied by capability", "permission"],
    ["Unsupported platform: AppImage is required", "unsupported"],
    ["The device is offline", "offline"],
    ["Could not install the update", "install"],
    ["Something unexpected happened", "unknown"],
  ])("classifies %s as %s", (message, kind) => {
    expect(classifyUpdateError(new Error(message)).kind).toBe(kind);
  });

  it("reads plugin string errors and command-style objects", () => {
    expect(classifyUpdateError("signature verification failed").message).toBe(
      "signature verification failed",
    );
    expect(classifyUpdateError({ message: "403 forbidden" }).kind).toBe("permission");
    expect(classifyUpdateError({ error: "network unreachable" }).kind).toBe("network");
  });

  it("provides actionable copy for every failure kind", () => {
    for (const kind of [
      "offline",
      "network",
      "signature",
      "permission",
      "unsupported",
      "install",
      "relaunch",
      "unknown",
    ] as const) {
      const copy = getUpdateErrorCopy({
        kind,
        message: "details",
        details: null,
      });

      expect(copy.title.length).toBeGreaterThan(0);
      expect(copy.description.length).toBeGreaterThan(0);
    }

    expect(
      getUpdateErrorCopy({ kind: "signature", message: "bad", details: null }).description,
    ).toContain("Nothing was installed");
  });
});

describe("last update check persistence", () => {
  beforeEach(() => {
    clearLastUpdateCheck();
  });

  it("round-trips a timestamp", () => {
    writeLastUpdateCheck(1_700_000_000_000);
    expect(readLastUpdateCheck()).toBe(1_700_000_000_000);
    expect(localStorage.getItem(UPDATE_LAST_CHECK_KEY)).toBe("1700000000000");
  });

  it("ignores malformed stored values", () => {
    localStorage.setItem(UPDATE_LAST_CHECK_KEY, "not-a-number");
    expect(readLastUpdateCheck()).toBeNull();
    localStorage.setItem(UPDATE_LAST_CHECK_KEY, "-1");
    expect(readLastUpdateCheck()).toBeNull();
  });
});
