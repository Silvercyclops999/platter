/**
 * Tags read from the music files (explicit, versions, editions): badges for
 * songs and albums, edition groups, and the scan's progress. Loaded from the
 * core once and again whenever the scan or a library sync changes them
 * (docs/IPC.md "File tags").
 */
import { create } from "zustand";
import { backend, type Backend } from "../lib/backend";
import type { AlbumBadges, Edition, TagIndex, TagScanStatus, TrackBadges } from "../lib/types";

interface FileTagsState {
  index: TagIndex;
  scan: TagScanStatus | null;
  setIndex: (index: TagIndex) => void;
  setScan: (scan: TagScanStatus) => void;
}

const EMPTY: TagIndex = { tracks: {}, albums: {}, groups: {} };

export const useFileTags = create<FileTagsState>()((set) => ({
  index: EMPTY,
  scan: null,
  setIndex: (index) => set({ index }),
  setScan: (scan) => set({ scan }),
}));

const NONE: TrackBadges = {};

/** A song's badges (an empty object when its file has nothing to show). */
export const useTrackBadges = (trackId: string | null | undefined): TrackBadges =>
  useFileTags((s) => (trackId ? (s.index.tracks[trackId] ?? NONE) : NONE));

/** An album's tags, if its files have any. */
export const useAlbumBadges = (albumId: string | null | undefined): AlbumBadges | undefined =>
  useFileTags((s) => (albumId ? s.index.albums[albumId] : undefined));

/** The editions an album belongs to (face first), or [] when it has none. */
export function editionsOf(index: TagIndex, albumId: string): Edition[] {
  const group = index.albums[albumId]?.group;
  const list = group ? index.groups[group] : undefined;
  return list && list.length > 1 ? list : [];
}

export const useEditions = (albumId: string | null | undefined): Edition[] => {
  const index = useFileTags((s) => s.index);
  return albumId ? editionsOf(index, albumId) : [];
};

/** Load the index and the scan status, and keep them current. */
export function connectFileTags(b: Backend = backend): () => void {
  let alive = true;
  let seq = 0;
  const load = () => {
    const mine = ++seq;
    b.tagsIndex()
      .then((ix) => alive && mine === seq && useFileTags.getState().setIndex(ix))
      .catch(() => {});
  };
  load();
  b.tagScanStatus()
    .then((s) => alive && useFileTags.getState().setScan(s))
    .catch(() => {});
  const unsubs: Array<() => void> = [];
  const keep = (p: Promise<() => void>) => p.then((u) => (alive ? unsubs.push(u) : u()));
  keep(b.listen("tags://changed", load));
  keep(b.listen("library://changed", load));
  keep(b.listen("tags://progress", (s) => useFileTags.getState().setScan(s)));
  return () => {
    alive = false;
    unsubs.forEach((u) => u());
  };
}
