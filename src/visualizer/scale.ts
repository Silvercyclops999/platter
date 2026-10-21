/**
 * The visualizer's scales and how things move. Plain maths over the core's
 * frames (docs/IPC.md "Visualizer"), so the styles stay thin and the numbers
 * are tested on their own.
 */

export const BANDS = 64;
export const LOW_HZ = 20;
export const HIGH_HZ = 20_000;
/** What silence reads, in dBFS. */
export const FLOOR_DB = -90;
/** Frames further apart than this are a gap: everything falls to rest instead of freezing. */
export const GAP_MS = 500;
/** How often a watch is renewed (the core ends one 3 s after the last renewal). */
export const RENEW_MS = 1_000;

/** Lower edge of band `k` in Hz (k = BANDS is the top edge). */
export const bandEdge = (k: number) => LOW_HZ * Math.pow(HIGH_HZ / LOW_HZ, k / BANDS);
/** Band `k`'s centre in Hz (geometric). */
export const bandCentre = (k: number) => LOW_HZ * Math.pow(HIGH_HZ / LOW_HZ, (k + 0.5) / BANDS);

/** One step of a first-order follower towards `target`. */
export function follow(current: number, target: number, dtMs: number, tauMs: number): number {
  if (tauMs <= 0) return target;
  const k = 1 - Math.exp(-dtMs / tauMs);
  return current + (target - current) * k;
}

/** The frame to draw: the latest one, or none once frames stop coming. */
export function current<T>(frame: T | null, arrivedAt: number, now: number): T | null {
  return frame && now - arrivedAt <= GAP_MS ? frame : null;
}

/** Bars: what they cover, and how they move. */
export const BAR_LOW_HZ = 40;
export const BAR_HIGH_HZ = 16_000;
/** A bar rises this quickly... */
export const BAR_ATTACK_MS = 30;
/** ...and settles this slowly, so a hit reads as a hit and the row never flickers. */
export const BAR_RELEASE_MS = 260;
/** The dB a bar reads as empty and as full. */
export const BAR_FLOOR_DB = -66;
export const BAR_TOP_DB = -8;
/**
 * Music gets quieter toward the top: each bar is lifted this many dB per
 * octave above 1 kHz (and lowered below it), so the row moves evenly
 * instead of sloping down from the bass.
 */
export const BAR_TILT_DB_OCT = 3;
/** The tallest a bar stands, as a share of its room. */
export const BAR_MAX = 0.82;

/**
 * Which bands each of `count` bars covers, log-spaced from BAR_LOW_HZ to
 * BAR_HIGH_HZ. Every bar gets at least one band, and the bands never go
 * backwards, so the low bars (fewer bands per octave) still move on their own.
 */
export function barBands(count: number): [number, number][] {
  const out: [number, number][] = [];
  const toBand = (hz: number) => Math.round((Math.log(hz / LOW_HZ) / Math.log(HIGH_HZ / LOW_HZ)) * BANDS);
  let prev = toBand(BAR_LOW_HZ);
  for (let i = 0; i < count; i++) {
    const hi = BAR_LOW_HZ * Math.pow(BAR_HIGH_HZ / BAR_LOW_HZ, (i + 1) / count);
    const a = Math.min(prev, BANDS - 1);
    const b = Math.min(BANDS, Math.max(a + 1, toBand(hi)));
    out.push([a, b]);
    prev = b;
  }
  return out;
}

/**
 * The bars' heights, 0..1: each the loudest band it covers, rising fast and
 * settling slowly. A missing frame lets them all settle to nothing.
 */
export class BarLevels {
  readonly shown: Float32Array;
  private readonly ranges: [number, number][];
  private readonly tilt: Float32Array;

  constructor(count: number) {
    this.ranges = barBands(count);
    this.shown = new Float32Array(count);
    this.tilt = Float32Array.from(this.ranges, ([a, b]) => BAR_TILT_DB_OCT * Math.log2(Math.sqrt(bandEdge(a) * bandEdge(b)) / 1_000));
  }

  step(bands: ArrayLike<number> | null, dtMs: number): void {
    const dt = Math.max(0, Math.min(250, dtMs));
    for (let i = 0; i < this.shown.length; i++) {
      let want = 0;
      if (bands) {
        const [a, b] = this.ranges[i];
        let db = FLOOR_DB;
        for (let k = a; k < b; k++) {
          const v = bands[k];
          if (typeof v === "number" && Number.isFinite(v) && v > db) db = v;
        }
        if (db > FLOOR_DB) db += this.tilt[i];
        // A gentle curve, so loud and quiet bars stand further apart.
        want = BAR_MAX * Math.pow(Math.min(1, Math.max(0, (db - BAR_FLOOR_DB) / (BAR_TOP_DB - BAR_FLOOR_DB))), 1.4);
      }
      const cur = this.shown[i];
      this.shown[i] = follow(cur, want, dt, want > cur ? BAR_ATTACK_MS : BAR_RELEASE_MS);
    }
  }
}
