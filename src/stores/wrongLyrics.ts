/**
 * "Wrong lyrics?": which song the picker is open for, and the song whose
 * lines are being tapped to set the timing ("Tap to sync").
 */
import { create } from "zustand";
import { backend } from "../lib/backend";
import type { AppError, Track } from "../lib/types";
import { lyricsChanged } from "../shell/lyrics";
import { toast } from "./toasts";

interface WrongLyricsState {
  /** The song the picker is open for, or null. */
  track: Track | null;
  /** The song whose lines set the timing when clicked, or null. */
  tapping: string | null;
  /** Timing offsets asked for but not yet in the lyrics shown, per song (shown at once). */
  offsets: Record<string, number>;
}

export const useWrongLyrics = create<WrongLyricsState>()(() => ({ track: null, tapping: null, offsets: {} }));

export function openWrongLyrics(track: Track) {
  useWrongLyrics.setState({ track, tapping: null });
}

export function closeWrongLyrics() {
  useWrongLyrics.setState({ track: null });
}

export function setTapping(trackId: string | null) {
  useWrongLyrics.setState({ tapping: trackId });
}

const message = (e: unknown) => (e && typeof e === "object" && "message" in e ? String((e as AppError).message) : "Something went wrong.");

/** Pick a song's lyrics (a candidate key, "none", or null for automatic). */
export async function chooseLyrics(trackId: string, pick: string | null): Promise<boolean> {
  try {
    await backend.lyricsChoose(trackId, pick);
    lyricsChanged(trackId);
    return true;
  } catch (e) {
    toast.error(message(e));
    return false;
  }
}

/** Largest nudge the core takes. */
export const MAX_OFFSET_MS = 60_000;
/** One press of the nudge buttons. */
export const NUDGE_MS = 500;

const clampOffset = (ms: number) => Math.max(-MAX_OFFSET_MS, Math.min(MAX_OFFSET_MS, Math.round(ms / 100) * 100));

/** Writes going out per song: one at a time, the last value asked for wins. */
const writing = new Map<string, Promise<void>>();

function setPending(trackId: string, v: number | null) {
  useWrongLyrics.setState((s) => {
    const offsets = { ...s.offsets };
    if (v === null) delete offsets[trackId];
    else offsets[trackId] = v;
    return { offsets };
  });
}

/** The offset the song shows: the one asked for, until the lyrics carry it. */
export function shownOffset(trackId: string, fetchedMs: number | undefined): number {
  return useWrongLyrics.getState().offsets[trackId] ?? fetchedMs ?? 0;
}

/** The lyrics now carry `fetchedMs`: an offset asked for that matches is done. */
export function offsetArrived(trackId: string, fetchedMs: number) {
  if (!writing.has(trackId) && useWrongLyrics.getState().offsets[trackId] === fetchedMs) setPending(trackId, null);
}

/**
 * Set a song's timing offset (rounded to 0.1 s, kept within a minute). It
 * shows at once; writes go out one at a time, and presses made while one is
 * out are folded into the next, so none is lost.
 */
export function setLyricsOffset(trackId: string, offsetMs: number): Promise<void> {
  setPending(trackId, clampOffset(offsetMs));
  const running = writing.get(trackId);
  if (running) return running;
  const run = (async () => {
    let sent: number | undefined;
    try {
      for (;;) {
        const want = useWrongLyrics.getState().offsets[trackId];
        if (want === undefined || want === sent) break;
        await backend.lyricsOffset(trackId, want);
        sent = want;
      }
      writing.delete(trackId);
      lyricsChanged(trackId);
    } catch (e) {
      writing.delete(trackId);
      setPending(trackId, null);
      toast.error(message(e));
    }
  })();
  writing.set(trackId, run);
  return run;
}

/** Nudge a song's timing by `deltaMs` from what it shows now (the last press included). */
export function nudgeLyricsOffset(trackId: string, fetchedMs: number | undefined, deltaMs: number): Promise<void> {
  return setLyricsOffset(trackId, shownOffset(trackId, fetchedMs) + deltaMs);
}
