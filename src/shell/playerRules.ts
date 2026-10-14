/**
 * Rules for the player bar's extras: A-B loop points, the "Next:" line and
 * the volume readout. Kept apart from the components so they're easy to test.
 */
import type { AbLoop, PlayerState, QueueItem } from "../lib/types";

/** The shortest A-B loop. */
export const MIN_LOOP_MS = 1_000;
/** Loop handle keyboard step, and with Shift. */
export const LOOP_KEY_STEP = 1_000;
export const LOOP_KEY_BIG_STEP = 5_000;

/**
 * A loop from two points in either order, clamped to the track. Null when
 * they're under MIN_LOOP_MS apart.
 */
export function normalizeLoop(p: number, q: number, durationMs: number): AbLoop | null {
  const end = Math.max(0, durationMs);
  const a = Math.round(Math.max(0, Math.min(p, q, end)));
  const b = Math.round(Math.max(0, Math.min(Math.max(p, q), end)));
  return b - a >= MIN_LOOP_MS ? { aMs: a, bMs: b } : null;
}

/** "Next:" shows in the last 15 s, or the last 30 % of tracks under 50 s. */
export const UP_NEXT_LAST_MS = 15_000;
export const UP_NEXT_SHORT_TRACK_MS = 50_000;
export const UP_NEXT_SHORT_FRACTION = 0.3;

export function upNextWindowMs(durationMs: number): number {
  return durationMs < UP_NEXT_SHORT_TRACK_MS ? durationMs * UP_NEXT_SHORT_FRACTION : UP_NEXT_LAST_MS;
}

/** Whether the position is in the track's closing stretch. */
export function inUpNextWindow(positionMs: number, durationMs: number): boolean {
  if (durationMs <= 0) return false;
  return durationMs - positionMs <= upNextWindowMs(durationMs);
}

/** What plays after the current track: the user queue first, then the context. */
export function nextItem(state: PlayerState | null): QueueItem | null {
  if (!state) return null;
  return state.queue.userQueue[0] ?? state.queue.nextUp[0] ?? null;
}

/**
 * The item the "Next:" line names right now, or null when it's hidden: the
 * setting is off, nothing is next, repeat-one is on, a loop is set, or the
 * track isn't near its end.
 */
export function upNextShown(state: PlayerState | null, positionMs: number, durationMs: number, enabled: boolean): QueueItem | null {
  if (!enabled || !state?.track) return null;
  if (state.repeat === "one" || state.abLoop) return null;
  if (!inUpNextWindow(positionMs, durationMs)) return null;
  return nextItem(state);
}

/** How long the volume readout stays after the last change. */
export const READOUT_HOLD_MS = 1_000;

export function volumeLabel(volume: number, silent: boolean): string {
  return silent || volume <= 0 ? "Muted" : `${Math.round(volume * 100)}%`;
}
