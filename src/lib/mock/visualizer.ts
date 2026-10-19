/**
 * The visualizer in the browser mock: frames shaped like music (a spectrum
 * that falls off towards the top, a kick and a bass line on the beat, a
 * moving melody, a slightly wide stereo image) at the asked rate, only while
 * watched and playing, ending 3 s after the last renewal like the core.
 */
import type { SpectrumFrame } from "../types";

export const MOCK_BANDS = 64;
export const MOCK_WATCH_MS = 3_000;
const FLOOR = -90;
const SCOPE_POINTS = 256;

const toDb = (v: number) => (v > 0 ? Math.max(FLOOR, 20 * Math.log10(v)) : FLOOR);

/** Band k's centre in Hz (the core's 20 Hz to 20 kHz log bands). */
const centre = (k: number) => 20 * Math.pow(1000, (k + 0.5) / MOCK_BANDS);

/** A frame `ms` into a song at 120 bpm. Deterministic, so tests can check it. */
export function mockFrame(ms: number, scope: boolean): SpectrumFrame {
  const beat = (ms / 500) % 1;
  const kick = Math.exp(-beat * 9);
  const bar = Math.floor(ms / 2000);
  const melody = 440 * Math.pow(2, [0, 3, 7, 5, 10, 7, 3, 2][bar % 8] / 12);
  const bands: number[] = [];
  for (let k = 0; k < MOCK_BANDS; k++) {
    const f = centre(k);
    // Pink-ish: about 4 dB down per octave above 80 Hz, a roll-off at the top.
    let db = -14 - 4 * Math.max(0, Math.log2(f / 80)) - (f > 14_000 ? (f - 14_000) / 400 : 0);
    if (f < 120) db += 10 * kick - 4;
    db += 9 * Math.exp(-Math.pow(Math.log2(f / melody) * 3, 2));
    db += 9 * Math.exp(-Math.pow(Math.log2(f / (melody * 2)) * 3, 2)) - 3;
    db += 2.5 * Math.sin(ms / 170 + k * 1.7) * Math.sin(ms / 410 + k * 0.6);
    bands.push(Math.max(FLOOR, Math.min(0, db)));
  }
  const level = 0.13 + 0.1 * kick + 0.02 * Math.sin(ms / 700);
  const sway = 0.04 * Math.sin(ms / 1300);
  const rms: [number, number] = [toDb(level * (1 + sway)), toDb(level * (1 - sway))];
  const peak: [number, number] = [toDb(Math.min(1, level * 4.2 * (1 + sway))), toDb(Math.min(1, level * 4 * (1 - sway)))];
  const frame: SpectrumFrame = { t: Math.round(ms), bands, rms, peak };
  if (scope) {
    const pairs: number[] = [];
    for (let i = 0; i < SCOPE_POINTS; i++) {
      const t = ms / 1000 + i / 6_000;
      const mid = level * 1.6 * (Math.sin(2 * Math.PI * 82 * t) * kick + 0.6 * Math.sin(2 * Math.PI * melody * t + 0.3));
      const side = level * 0.35 * Math.sin(2 * Math.PI * melody * 1.5 * t + 1.1);
      pairs.push(Math.max(-1, Math.min(1, mid + side)), Math.max(-1, Math.min(1, mid - side)));
    }
    frame.scope = pairs;
  }
  return frame;
}

export class MockVisualizer {
  private timer: ReturnType<typeof setInterval> | null = null;
  private until = 0;
  private fps = 30;
  private scope = false;
  private started = 0;
  /** Calls to `watch` (tests). */
  calls: { on: boolean; fps: number; scope: boolean }[] = [];

  constructor(
    private readonly emit: (f: SpectrumFrame) => void,
    private readonly playing: () => boolean,
    private readonly now: () => number = () => Date.now(),
  ) {}

  watch(on: boolean, fps: number, scope: boolean): boolean {
    this.calls.push({ on, fps, scope });
    if (!on) {
      this.stop();
      return false;
    }
    const fresh = this.now() >= this.until;
    this.until = this.now() + MOCK_WATCH_MS;
    this.scope = scope;
    const want = Math.min(60, Math.max(1, Math.round(fps)));
    if (fresh) this.started = this.now();
    if (!this.timer || want !== this.fps) {
      this.fps = want;
      if (this.timer) clearInterval(this.timer);
      this.timer = setInterval(() => this.frame(), 1000 / want);
    }
    return true;
  }

  get watched(): boolean {
    return this.now() < this.until;
  }

  private frame() {
    if (!this.watched) {
      this.stop();
      return;
    }
    if (this.playing()) this.emit(mockFrame(this.now() - this.started, this.scope));
  }

  stop() {
    if (this.timer) clearInterval(this.timer);
    this.timer = null;
    this.until = 0;
  }
}
