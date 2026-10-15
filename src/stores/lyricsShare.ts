/**
 * Sharing a transcription to LRCLIB (docs/IPC.md "Sharing lyrics"): which
 * songs the user reviewed, so the lyrics view asks again. Reviewing is
 * opening the lyrics editor for the song, or playing it through with its
 * lyrics showing.
 */
import { create } from "zustand";
import { backend } from "../lib/backend";

/** Played this much of the song with lyrics showing counts as reviewed. */
export const PLAYED_THROUGH = 0.9;

interface LyricsShareState {
  /** Goes up when a song was reviewed or shared, so the offer asks again. */
  version: number;
}

export const useLyricsShare = create<LyricsShareState>()(() => ({ version: 0 }));

export function lyricsShareChanged() {
  useLyricsShare.setState((s) => ({ version: s.version + 1 }));
}

/** The user reviewed the song's lyrics. */
export async function markReviewed(trackId: string): Promise<void> {
  try {
    await backend.lyricsShareReviewed(trackId);
    lyricsShareChanged();
  } catch {
    // Not offering sharing yet is fine.
  }
}
