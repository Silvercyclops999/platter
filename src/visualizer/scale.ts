/**
 * The visualizer's scales and how its lines move. Everything here is plain
 * maths over the core's frames (docs/IPC.md "Visualizer"), so the views stay
 * thin and the numbers are tested on their own.
 */
import type { VisualizerSmoothing } from "../lib/types";

export const BANDS = 64;
export const LOW_HZ = 20;
export const HIGH_HZ = 20_000;
/** What silence reads, in dBFS. */
export const FLOOR_DB = -90;
/** The Analyser's dB scale: top and bottom of the plot. */
export const DB_TOP = 0;
export const DB_BOTTOM = -72;
/** The frequency labels on the Analyser's axis. */
export const FREQ_TICKS = [50, 200, 1_000, 5_000, 15_000] as const;
/** The dB lines on the Analyser. */
export const DB_TICKS = [0, -12, -24, -36, -48, -60, -72] as const;
/** Frames further apart than this are a gap: the lines fall to silence instead of freezing. */
export const GAP_MS = 500;
/** How often a watch is renewed (the core ends one 3 s after the last renewal). */
export const RENEW_MS = 1_000;

/** Lower edge of band `k` in Hz (k = BANDS is the top edge). */
export const bandEdge = (k: number) => LOW_HZ * Math.pow(HIGH_HZ / LOW_HZ, k / BANDS);
/** Band `k`'s centre in Hz (geometric). */
export const bandCentre = (k: number) => LOW_HZ * Math.pow(HIGH_HZ / LOW_HZ, (k + 0.5) / BANDS);

/** 0..1 across the log frequency axis. */
export const freqX = (hz: number) => Math.log(hz / LOW_HZ) / Math.log(HIGH_HZ / LOW_HZ);
/** 0 at the top of the plot, 1 at the bottom. */
export const dbY = (db: number) => {
  const v = (DB_TOP - db) / (DB_TOP - DB_BOTTOM);
  return Math.min(1, Math.max(0, v));
};

export function freqLabel(hz: number): string {
  return hz >= 1000 ? `${hz / 1000}k` : String(hz);
}

/** How long a line takes to fall back by about two thirds, per smoothing setting (ms). */
export const RELEASE_MS: Record<VisualizerSmoothing, number> = { low: 90, medium: 200, high: 450 };
/** Rises are quick at every setting, so a hit still reads as a hit. */
export const ATTACK_MS = 25;
/** The peak-hold line waits this long, then falls this fast. */
export const PEAK_HOLD_MS = 900;
export const PEAK_FALL_DB_PER_S = 14;

/** One step of a first-order follower towards `target`. */
export function follow(current: number, target: number, dtMs: number, tauMs: number): number {
  if (tauMs <= 0) return target;
  const k = 1 - Math.exp(-dtMs / tauMs);
  return current + (target - current) * k;
}

/**
 * The Analyser's lines: each band rises fast and falls by the smoothing
 * setting, and a peak-hold line sits over it (left out under reduced motion).
 */
export class AnalyserState {
  readonly shown = new Float32Array(BANDS).fill(FLOOR_DB);
  readonly peaks = new Float32Array(BANDS).fill(FLOOR_DB);
  private readonly held = new Float32Array(BANDS);

  /** Moves the lines `dtMs` on towards `target` (null: silence). */
  step(target: ArrayLike<number> | null, dtMs: number, smoothing: VisualizerSmoothing, peakHold: boolean): void {
    const release = RELEASE_MS[smoothing];
    for (let k = 0; k < BANDS; k++) {
      const want = target ? Math.max(FLOOR_DB, target[k] ?? FLOOR_DB) : FLOOR_DB;
      const cur = this.shown[k];
      const v = follow(cur, want, dtMs, want > cur ? ATTACK_MS : release);
      this.shown[k] = v;
      if (!peakHold) {
        this.peaks[k] = FLOOR_DB;
        continue;
      }
      if (v >= this.peaks[k]) {
        this.peaks[k] = v;
        this.held[k] = 0;
      } else {
        this.held[k] += dtMs;
        if (this.held[k] > PEAK_HOLD_MS) this.peaks[k] = Math.max(v, this.peaks[k] - (PEAK_FALL_DB_PER_S * dtMs) / 1000);
      }
    }
  }
}

/**
 * A VU needle: a critically damped second-order movement that reaches 99%
 * of a step in 300 ms, rising and falling alike, like the meters it copies.
 */
export const VU_MS = 300;
const VU_W = 6.64 / (VU_MS / 1000);

/** Where 0 VU sits, in dBFS RMS. The meter face prints it. */
export const VU_REF_DBFS = -10;
/** The marks on the VU scale. */
export const VU_MARKS = [-20, -10, -7, -5, -3, -2, -1, 0, 1, 2, 3] as const;
/** Peak lamp: lights at this many dBFS and stays lit a moment. */
export const PEAK_LAMP_DBFS = -1;
export const PEAK_LAMP_MS = 1_200;

/** 0..1 along the VU arc for a VU reading: the scale is linear in voltage, +3 at the end. */
export function vuPosition(vu: number): number {
  return Math.min(1.04, Math.max(0, Math.pow(10, (vu - 3) / 20)));
}

export class Needle {
  /** Amplitude relative to 0 VU (1 = 0 VU). */
  pos = 0;
  private vel = 0;

  /** Moves `dtMs` on towards an RMS reading in dBFS. */
  step(rmsDb: number, dtMs: number): void {
    const target = rmsDb <= FLOOR_DB ? 0 : Math.pow(10, (rmsDb - VU_REF_DBFS) / 20);
    // Small sub-steps keep the spring stable at low frame rates.
    let left = Math.min(dtMs, 250) / 1000;
    while (left > 0) {
      const h = Math.min(left, 1 / 240);
      const acc = VU_W * VU_W * (target - this.pos) - 2 * VU_W * this.vel;
      this.vel += acc * h;
      this.pos += this.vel * h;
      left -= h;
    }
    if (this.pos < 0) {
      this.pos = 0;
      this.vel = 0;
    }
  }

  /** The reading in VU (dB from 0 VU). */
  get vu(): number {
    return this.pos > 0 ? 20 * Math.log10(this.pos) : -Infinity;
  }

  /** 0..1 along the arc. */
  get arc(): number {
    return this.pos > 0 ? vuPosition(this.vu) : 0;
  }
}

/** The frame to draw: the latest one, or none once frames stop coming. */
export function current<T>(frame: T | null, arrivedAt: number, now: number): T | null {
  return frame && now - arrivedAt <= GAP_MS ? frame : null;
}

/** The low end's level (bands under 150 Hz), 0..1. */
export function lowEnd(bands: ArrayLike<number> | null): number {
  if (!bands) return 0;
  let sum = 0;
  let n = 0;
  for (let k = 0; k < BANDS && bandCentre(k) < 150; k++) {
    sum += Math.pow(10, Math.max(FLOOR_DB, bands[k]) / 20);
    n++;
  }
  return n ? Math.min(1, (sum / n) * 2) : 0;
}

/**
 * The goniometer's point for a stereo pair: mid up, side across (left
 * channel alone on the upper-left diagonal), so a mono signal draws a
 * vertical line and a wide one spreads out.
 */
export function scopePoint(l: number, r: number): [x: number, y: number] {
  return [(r - l) * Math.SQRT1_2, (l + r) * Math.SQRT1_2];
}
