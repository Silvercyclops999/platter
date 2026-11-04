/**
 * Moments: songs bookmarked at a time show as ticks on the seek bar, and a
 * seek that lands close to one goes to it exactly.
 */
import { useMemo } from "react";
import { useCurate } from "./curate";

/** The playing song's marked times (ms), in order, without repeats. */
export function useMoments(trackId: string | null): number[] {
  const all = useCurate((s) => s.bookmarks);
  return useMemo(() => {
    if (!trackId) return [];
    const at = all.filter((b) => b.kind === "track" && b.target === trackId && (b.positionMs ?? 0) > 0).map((b) => b.positionMs!);
    return [...new Set(at)].sort((a, b) => a - b);
  }, [all, trackId]);
}

/** How close a seek has to land to a moment to go to it: 2 s or 1.5% of the song. */
export function snapRange(durationMs: number): number {
  return Math.max(2000, durationMs * 0.015);
}

/** The moment `ms` lands on, if one is close enough; otherwise `ms`. */
export function snapToMoment(ms: number, moments: number[], durationMs: number): number {
  let best: number | null = null;
  for (const m of moments) {
    if (Math.abs(m - ms) <= snapRange(durationMs) && (best === null || Math.abs(m - ms) < Math.abs(best - ms))) best = m;
  }
  return best ?? ms;
}
