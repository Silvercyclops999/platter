/**
 * Stats page chart helpers: the average, what the minutes chart says to a
 * screen reader, and each top row's share of the #1. Pure, so tests check
 * them. The sentences and shapes are in statsStory.
 */
import { formatMinutes, minutes } from "./stats";
import type { StatsPoint } from "./statsTypes";

/** The average of the points with listening, or 0. */
export function averageMs(points: StatsPoint[]): number {
  const on = points.filter((p) => p.ms > 0);
  return on.length ? on.reduce((s, p) => s + p.ms, 0) / on.length : 0;
}

/** What the bar chart says to a screen reader. */
export function seriesSummary(points: StatsPoint[], bucket: "day" | "month", label: (p: StatsPoint) => string): string {
  const on = points.filter((p) => p.ms > 0);
  if (!on.length) return `Minutes listened per ${bucket}: nothing in this range.`;
  const best = on.reduce((a, b) => (b.ms > a.ms ? b : a));
  return `Minutes listened per ${bucket}. ${on.length} of ${points.length} ${bucket}s with listening, averaging ${minutes(averageMs(points)).toLocaleString()} minutes. Most: ${label(best)}, ${formatMinutes(best.ms)}.`;
}

/** Each row's bar: its share of the first (top) row, 0 to 1. */
export function shareOfTop(values: number[]): number[] {
  const top = values[0] ?? 0;
  return values.map((v) => (top > 0 ? Math.min(1, v / top) : 0));
}
