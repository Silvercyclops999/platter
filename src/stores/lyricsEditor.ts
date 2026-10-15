/**
 * The lyrics editor: which song it's open for, and which songs have lyrics
 * the user wrote (so menus can say "Add lyrics" or "Edit lyrics"). `version`
 * goes up on every save or removal so the lyrics views ask again.
 */
import { create } from "zustand";
import { backend } from "../lib/backend";
import type { Track } from "../lib/types";
import { markReviewed } from "./lyricsShare";

interface LyricsEditorState {
  /** The song being edited, or null when the editor is closed. */
  track: Track | null;
  /** Songs with their own lyrics. */
  ownIds: ReadonlySet<string>;
  version: number;
}

export const useLyricsEditor = create<LyricsEditorState>()(() => ({
  track: null,
  ownIds: new Set(),
  version: 0,
}));

export function openLyricsEditor(track: Track) {
  useLyricsEditor.setState({ track });
  // Opening the editor counts as checking the lyrics (sharing a transcription).
  void markReviewed(track.id);
}

export function closeLyricsEditor() {
  useLyricsEditor.setState({ track: null });
}

/** Whether the song has lyrics the user wrote. */
export const hasOwnLyrics = (trackId: string) => useLyricsEditor.getState().ownIds.has(trackId);

/** "Add lyrics" or "Edit lyrics" for a song. */
export const lyricsEditLabel = (trackId: string) => (hasOwnLyrics(trackId) ? "Edit lyrics" : "Add lyrics");

/** Load which songs have their own lyrics (at startup). Quiet on failure. */
export async function loadOwnLyricsIds(): Promise<void> {
  try {
    const ids = await backend.ownLyricsIds();
    useLyricsEditor.setState({ ownIds: new Set(ids) });
  } catch {
    // Menus then say "Add lyrics"; the editor still loads what's saved.
  }
}

/** A save or removal: update the set and make the lyrics views ask again. */
export function ownLyricsChanged(trackId: string, has: boolean) {
  const ids = new Set(useLyricsEditor.getState().ownIds);
  if (has) ids.add(trackId);
  else ids.delete(trackId);
  useLyricsEditor.setState((s) => ({ ownIds: ids, version: s.version + 1 }));
}
