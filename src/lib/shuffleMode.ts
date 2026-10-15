/**
 * The shuffle button's states. Playlists and Liked Songs cycle off, shuffle,
 * smart shuffle (while its setting is on); everything else is on or off.
 */
import type { PlayContextKind, PlayerState } from "./types";

export type ShuffleMode = "off" | "shuffle" | "smart";

/** Smart shuffle is offered on playlists and Liked Songs only. */
export function smartOffered(kind: PlayContextKind | null | undefined): boolean {
  return kind === "playlist" || kind === "liked";
}

/** The stored mode of a context, playing or not. */
export function contextShuffleMode(state: PlayerState | null | undefined, id: string): ShuffleMode {
  if (state?.smartContexts?.includes(id)) return "smart";
  return state?.shuffledContexts?.includes(id) ? "shuffle" : "off";
}

/** The playing context's mode. */
export function playingShuffleMode(state: PlayerState | null | undefined): ShuffleMode {
  if (state?.smartShuffle) return "smart";
  return state?.shuffle ? "shuffle" : "off";
}

/** What a click turns it into. */
export function nextShuffleMode(mode: ShuffleMode, smart: boolean): ShuffleMode {
  if (mode === "off") return "shuffle";
  if (mode === "shuffle" && smart) return "smart";
  return "off";
}

/**
 * The button's name says what the next click does. With smart shuffle in the
 * cycle it names the playlist; otherwise it's a plain on/off.
 */
export function shuffleLabel(mode: ShuffleMode, smart: boolean, name?: string | null): string {
  if (!smart) return mode === "off" ? "Enable shuffle" : "Disable shuffle";
  const what = name ? ` for ${name}` : "";
  if (mode === "off") return `Turn on shuffle${what}`;
  if (mode === "shuffle") return `Turn on smart shuffle${what}`;
  return `Turn off smart shuffle${what}`;
}
