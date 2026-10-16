/**
 * What every key draws from: the latest state, softened so keys don't
 * flicker. Platter missing for a moment keeps the last picture for a grace
 * period before keys say it isn't running; a song that just stopped stays
 * up for a few seconds; the elapsed time runs on between polls.
 */
import type { PlatterError, PlatterState } from "./client.js";

/**
 * - `ok`: Platter answers.
 * - `connecting`: no answer yet, still within the grace period.
 * - `offline`: Platter isn't running (or its control API is off).
 * - `unpaired`: Platter answers but this plugin isn't paired.
 * - `approve`: Platter is asking whether to allow this plugin (choose Allow there).
 * - `refused`: Platter turned the request away (Allow Stream Deck off).
 */
export type Conn = "ok" | "connecting" | "offline" | "unpaired" | "approve" | "refused";

export interface Snapshot {
  conn: Conn;
  /** The state to draw: the live one, the held last song, or null. */
  state: PlatterState | null;
  /** `state` is the last song, kept after playback stopped. */
  held: boolean;
  /** When `state` arrived (ms), for the elapsed time. */
  at: number;
}

/** No answer for this long and keys say Platter isn't running. */
export const GRACE_MS = 8000;
/** A song stays on the keys this long after playback stops. */
export const HOLD_MS = 8000;

/** A song is loaded (a private session hides its title, not its length). */
export const hasSong = (s: PlatterState | null | undefined): s is PlatterState => !!s && (!!s.trackId || s.durationMs > 0);

export class Live {
  private lastOk: number | null = null;
  private song: { state: PlatterState; at: number } | null = null;
  private snap: Snapshot;

  constructor(
    private readonly now: () => number = Date.now,
    private readonly graceMs = GRACE_MS,
    private readonly holdMs = HOLD_MS,
  ) {
    this.snap = { conn: "connecting", state: null, held: false, at: now() };
    this.started = now();
  }

  private readonly started: number;

  get snapshot(): Snapshot {
    return this.snap;
  }

  /** Take one poll's result; gives the snapshot keys draw. */
  update(s: PlatterState | null, err: PlatterError | null): Snapshot {
    const t = this.now();
    if (s) {
      this.lastOk = t;
      if (s.trackId || s.durationMs > 0) {
        this.song = { state: s, at: t };
        this.snap = { conn: "ok", state: s, held: false, at: t };
      } else if (this.song && t - this.song.at < this.holdMs) {
        // Playback stopped: keep the last song up, paused, a little longer.
        const held = { ...this.song.state, playing: false, volume: s.volume, muted: s.muted, sleep: s.sleep };
        this.snap = { conn: "ok", state: held, held: true, at: t };
      } else {
        this.song = null;
        this.snap = { conn: "ok", state: s, held: false, at: t };
      }
      return this.snap;
    }
    const kind = err?.kind ?? "failed";
    if (kind === "busy") return this.snap;
    if (kind === "approve") {
      this.snap = { conn: "approve", state: null, held: false, at: t };
      return this.snap;
    }
    if (kind === "unpaired" || kind === "wrongCode" || kind === "denied") {
      this.snap = { conn: "unpaired", state: null, held: false, at: t };
      return this.snap;
    }
    if (kind === "refused") {
      this.snap = { conn: "refused", state: null, held: false, at: t };
      return this.snap;
    }
    // Offline or failed: hold on through the grace period.
    const since = this.lastOk ?? this.started;
    if (t - since < this.graceMs) {
      this.snap = { ...this.snap, conn: "connecting" };
    } else {
      this.song = null;
      this.snap = { conn: "offline", state: null, held: false, at: t };
    }
    return this.snap;
  }
}

/** Elapsed ms at `now`: the polled position, run on while playing. */
export function elapsed(snap: Snapshot, now: number): number {
  const s = snap.state;
  if (!s) return 0;
  const run = s.playing && !snap.held ? Math.max(0, now - snap.at) : 0;
  const pos = s.positionMs + run;
  return s.durationMs > 0 ? Math.min(pos, s.durationMs) : pos;
}

/** 0 to 1 through the song. */
export function progress(snap: Snapshot, now: number): number {
  const d = snap.state?.durationMs ?? 0;
  return d > 0 ? Math.max(0, Math.min(1, elapsed(snap, now) / d)) : 0;
}

/** m:ss, or h:mm:ss past an hour. */
export function clock(ms: number): string {
  const total = Math.max(0, Math.floor(ms / 1000));
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const sec = String(total % 60).padStart(2, "0");
  return h > 0 ? `${h}:${String(m).padStart(2, "0")}:${sec}` : `${m}:${sec}`;
}
