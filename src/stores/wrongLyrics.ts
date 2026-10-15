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
}

export const useWrongLyrics = create<WrongLyricsState>()(() => ({ track: null, tapping: null }));

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

/** Set a song's timing offset (rounded to 0.1 s, kept within a minute). */
export async function setLyricsOffset(trackId: string, offsetMs: number): Promise<void> {
  const v = Math.max(-MAX_OFFSET_MS, Math.min(MAX_OFFSET_MS, Math.round(offsetMs / 100) * 100));
  try {
    await backend.lyricsOffset(trackId, v);
    lyricsChanged(trackId);
  } catch (e) {
    toast.error(message(e));
  }
}
