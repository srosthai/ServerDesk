import { describe, expect, it } from "vitest";
import { filteredLogText, filterLogLines, formatByteSize } from "@/features/logs/logView";

describe("log view helpers", () => {
  it("filters the retrieved tail case-insensitively with line numbers", () => {
    const filtered = filterLogLines(
      "2026-11-09 INFO app started\n2026-11-09 ERROR queue failed\n2026-11-09 INFO retry\n",
      " error ",
    );

    expect(filtered.totalLines).toBe(3);
    expect(filtered.matchedLines).toBe(1);
    expect(filtered.lines).toEqual([{ number: 2, text: "2026-11-09 ERROR queue failed" }]);
    expect(filteredLogText(filtered)).toBe("2026-11-09 ERROR queue failed");
  });

  it("returns every line when the filter is empty", () => {
    const filtered = filterLogLines("first\nsecond\n", "");

    expect(filtered.totalLines).toBe(2);
    expect(filtered.matchedLines).toBe(2);
    expect(filteredLogText(filtered)).toBe("first\nsecond");
  });

  it("handles empty content and no matches", () => {
    expect(filterLogLines("", "anything")).toMatchObject({
      totalLines: 0,
      matchedLines: 0,
      lines: [],
    });
    expect(filterLogLines("first\nsecond\n", "missing")).toMatchObject({
      totalLines: 2,
      matchedLines: 0,
      lines: [],
    });
  });

  it("formats byte sizes for the retrieval metadata", () => {
    expect(formatByteSize(null)).toBe("unknown");
    expect(formatByteSize(0)).toBe("0 B");
    expect(formatByteSize(1023)).toBe("1023 B");
    expect(formatByteSize(1024)).toBe("1.0 KiB");
    expect(formatByteSize(64 * 1024)).toBe("64 KiB");
    expect(formatByteSize(5 * 1024 * 1024)).toBe("5.0 MiB");
  });
});
