/**
 * Rules for the lyrics editor's sync step, kept apart from the component so
 * they can be tested on their own: stamping a line, stepping back, nudging,
 * dragging a mark on the timeline, and keeping times in order.
 */
import { stampable, type EditLine } from "../lib/lrc";

/** Alt+Left/Right moves the selected line by this much. */
export const NUDGE_MS = 100;
/** Backspace rewinds the song by this much. */
export const STEP_BACK_REWIND_MS = 3_000;

/** The next line with words after `i` (-1: none). */
export function nextStampable(lines: EditLine[], i: number): number {
  for (let j = i + 1; j < lines.length; j++) if (stampable(lines[j])) return j;
  return -1;
}

/** The line with words before `i` (-1: none). */
export function prevStampable(lines: EditLine[], i: number): number {
  for (let j = Math.min(i, lines.length) - 1; j >= 0; j--) if (stampable(lines[j])) return j;
  return -1;
}

/** The first line with words (-1: none). */
export const firstStampable = (lines: EditLine[]) => nextStampable(lines, -1);

/** The nearest set times before and after line `i`. */
function neighbours(lines: EditLine[], i: number): { lo: number; hi: number } {
  let lo = 0;
  let hi = Infinity;
  for (let j = i - 1; j >= 0; j--) {
    const t = lines[j].startMs;
    if (t !== null) {
      lo = t;
      break;
    }
  }
  for (let j = i + 1; j < lines.length; j++) {
    const t = lines[j].startMs;
    if (t !== null) {
      hi = t;
      break;
    }
  }
  return { lo, hi };
}

const set = (lines: EditLine[], i: number, startMs: number | null): EditLine[] =>
  lines.map((l, j) => (j === i ? { ...l, startMs } : l));

/**
 * Space: line `i` starts now. Later lines already timed before `ms` lose
 * their time (they'd be out of order); you'll stamp them again as you go.
 */
export function stamp(lines: EditLine[], i: number, ms: number): EditLine[] {
  if (!lines[i] || !stampable(lines[i])) return lines;
  const t = Math.max(0, Math.round(ms));
  return lines.map((l, j) => {
    if (j === i) return { ...l, startMs: t };
    if (j > i && l.startMs !== null && l.startMs < t) return { ...l, startMs: null };
    return l;
  });
}

/** A line's time moved to `ms`, kept between its neighbours' times and inside the song. */
export function moveTo(lines: EditLine[], i: number, ms: number, durationMs = Infinity): EditLine[] {
  if (!lines[i] || lines[i].startMs === null) return lines;
  const { lo, hi } = neighbours(lines, i);
  const max = Math.min(hi, durationMs > 0 ? durationMs : Infinity);
  const t = Math.round(Math.min(Math.max(ms, lo), max));
  return t === lines[i].startMs ? lines : set(lines, i, t);
}

/** Alt+Left/Right: the line's time by ±0.1 s, kept in order. */
export function nudge(lines: EditLine[], i: number, deltaMs: number, durationMs = Infinity): EditLine[] {
  const t = lines[i]?.startMs;
  return t === null || t === undefined ? lines : moveTo(lines, i, t + deltaMs, durationMs);
}

/** Every time cleared (back to plain lyrics). */
export const clearTimes = (lines: EditLine[]): EditLine[] => lines.map((l) => (l.startMs === null ? l : { ...l, startMs: null }));

/**
 * The Write step's text changed: new lines, keeping the time of each line
 * with words whose text is unchanged at the same place among the lines
 * with words.
 */
export function reconcile(old: EditLine[], next: EditLine[]): EditLine[] {
  const oldWords = old.filter(stampable);
  let k = 0;
  return next.map((l) => {
    if (!stampable(l)) return l;
    const o = oldWords[k++];
    return o && o.text === l.text && o.startMs !== null ? { ...l, startMs: o.startMs } : l;
  });
}

/** How much of the song the timeline shows at once: fine enough to drag a mark by a tenth. */
export const TIMELINE_SPAN_MS = 20_000;

export interface TimelineWindow {
  start: number;
  span: number;
}

/** The stretch of song the timeline shows: `centre` a third of the way in, kept inside the song. */
export function timelineWindow(centre: number, durationMs: number): TimelineWindow {
  const span = durationMs > 0 ? Math.min(durationMs, TIMELINE_SPAN_MS) : TIMELINE_SPAN_MS;
  const max = Math.max(0, durationMs - span);
  const start = Math.round(Math.min(Math.max(centre - span / 3, 0), max));
  return { start, span };
}

/** Where a time sits on the timeline, 0 to 1. */
export const timelineFraction = (ms: number, durationMs: number) => (durationMs > 0 ? Math.min(1, Math.max(0, ms / durationMs)) : 0);
