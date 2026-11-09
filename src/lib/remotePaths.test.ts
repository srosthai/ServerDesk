import { describe, expect, it } from "vitest";
import {
  formatFileSize,
  isRemotePathWithin,
  normalizeRemotePath,
  remoteBreadcrumbs,
  remoteParentPath,
} from "@/lib/remotePaths";

describe("remote path helpers", () => {
  it("normalizes absolute paths and rejects unsafe forms", () => {
    expect(normalizeRemotePath(" /var//www/./app/ ")).toBe("/var/www/app");
    expect(normalizeRemotePath("/")).toBe("/");
    expect(normalizeRemotePath("//")).toBe("/");

    for (const invalid of [
      "",
      "   ",
      "var/www",
      "../etc",
      "/var/www/../etc",
      "/var/www/\napp",
      `/${"a".repeat(1025)}`,
    ]) {
      expect(normalizeRemotePath(invalid), invalid).toBeNull();
    }
  });

  it("keeps browsing inside the selected root boundary", () => {
    expect(isRemotePathWithin("/", "/etc")).toBe(true);
    expect(isRemotePathWithin("/var/www", "/var/www")).toBe(true);
    expect(isRemotePathWithin("/var/www", "/var/www/app")).toBe(true);
    expect(isRemotePathWithin("/var/www", "/var/wwwroot")).toBe(false);
    expect(isRemotePathWithin("/var/www", "/var")).toBe(false);
  });

  it("computes parents and breadcrumbs without escaping the root", () => {
    expect(remoteParentPath("/var/www", "/var/www/app/public")).toBe("/var/www/app");
    expect(remoteParentPath("/var/www", "/var/www/app")).toBe("/var/www");
    expect(remoteParentPath("/var/www", "/var/www")).toBeNull();
    expect(remoteParentPath("/", "/var")).toBe("/");
    expect(remoteParentPath("/var/www", "/etc")).toBeNull();

    expect(remoteBreadcrumbs("/var/www", "/var/www/app/public")).toEqual([
      { name: "www", path: "/var/www" },
      { name: "app", path: "/var/www/app" },
      { name: "public", path: "/var/www/app/public" },
    ]);
    expect(remoteBreadcrumbs("/", "/var/www")).toEqual([
      { name: "/", path: "/" },
      { name: "var", path: "/var" },
      { name: "www", path: "/var/www" },
    ]);
  });

  it("formats byte sizes for the metadata table", () => {
    expect(formatFileSize(null)).toBe("Unavailable");
    expect(formatFileSize(12)).toBe("12 B");
    expect(formatFileSize(2048)).toBe("2.0 KiB");
    expect(formatFileSize(10 * 1024 * 1024)).toBe("10 MiB");
  });
});
