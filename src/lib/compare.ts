/**
 * Compare's moment mapping, the same rule as the core: the same time when
 * two versions' lengths are within 5%, else the same share of the song.
 * The core does the real switching; the mock and the UI's labels use this.
 */
import type { AbLoop } from "./types";

export const APPROXIMATE_OVER = 0.05;

export function compareApproximate(aMs: number, bMs: number): boolean {
  const long = Math.max(aMs, bMs);
  return long > 0 && Math.abs(aMs - bMs) / long > APPROXIMATE_OVER;
}

export function compareMap(posMs: number, fromMs: number, toMs: number): number {
  if (fromMs <= 0 || toMs <= 0) return 0;
  const at = compareApproximate(fromMs, toMs) ? Math.round((Math.min(posMs, fromMs) * toMs) / fromMs) : posMs;
  return Math.max(0, Math.min(at, toMs - 1));
}

export function compareMapLoop(ab: AbLoop, fromMs: number, toMs: number): AbLoop {
  const aMs = compareMap(ab.aMs, fromMs, toMs);
  return { aMs, bMs: Math.max(compareMap(ab.bMs, fromMs, toMs), aMs + 1) };
}
