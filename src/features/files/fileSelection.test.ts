import { describe, expect, it } from "vitest";
import {
  addFileSelectionRange,
  retainFileSelection,
  toggleFileSelection,
} from "@/features/files/fileSelection";

const paths = ["/var/www/a.txt", "/var/www/b.txt", "/var/www/c.txt", "/var/www/d.txt"];

describe("file selection helpers", () => {
  it("toggles one path without changing the previous set", () => {
    const initial = new Set(["/var/www/a.txt"]);
    const added = toggleFileSelection(initial, "/var/www/b.txt");
    const removed = toggleFileSelection(added, "/var/www/a.txt");

    expect([...initial]).toEqual(["/var/www/a.txt"]);
    expect([...added]).toEqual(["/var/www/a.txt", "/var/www/b.txt"]);
    expect([...removed]).toEqual(["/var/www/b.txt"]);
  });

  it("adds every path between the anchor and target", () => {
    const selected = new Set(["/var/www/d.txt"]);
    const next = addFileSelectionRange(selected, paths, "/var/www/b.txt", "/var/www/d.txt");

    expect([...next]).toEqual(["/var/www/d.txt", "/var/www/b.txt", "/var/www/c.txt"]);
    expect(next.has("/var/www/a.txt")).toBe(false);
  });

  it("supports ranges in either direction and ignores unknown anchors", () => {
    const forward = addFileSelectionRange(new Set(), paths, "/var/www/a.txt", "/var/www/c.txt");
    expect([...forward]).toEqual(paths.slice(0, 3));

    const backwards = addFileSelectionRange(new Set(), paths, "/var/www/c.txt", "/var/www/a.txt");
    expect([...backwards]).toEqual(paths.slice(0, 3));

    const unknown = addFileSelectionRange(new Set(), paths, "/missing", "/var/www/b.txt");
    expect([...unknown]).toEqual([]);
  });

  it("drops selections that no longer exist after a refresh", () => {
    const selected = new Set(["/var/www/a.txt", "/var/www/removed.txt"]);
    const retained = retainFileSelection(selected, ["/var/www/a.txt", "/var/www/b.txt"]);

    expect([...retained]).toEqual(["/var/www/a.txt"]);
    expect(selected.has("/var/www/removed.txt")).toBe(true);
  });
});
