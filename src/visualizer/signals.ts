/**
 * What the art styles move with, worked out from the core's spectrum frames
 * in the page (cheap, no engine work): the low, mid and high energy, each
 * smoothed and scaled to the song's own range so a quiet record moves as
 * well as a loud one, and a beat pulse from spectral flux in the low bands.
 * Imperative and allocation free per frame: the views keep one of these and
 * step it from their animation frames, never through React state.
 */
import type { VisualizerIntensity } from "../lib/types";
import { BANDS, bandCentre, follow } from "./scale";

/** Bands whose centre sits in [lo, hi) Hz. */
function bandRange(lo: number, hi: number): [number, number] {
  let a = BANDS;
  let b = 0;
  for (let k = 0; k < BANDS; k++) {
    const c = bandCentre(k);
    if (c >= lo && c < hi) {
      a = Math.min(a, k);
      b = Math.max(b, k + 1);
    }
  }
  return [a, b];
}

/** The kick and bass (beats are found here), the body of the mix, and the air. */
export const LOW_BANDS = bandRange(35, 160);
export const MID_BANDS = bandRange(250, 2_500);
export const HIGH_BANDS = bandRange(4_000, 14_000);

/** A band's dB as 0..1: -72 dB is nothing, -6 dB is full. */
const unit = (db: number) => Math.min(1, Math.max(0, (db + 72) / 66));

function mean(bands: ArrayLike<number>, [a, b]: [number, number]): number {
  let s = 0;
  for (let k = a; k < b; k++) s += unit(bands[k]);
  return b > a ? s / (b - a) : 0;
}

/** How each intensity moves: how far the art reacts, and how long a beat's pulse lasts. */
export const INTENSITY: Record<VisualizerIntensity, { gain: number; pulse: number; pulseMs: number; releaseMs: number }> = {
  calm: { gain: 0.6, pulse: 0.6, pulseMs: 260, releaseMs: 320 },
  lively: { gain: 1, pulse: 1, pulseMs: 180, releaseMs: 170 },
};

/** Beats closer together than this are one beat. */
export const BEAT_MIN_GAP_MS = 200;
/** Flux (mean dB rise across the low bands) under this is never a beat, however quiet the song. */
export const BEAT_MIN_FLUX = 1.2;
/** How many standard deviations over the recent mean a rise must be. */
export const BEAT_K = 1.6;
/** The flux statistics follow the song over about this long. */
const FLUX_TAU_MS = 1_200;

/**
 * One energy's range follower: the floor creeps up slowly and drops fast,
 * the ceiling rises fast and sinks slowly, and the value is read as where it
 * sits between them. A silence gate keeps nothing at nothing.
 */
class Range {
  lo = 0.4;
  hi = 0.6;
  read(v: number, dt: number): number {
    this.lo = v < this.lo ? follow(this.lo, v, dt, 120) : follow(this.lo, v, dt, 3_500);
    this.hi = v > this.hi ? follow(this.hi, v, dt, 60) : follow(this.hi, v, dt, 4_000);
    const span = Math.max(0.14, this.hi - this.lo);
    const rel = Math.min(1, Math.max(0, (v - this.lo) / span));
    const gate = Math.min(1, Math.max(0, (v - 0.04) / 0.2));
    // Mostly relative, a little absolute, so loud passages still read as loud.
    return gate * (0.72 * rel + 0.28 * v);
  }
}

export class MusicSignals {
  /** Smoothed energies, 0..1. */
  low = 0;
  mid = 0;
  high = 0;
  /** The beat pulse: 1 on a beat, falling back to 0. */
  pulse = 0;
  /** Beats found so far (tests and the ripples). */
  beatCount = 0;
  /** When the last few beats came (stage seconds) and how strong they were, newest first. */
  readonly beatAt = new Float32Array(4).fill(-100);
  readonly beatStrength = new Float32Array(4);
  /** Slow phases the styles drift by: the flow speeds up with the bass, the drift with the mids. */
  flow = 0;
  drift = 0;

