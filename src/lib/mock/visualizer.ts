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

/** A bump `db` high around `hz`, `width` octaves wide, at band centre `f`. */
const bump = (f: number, hz: number, width: number, db: number) => db * Math.exp(-Math.pow(Math.log2(f / hz) / width, 2));
/** Rising edge `from`..`to` Hz, flat between, for noise-like hits. */
const shelf = (f: number, from: number, to: number) => (f < from ? Math.exp(-Math.pow(Math.log2(from / f) * 1.6, 2)) : f > to ? Math.exp(-Math.pow(Math.log2(f / to) * 1.6, 2)) : 1);
/** A small, repeatable wobble per band and moment. */
const jitter = (k: number, n: number) => {
  const x = Math.sin(k * 12.9898 + n * 78.233) * 43758.5453;
  return x - Math.floor(x) - 0.5;
};

/** The chords, two bars each (semitones over A). */
const CHORDS = [
  [0, 3, 7],
  [-4, 0, 3],
  [-7, -3, 0],
  [-2, 2, 5],
];
const BASS = [0, 0, -4, -4, -7, -7, -2, -5];

/**
 * A frame `ms` into a song at 120 bpm, shaped like a band playing:
 * a kick on every beat (and a pickup before the third every other bar),
 * a snare on two and four, hats on the eighths (sixteenths in the chorus),
 * a bass line, chords swelling over each bar, a melody, and sections: eight
 * bars of verse, eight of chorus, and every 32 bars two bars of breakdown
 * without drums. Deterministic, so tests can check it.
 */
export function mockFrame(ms: number, scope: boolean): SpectrumFrame {
  const beatN = Math.floor(ms / 500);
  const beat = (ms / 500) % 1;
  const bar = Math.floor(ms / 2000);
  const inBar = beatN % 4;
  const chorus = bar % 16 >= 8;
  const breakdown = bar % 32 >= 30;
  const drums = breakdown ? 0 : 1;
  // Hits: time since each one, in ms.
  const sinceBeat = beat * 500;
  const pickup = bar % 2 === 1 && inBar === 1 && beat > 0.75 ? (beat - 0.75) * 500 : Infinity;
  const kick = drums * Math.max(Math.exp(-sinceBeat / 75), Math.exp(-pickup / 75) * 0.8);
  const snare = drums * (inBar === 1 || inBar === 3 ? Math.exp(-sinceBeat / 120) : 0);
  const hatStep = chorus ? 125 : 250;
  const sinceHat = ms % hatStep;
  const hat = drums * Math.exp(-sinceHat / 38) * (Math.floor(ms / hatStep) % 2 ? 0.75 : 1);
  const open = drums * (inBar === 3 && beat > 0.5 ? Math.exp(-(beat - 0.5) * 500 / 180) : 0);
  const chord = CHORDS[Math.floor(bar / 2) % CHORDS.length];
  const swell = 0.55 + 0.45 * Math.sin(Math.PI * Math.min(1, (ms % 2000) / 1600));
  const root = 110 * Math.pow(2, BASS[beatN % BASS.length] / 12);
  const melody = 440 * Math.pow(2, [0, 3, 7, 5, 10, 7, 3, 2][(beatN >> 1) % 8] / 12);
  const lift = chorus ? 4 : 0;
  const bands: number[] = [];
  for (let k = 0; k < MOCK_BANDS; k++) {
    const f = centre(k);
    // Pink-ish: about 4 dB down per octave above 80 Hz, a roll-off at the top.
    let db = -30 - 4 * Math.max(0, Math.log2(f / 80)) - (f > 14_000 ? (f - 14_000) / 300 : 0);
    db += bump(f, 55, 0.55, 20 * kick);
    db += bump(f, root / 2, 0.35, 15) + bump(f, root, 0.3, 9);
    for (const n of chord) db += bump(f, 220 * Math.pow(2, n / 12), 0.12, (9 + lift) * swell) + bump(f, 440 * Math.pow(2, n / 12), 0.1, (6 + lift) * swell);
    db += bump(f, melody, 0.1, 12 + lift) + bump(f, melody * 2, 0.08, 6 + lift);
    db += shelf(f, 180, 4_000) * 14 * snare + shelf(f, 5_000, 11_000) * 10 * snare;
    db += shelf(f, 7_000, 15_000) * (12 * hat + 9 * open + lift);
    db += 2.2 * jitter(k, Math.floor(ms / 33));
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
