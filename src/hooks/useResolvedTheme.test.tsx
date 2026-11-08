import { act, renderHook, waitFor } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { useResolvedTheme } from "@/hooks/useResolvedTheme";

describe("useResolvedTheme", () => {
  it("follows the document theme class used by ThemeSync", async () => {
    const { result } = renderHook(() => useResolvedTheme());
    expect(result.current).toBe("light");

    act(() => {
      document.documentElement.classList.add("dark");
    });

    await waitFor(() => expect(result.current).toBe("dark"));

    act(() => {
      document.documentElement.classList.remove("dark");
    });

    await waitFor(() => expect(result.current).toBe("light"));
  });
});
