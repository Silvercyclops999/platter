/**
 * The share card dialog: which item it's for, and the lines picked in the
 * lyrics view for a lyric card.
 */
import { create } from "zustand";
import type { ShareKind } from "../lib/types";
import { LYRIC_LINES_MAX } from "../lib/shareCard";

/** What a card is about. Title and subtitle are looked up when not given. */
export interface CardSubject {
  kind: Exclude<ShareKind, "artist">;
  id: string;
  title?: string;
  subtitle?: string;
  thumb?: string | null;
  durationMs?: number;
  /** A lyric card's lines. */
  lyrics?: string[];
}

interface ShareCardStore {
  subject: CardSubject | null;
  open: (s: CardSubject) => void;
  close: () => void;
}

export const useShareCard = create<ShareCardStore>()((set) => ({
  subject: null,
  open: (subject) => set({ subject }),
  close: () => set({ subject: null }),
}));

export const openShareCard = (s: CardSubject) => useShareCard.getState().open(s);

/** Picking lyric lines for a card, in the lyrics view. */
interface LyricPickStore {
  /** Picking is on (clicks pick lines instead of seeking). */
  picking: boolean;
  /** The song the picks are for. */
  trackId: string | null;
  /** Line indexes, in song order. */
  picked: number[];
  start: (trackId: string) => void;
  toggle: (i: number) => void;
  cancel: () => void;
}

export const useLyricPick = create<LyricPickStore>()((set, get) => ({
  picking: false,
  trackId: null,
  picked: [],
  start: (trackId) => set({ picking: true, trackId, picked: [] }),
  toggle: (i) => {
    const p = get().picked;
    if (p.includes(i)) set({ picked: p.filter((x) => x !== i) });
    else if (p.length < LYRIC_LINES_MAX) set({ picked: [...p, i].sort((a, b) => a - b) });
  },
  cancel: () => set({ picking: false, trackId: null, picked: [] }),
}));
