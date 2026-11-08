import { describe, expect, it } from "vitest";
import { normalizeProjectPath } from "@/features/projects/projectPaths";

describe("normalizeProjectPath", () => {
  it("normalizes absolute paths with duplicate, trailing, and current segments", () => {
    expect(normalizeProjectPath(" /var/www//storefront/ ")).toBe("/var/www/storefront");
    expect(normalizeProjectPath("/var/www/./storefront")).toBe("/var/www/storefront");
    expect(normalizeProjectPath("/")).toBe("/");
    expect(normalizeProjectPath("//")).toBe("/");
    expect(normalizeProjectPath("/var/www/My App")).toBe("/var/www/My App");
  });

  it("rejects relative paths, parent segments, and control characters", () => {
    expect(normalizeProjectPath("")).toBeNull();
    expect(normalizeProjectPath("   ")).toBeNull();
    expect(normalizeProjectPath("var/www/storefront")).toBeNull();
    expect(normalizeProjectPath("../storefront")).toBeNull();
    expect(normalizeProjectPath("/var/www/../etc")).toBeNull();
    expect(normalizeProjectPath("/var/www/\nstorefront")).toBeNull();
    expect(normalizeProjectPath("/var/www/\u0000storefront")).toBeNull();
  });
});
