import { act, renderHook } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { SLOW_LOAD_MS, useReload, useTakingLong } from "../lib/useLoad";

/** A fetch whose answers the test hands out in any order. */
function held() {
  const calls: ((v: string) => void)[] = [];
  return { fetch: () => new Promise<string>((r) => calls.push(r)), calls };
}

describe("background page reloads", () => {
  it("the newest reload wins, even when an older one answers last", async () => {
    const f = held();
    const { result } = renderHook(({ loaded }) => useReload(loaded, f.fetch), { initialProps: { loaded: "first load" } });
    act(() => result.current.reload());
    act(() => result.current.reload());
    await act(async () => f.calls[1]("newer"));
    await act(async () => f.calls[0]("older"));
    expect(result.current.data).toBe("newer");
  });

  it("new data from the page's own load replaces a reload and drops ones still on their way", async () => {
    const f = held();
    const { result, rerender } = renderHook(({ loaded }) => useReload(loaded, f.fetch), { initialProps: { loaded: "first load" } });
    act(() => result.current.reload());
    await act(async () => f.calls[0]("reloaded"));
    expect(result.current.data).toBe("reloaded");
    act(() => result.current.reload());
    rerender({ loaded: "library changed" });
    expect(result.current.data).toBe("library changed");
    await act(async () => f.calls[1]("stale"));
    expect(result.current.data).toBe("library changed");
  });
});

describe("a skeleton that's up too long", () => {
  it("gets a note once loading passes the limit, and loses it when the data comes", () => {
    vi.useFakeTimers();
    try {
      const { result, rerender } = renderHook(({ waiting }) => useTakingLong(waiting), { initialProps: { waiting: true } });
      expect(result.current).toBe(false);
      act(() => vi.advanceTimersByTime(SLOW_LOAD_MS - 1));
      expect(result.current).toBe(false);
      act(() => vi.advanceTimersByTime(1));
      expect(result.current).toBe(true);
      rerender({ waiting: false });
      expect(result.current).toBe(false);
    } finally {
      vi.useRealTimers();
    }
  });
});
