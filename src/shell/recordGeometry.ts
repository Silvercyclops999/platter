/**
 * The measurements of a 12-inch long-playing record and a 9-inch pivoted
 * tonearm, in millimetres, and the sums that place the grooves and the arm.
 * The drawing (components/Record.tsx, shell/Turntable.tsx) uses these numbers
 * as its units, so every part is to scale.
 *
 * Record: 302 mm across, a 100 mm label, a 7.26 mm spindle hole. Music is
 * cut from 292.1 mm across (after the lead-in) and no tighter than 120.6 mm;
 * the run-out spirals to the closed locked groove at 106.4 mm.
 *
 * Arm: pivot 212 mm from the spindle, 228.5 mm from pivot to stylus (16.5 mm
 * overhang), the headshell turned 23 degrees towards the spindle.
 */

export const RECORD_R = 151;
export const LABEL_R = 50;
export const SPINDLE_HOLE_R = 3.63;
/** The spindle shaft is a little under the hole (7.15 mm). */
export const SPINDLE_R = 3.575;
/** The outermost groove at recording pitch; outside it is the flat land of the edge. */
export const EDGE_GROOVE_R = 148.8;
/** Where music starts, after a turn or two of lead-in. */
export const MUSIC_START_R = 146.05;
/** The tightest music may be cut. */
export const MUSIC_MIN_R = 60.3;
/** The closed groove at the end of the run-out. */
export const LOCKED_R = 53.2;
/** The platter shows a thin machined rim past the record. */
export const PLATTER_R = 152.5;

/** Pivot to spindle. */
export const ARM_MOUNT = 212;
/** Pivot to stylus. */
export const ARM_LENGTH = 228.5;
/** The headshell's turn towards the spindle, in degrees. */
export const ARM_OFFSET_DEG = 23;

/** The drawing's box, in millimetres: the record on the left, the arm's base at the top right. */
export const VIEW_W = 412;
export const VIEW_H = 346;
/** The spindle. */
export const CENTRE = { x: 160, y: 190 } as const;
/** Where the pivot sits: 28 degrees up from level, to the right of the spindle. */
const PIVOT_BEARING = (28 * Math.PI) / 180;
export const PIVOT = {
  x: CENTRE.x + ARM_MOUNT * Math.cos(PIVOT_BEARING),
  y: CENTRE.y - ARM_MOUNT * Math.sin(PIVOT_BEARING),
} as const;
/** On its rest the arm runs straight down the plinth beside the record. */
export const REST_DEG = 90;

/** A side longer than this is cut to the tightest radius; a shorter one ends further out. */
const FULL_SIDE_MS = 22 * 60_000;
/** Sides are split at about this length when the album doesn't say where. */
export const SIDE_MAX_MS = 24 * 60_000;

/** The innermost music groove of a side this long (a short side leaves a wider run-out). */
export function innerMusicRadius(sideMs: number): number {
  const fill = Math.min(1, Math.max(0.4, sideMs / FULL_SIDE_MS));
  return MUSIC_START_R - (MUSIC_START_R - MUSIC_MIN_R) * fill;
}

/**
 * The groove radius at a point through the side (0 at the first note, 1 at
 * the last). The pitch is even and the record turns at a steady speed, so
 * every turn takes the same time and the radius falls in step with time.
 */
export function grooveRadius(progress: number, sideMs: number): number {
  const p = Math.min(1, Math.max(0, progress));
  return MUSIC_START_R - (MUSIC_START_R - innerMusicRadius(sideMs)) * p;
}

/** How far through the side: the songs before this one, plus the position in it. */
export function sideProgress(durationsMs: readonly number[], index: number, positionMs: number): number {
  const total = durationsMs.reduce((a, b) => a + b, 0);
  if (total <= 0 || index < 0 || index >= durationsMs.length) return 0;
  let before = 0;
  for (let i = 0; i < index; i++) before += durationsMs[i];
  const within = Math.min(Math.max(0, positionMs), durationsMs[index]);
  return (before + within) / total;
}

/** One song's ring of grooves: from its outer edge in to its inner edge. */
export interface TrackBand {
  outer: number;
  inner: number;
}

/** Each song's band of grooves, outside in, at the real place its music sits on the side. */
export function trackBands(durationsMs: readonly number[]): TrackBand[] {
  const total = durationsMs.reduce((a, b) => a + b, 0);
  if (total <= 0) return [];
  const bands: TrackBand[] = [];
  let at = 0;
  for (const d of durationsMs) {
    const outer = grooveRadius(at / total, total);
    at += d;
    bands.push({ outer, inner: grooveRadius(at / total, total) });
  }
  return bands;
}

/**
 * The arm's direction, in degrees (screen axes: 0 is right, 90 is down), for
 * the stylus to sit on the groove at radius r. The stylus is where a circle
 * of the arm's length round the pivot crosses the groove's circle round the
 * spindle.
 */
export function armAngleForRadius(r: number): number {
  const dx = CENTRE.x - PIVOT.x;
  const dy = CENTRE.y - PIVOT.y;
  const d = Math.hypot(dx, dy);
  const cos = (ARM_LENGTH ** 2 + d ** 2 - r ** 2) / (2 * ARM_LENGTH * d);
  const phi = Math.acos(Math.min(1, Math.max(-1, cos)));
  return ((Math.atan2(dy, dx) - phi) * 180) / Math.PI;
}

/** Where the stylus is with the arm at this angle. */
export function stylusAt(deg: number): { x: number; y: number } {
  const a = (deg * Math.PI) / 180;
  return { x: PIVOT.x + ARM_LENGTH * Math.cos(a), y: PIVOT.y + ARM_LENGTH * Math.sin(a) };
}

/**
 * An album's songs split into sides when it doesn't say where its sides
 * are: in order, each side as long as it can be up to SIDE_MAX_MS.
 */
export function splitSides(durationsMs: readonly number[]): number[][] {
  const sides: number[][] = [];
  let cur: number[] = [];
  let len = 0;
  durationsMs.forEach((d, i) => {
    if (cur.length && len + d > SIDE_MAX_MS) {
      sides.push(cur);
      cur = [];
      len = 0;
    }
    cur.push(i);
    len += d;
  });
  if (cur.length) sides.push(cur);
  return sides;
}

/** A small seeded generator, so a record's grooves come out the same each time. */
export function seeded(key: string): () => number {
  let h = 1779033703 ^ key.length;
  for (let i = 0; i < key.length; i++) {
    h = Math.imul(h ^ key.charCodeAt(i), 3432918353);
    h = (h << 13) | (h >>> 19);
  }
  let s = h >>> 0;
  return () => {
    s = (s + 0x6d2b79f5) >>> 0;
    let t = s;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}
