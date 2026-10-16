/**
 * Press and dial timing, apart from the Stream Deck code so it can be tested
 * with fake timers: single, double, triple and long presses, hold to
 * repeat, and an overlay that hides a while after the last turn.
 */

export type Gesture = "single" | "double" | "triple" | "long";

/** A press held this long is a long press. */
export const LONG_MS = 450;
/** Presses closer than this count together (double, triple). */
export const GAP_MS = 300;

/**
 * Tells single, double, triple and long presses apart. A long press fires
 * while the key is still held; a triple fires on the third press; single
 * and double wait out the gap. In a multi-action a press is always one
 * single, at once.
 */
export class Presses {
  private count = 0;
  private longTimer: ReturnType<typeof setTimeout> | null = null;
  private gapTimer: ReturnType<typeof setTimeout> | null = null;
  private longFired = false;
  private skipUp = false;

  constructor(
    private readonly onGesture: (g: Gesture) => void,
    private readonly opts: { longMs?: number; gapMs?: number; double?: boolean; long?: boolean } = {},
  ) {}

  private get longMs() {
    return this.opts.longMs ?? LONG_MS;
  }
  private get gapMs() {
    return this.opts.gapMs ?? GAP_MS;
  }

  down(inMultiAction = false): void {
    if (inMultiAction) {
      this.skipUp = true;
      this.reset();
      this.onGesture("single");
      return;
    }
    this.skipUp = false;
    if (this.gapTimer) clearTimeout(this.gapTimer);
    this.gapTimer = null;
    this.count++;
    this.longFired = false;
    if (this.count === 1 && this.opts.long !== false) {
      this.longTimer = setTimeout(() => {
        this.longTimer = null;
        this.longFired = true;
        this.count = 0;
        this.onGesture("long");
      }, this.longMs);
    }
  }

  up(): void {
    if (this.skipUp) {
      this.skipUp = false;
      return;
    }
    if (this.longTimer) clearTimeout(this.longTimer);
    this.longTimer = null;
    if (this.longFired) {
      this.longFired = false;
      return;
    }
    if (this.count === 0) return;
    if (this.opts.double === false) {
      this.count = 0;
      this.onGesture("single");
      return;
    }
    if (this.count >= 3) {
      this.count = 0;
      this.onGesture("triple");
      return;
    }
    this.gapTimer = setTimeout(() => {
      this.gapTimer = null;
      const n = this.count;
      this.count = 0;
      this.onGesture(n >= 2 ? "double" : "single");
    }, this.gapMs);
  }

  /** Forget a press in progress (the key went away). */
  reset(): void {
    if (this.longTimer) clearTimeout(this.longTimer);
    if (this.gapTimer) clearTimeout(this.gapTimer);
    this.longTimer = this.gapTimer = null;
    this.count = 0;
    this.longFired = false;
  }
}

/** Holding a key repeats it after this long, ... */
export const REPEAT_AFTER_MS = 400;
/** ... this often. */
export const REPEAT_EVERY_MS = 150;

/** Fires at once on press, then again and again while held. */
export class HoldRepeat {
  private wait: ReturnType<typeof setTimeout> | null = null;
  private every: ReturnType<typeof setInterval> | null = null;

  constructor(private readonly fire: () => void) {}

  down(inMultiAction = false): void {
    this.up();
    this.fire();
    if (inMultiAction) return;
    this.wait = setTimeout(() => {
      this.wait = null;
      this.every = setInterval(this.fire, REPEAT_EVERY_MS);
    }, REPEAT_AFTER_MS);
  }

  up(): void {
    if (this.wait) clearTimeout(this.wait);
    if (this.every) clearInterval(this.every);
    this.wait = this.every = null;
  }
}

/** The volume overlay on a dial hides this long after the last turn. */
export const OVERLAY_MS = 2000;

/** Shown on each poke, hidden `ms` after the last one. */
export class Overlay {
  private timer: ReturnType<typeof setTimeout> | null = null;
  visible = false;

  constructor(
    private readonly onChange: (visible: boolean) => void,
    private readonly ms = OVERLAY_MS,
  ) {}

  poke(): void {
    if (this.timer) clearTimeout(this.timer);
    const was = this.visible;
    this.visible = true;
    if (!was) this.onChange(true);
    this.timer = setTimeout(() => {
      this.timer = null;
      this.visible = false;
      this.onChange(false);
    }, this.ms);
  }

  hide(): void {
    if (this.timer) clearTimeout(this.timer);
    this.timer = null;
    if (this.visible) {
      this.visible = false;
      this.onChange(false);
    }
  }
}

/** Two dial presses closer than this switch the dial's view. */
export const DIAL_DOUBLE_MS = 300;
