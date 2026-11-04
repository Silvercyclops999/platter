/**
 * Moments: a song bookmarked at a time shows a tick on the seek bar there,
 * a seek let go near a tick starts right at it, and one tap on Now playing
 * marks the moment playing.
 */
import { act, render, renderHook } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { Slider } from "../components/Slider";
import { backend } from "../lib/backend";
import type { Bookmark } from "../lib/types";
import { useCurate } from "../stores/curate";
import { snapRange, snapToMoment, useMoments } from "../stores/moments";

const bm = (id: string, target: string, positionMs: number | null, kind: Bookmark["kind"] = "track"): Bookmark => ({
  id,
  kind,
  target,
  title: "t",
  positionMs,
  createdAt: 1,
});

afterEach(() => {
  vi.restoreAllMocks();
  act(() => useCurate.setState({ bookmarks: [] }));
});

describe("moments", () => {
  it("are the playing song's marked times, in order, once each", () => {
    act(() =>
      useCurate.setState({
        bookmarks: [bm("1", "s", 90_000), bm("2", "s", 30_000), bm("3", "s", 90_000), bm("4", "other", 10_000), bm("5", "s", 0), bm("6", "s", null, "page")],
      }),
    );
    const { result } = renderHook(() => useMoments("s"));
    expect(result.current).toEqual([30_000, 90_000]);
    expect(renderHook(() => useMoments(null)).result.current).toEqual([]);
  });

  it("a seek near a moment goes to it, the nearest when two are close", () => {
    const d = 200_000;
    expect(snapRange(d)).toBe(3000);
    expect(snapToMoment(31_500, [30_000, 90_000], d)).toBe(30_000);
    expect(snapToMoment(34_000, [30_000, 90_000], d)).toBe(34_000);
    expect(snapToMoment(31_000, [30_000, 32_500], d)).toBe(30_000);
    expect(snapRange(60_000)).toBe(2000);
    expect(snapToMoment(5000, [], d)).toBe(5000);
  });

  it("show as ticks on the seek bar", () => {
    const { container } = render(<Slider label="Seek" value={0} max={200_000} marks={[50_000, 150_000]} />);
    const ticks = [...container.querySelectorAll<HTMLElement>(".slider__mark")];
    expect(ticks.map((t) => t.style.left)).toEqual(["25%", "75%"]);
    expect(ticks.every((t) => t.getAttribute("aria-hidden") === "true")).toBe(true);
  });

  it("marking says where, and says so when it fails", async () => {
    const { bookmarkTrack } = await import("../stores/curate");
    const { useToasts } = await import("../stores/toasts");
    const { usePlayer } = await import("../stores/player");
    const track = { id: "s", title: "Song", artistTitle: "A", thumb: null } as never;
    act(() => usePlayer.setState({ state: { ...(usePlayer.getState().state ?? {}), track } as never, positionMs: 92_400 }));
    const add = vi.spyOn(backend, "bookmarkAdd").mockResolvedValue(undefined as never);
    await bookmarkTrack(track);
    expect(add).toHaveBeenCalledWith(expect.objectContaining({ kind: "track", target: "s", positionMs: 92_400 }));
    const toasts = () => useToasts.getState().toasts;
    expect(toasts()[toasts().length - 1].message).toBe("Bookmarked at 1:32");
    add.mockRejectedValue(new Error("disk full"));
    await bookmarkTrack(track);
    expect(toasts()[toasts().length - 1].kind).toBe("error");
  });
});
