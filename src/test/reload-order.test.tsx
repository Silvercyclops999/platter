import { act, renderHook } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { useReload } from "../lib/useLoad";

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
