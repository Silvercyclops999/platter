/** What each key shows for a state. Kept apart from the Stream Deck code so it can be tested. */
import type { PlatterError, PlatterState } from "./client.js";

export interface KeyView {
  /** 0 or 1: which of the action's manifest images. */
  state: 0 | 1;
  /** An image over the manifest one (the cover), or undefined for the manifest image. */
  image?: string;
  title?: string;
}

/** Short words for a key when Platter can't be used. */
export function problemTitle(err: PlatterError | null): string | undefined {
  if (!err) return undefined;
  if (err.kind === "unpaired" || err.kind === "wrongCode") return "Pair";
  if (err.kind === "offline") return "Off";
  return undefined;
}

/** Play / Pause: the cover when there is one, else play or pause. */
export function playPauseView(s: PlatterState | null, err: PlatterError | null): KeyView {
  if (!s) return { state: 0, title: problemTitle(err) };
  return { state: s.playing ? 1 : 0, image: s.cover ?? undefined };
}

export function likeView(s: PlatterState | null, err: PlatterError | null): KeyView {
  if (!s) return { state: 0, title: problemTitle(err) };
  return { state: s.liked ? 1 : 0 };
}

export function plainView(s: PlatterState | null, err: PlatterError | null): KeyView {
  return { state: 0, title: s ? undefined : problemTitle(err) };
}

/** The Stream Deck + dial's $B1 layout. */
export function volumeFeedback(s: PlatterState | null, err: PlatterError | null): { title: string; value: string; indicator: number } {
  if (!s) return { title: "Platter", value: problemTitle(err) ?? "", indicator: 0 };
  const v = s.muted ? 0 : Math.max(0, Math.min(100, Math.round(s.volume)));
  return { title: "Volume", value: s.muted ? "Muted" : `${v}%`, indicator: v };
}

/** Volume points per dial tick. */
export const DIAL_STEP = 2;