  private readonly prev = new Float32Array(BANDS);
  private havePrev = false;
  private lastFrame: unknown = null;
  private lastBeat = -Infinity;
  private fluxMean = 0;
  private fluxVar = 1;
  private readonly ranges = { low: new Range(), mid: new Range(), high: new Range() };
  private raw = { low: 0, mid: 0, high: 0 };

  /**
   * Moves everything `dtMs` on. `bands` is the latest frame's (null when
   * frames stopped: everything falls back to rest), `frameKey` tells a new
   * frame from the same one drawn again, `nowMs` is the stage's clock.
   */
  step(bands: ArrayLike<number> | null, frameKey: unknown, dtMs: number, nowMs: number, intensity: VisualizerIntensity = "calm"): void {
    const how = INTENSITY[intensity];
    const dt = Math.max(0, Math.min(250, dtMs));
    if (bands && frameKey !== this.lastFrame) {
      this.lastFrame = frameKey;
      this.raw = { low: mean(bands, LOW_BANDS), mid: mean(bands, MID_BANDS), high: mean(bands, HIGH_BANDS) };
      this.onset(bands, nowMs, how.pulse);
    } else if (!bands) {
      this.raw = { low: 0, mid: 0, high: 0 };
      this.havePrev = false;
      this.lastFrame = null;
    }
    const lo = this.ranges.low.read(this.raw.low, dt) * how.gain;
    const mi = this.ranges.mid.read(this.raw.mid, dt) * how.gain;
    const hi = this.ranges.high.read(this.raw.high, dt) * how.gain;
    // Quick to rise, slower to settle.
    this.low = follow(this.low, lo, dt, lo > this.low ? 40 : how.releaseMs);
    this.mid = follow(this.mid, mi, dt, mi > this.mid ? 70 : how.releaseMs * 1.4);
    this.high = follow(this.high, hi, dt, hi > this.high ? 30 : how.releaseMs * 0.8);
    this.pulse *= Math.exp(-dt / how.pulseMs);
    this.flow += (dt / 1000) * (0.1 + 0.6 * this.low + 0.25 * this.pulse);
    this.drift += (dt / 1000) * (0.04 + 0.16 * this.mid);
  }

  /** Spectral flux over the low bands against its own recent statistics. */
  private onset(bands: ArrayLike<number>, nowMs: number, pulseGain: number) {
    const [a, b] = LOW_BANDS;
    let flux = 0;
    if (this.havePrev) {
      for (let k = a; k < b; k++) flux += Math.max(0, bands[k] - this.prev[k]);
      flux /= b - a;
    }
    for (let k = 0; k < BANDS; k++) this.prev[k] = bands[k];
    const first = !this.havePrev;
    this.havePrev = true;
    if (first) return;
    const sd = Math.sqrt(this.fluxVar);
    const over = flux - this.fluxMean;
    const beat = flux > BEAT_MIN_FLUX && over > BEAT_K * sd && nowMs - this.lastBeat >= BEAT_MIN_GAP_MS;
    // The statistics: a running mean and variance over about FLUX_TAU_MS of frames.
    const k = 1 - Math.exp(-33 / FLUX_TAU_MS);
    this.fluxMean += (flux - this.fluxMean) * k;
    this.fluxVar += (over * over - this.fluxVar) * k;
    if (!beat) return;
    this.lastBeat = nowMs;
    this.beatCount++;
    const strength = Math.min(1, Math.max(0.45, over / (3 * sd + 1)));
    this.pulse = Math.max(this.pulse, strength * pulseGain);
    this.beatAt.copyWithin(1, 0);
    this.beatStrength.copyWithin(1, 0);
    this.beatAt[0] = nowMs / 1000;
    this.beatStrength[0] = strength;
  }
}
