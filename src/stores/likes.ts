/**
 * Likes in the UI: which songs are liked, liking and unliking (shown at
 * once, put back if the core says no), and the "saved in" menu. Every
 * change goes to the core (docs/IPC.md "Likes"), which queues the Plex
 * rating write and sends `likes://changed`.
 */
import { create } from "zustand";
import { backend, type Backend } from "../lib/backend";
import type { AppError, Track } from "../lib/types";
import { useSettings } from "./settings";
import { toast, useToasts } from "./toasts";

/** The saved-in menu: the song and where it opens. */
export interface SavedInAsk {
  track: Track;
  /** The control it opened from (the menu lines up with it). */
  anchor: { left: number; right: number; top: number; bottom: number } | null;
  /** Element that gets focus back when the menu closes. */
  returnFocus: HTMLElement | null;
}

interface LikesState {
  /** Liked song ids. */
  ids: ReadonlySet<string>;
  /** The liked ids have loaded once. */
  loaded: boolean;
  savedIn: SavedInAsk | null;
  setIds: (ids: Iterable<string>) => void;
  openSavedIn: (track: Track, at?: HTMLElement | null) => void;
  closeSavedIn: () => void;
}

export const useLikes = create<LikesState>()((set) => ({
  ids: new Set(),
  loaded: false,
  savedIn: null,
  setIds: (ids) => set({ ids: new Set(ids), loaded: true }),
  openSavedIn: (track, at) => {
    const live = at && at.isConnected ? at : null;
    const r = live?.getBoundingClientRect();
    set({
      savedIn: {
        track,
        anchor: r ? { left: r.left, right: r.right, top: r.top, bottom: r.bottom } : null,
        returnFocus: live ?? (document.activeElement instanceof HTMLElement ? document.activeElement : null),
      },
    });
  },
  closeSavedIn: () => set({ savedIn: null }),
}));

/** Whether a song is liked (re-renders when that changes). */
export const useIsLiked = (trackId: string | null | undefined) => useLikes((s) => !!trackId && s.ids.has(trackId));

export const isLiked = (trackId: string) => useLikes.getState().ids.has(trackId);

const UNDO_MS = 8000;

function errorText(e: unknown): string {
  const m = e && typeof e === "object" && "message" in e ? String((e as AppError).message) : "";
  return m || "Something went wrong";
}

const songs = (n: number) => (n === 1 ? "1 song" : `${n} songs`);

/** The last like control used, so "Change" in the toast can open the menu beside it. */
let lastControl: HTMLElement | null = null;

/** Change the liked set here at once; returns how to put it back. */
function applyLocal(ids: string[], liked: boolean): () => void {
  const before = useLikes.getState().ids;
  const next = new Set(before);
  for (const id of ids) {
    if (liked) next.add(id);
    else next.delete(id);
  }
  useLikes.setState({ ids: next });
  return () => {
    const now = new Set(useLikes.getState().ids);
    for (const id of ids) {
      if (before.has(id)) now.add(id);
      else now.delete(id);
    }
    useLikes.setState({ ids: now });
  };
}

export interface SetLikedOpts {
  /** Say so in a toast (default true). */
  toast?: boolean;
  /** The control used (anchors the saved-in menu from the toast's Change). */
  from?: HTMLElement | null;
  /** Called after an Undo from the toast. */
  onUndo?: () => void;
}

/**
 * Like or unlike songs. Shows at once; if the core refuses, it goes back
 * with a toast. One liked song gets "Change" in its toast (opens the saved-in
 * menu); unliking gets "Undo".
 */
export async function setLiked(tracks: Track[], liked: boolean, opts: SetLikedOpts = {}): Promise<boolean> {
  const ids = [...new Set(tracks.map((t) => t.id))].filter((id) => isLiked(id) !== liked);
  if (!ids.length) return true;
  if (opts.from) lastControl = opts.from;
  const undoLocal = applyLocal(ids, liked);
  try {
    await backend.likesSet(ids, liked);
  } catch (e) {
    undoLocal();
    toast.error(`${liked ? "Couldn't add to Liked Songs" : "Couldn't remove from Liked Songs"}: ${errorText(e)}`);
    return false;
  }
  if (opts.toast === false) return true;
  const changed = tracks.filter((t) => ids.includes(t.id));
  if (liked) {
    const one = changed.length === 1 ? changed[0] : null;
    useToasts.getState().push({
      kind: "info",
      message: one ? "Added to Liked Songs" : `Added ${songs(changed.length)} to Liked Songs`,
      ...(one?.thumb ? { art: { path: one.thumb } } : {}),
      action: one ? { label: "Change", run: () => useLikes.getState().openSavedIn(one, lastControl) } : undefined,
    });
  } else {
    useToasts.getState().push(
      {
        kind: "info",
        message: changed.length === 1 ? "Removed from Liked Songs" : `Removed ${songs(changed.length)} from Liked Songs`,
        ...(changed.length === 1 && changed[0].thumb ? { art: { path: changed[0].thumb } } : {}),
        action: {
          label: "Undo",
          run: () => {
            opts.onUndo?.();
            void setLiked(changed, true, { toast: false });
          },
        },
      },
      UNDO_MS,
    );
  }
  return true;
}

/** Like if any of them isn't liked yet, otherwise unlike them all. */
export function toggleLiked(tracks: Track[], opts: SetLikedOpts = {}): Promise<boolean> {
  const all = tracks.length > 0 && tracks.every((t) => isLiked(t.id));
  return setLiked(tracks, !all, opts);
}

/** "Add to Liked Songs" / "Remove from Liked Songs" for a menu. */
export function likeMenuLabel(trackIds: string[]): string {
  return trackIds.length > 0 && trackIds.every((id) => isLiked(id)) ? "Remove from Liked Songs" : "Add to Liked Songs";
}

/** Star rating text for a Plex rating (0 to 10): "3 stars", "4.5 stars". */
export function starsLabel(rating: number): string {
  const stars = Math.round(rating) / 2;
  return stars === 1 ? "1 star" : `${stars} stars`;
}

/**
 * The grey hint by Liked Songs in the saved-in menu, so a star rating is
 * never overwritten unnoticed: "Rated 3 stars in Plex" on a song that isn't
 * liked, "Was 3 stars in Plex" on one just liked over a lower rating.
 */
export function ratingHint(track: Pick<Track, "userRating">, liked: boolean, likedMin: number): string | null {
  const r = track.userRating ?? 0;
  if (r <= 0 || r >= likedMin) return null;
  return liked ? `Was ${starsLabel(r)} in Plex` : `Rated ${starsLabel(r)} in Plex`;
}

/** The lowest rating that counts as liked (Settings). */
export function likedMinRating(): number {
  return useSettings.getState().settings?.likedMeans === "fourUp" ? 8 : 10;
}

let reload: () => void = () => {};

/** Load the liked ids again (after the "liked means" setting is saved). */
export function reloadLikes() {
  reload();
}

/** Load the liked ids, and again when likes or the library change. */
export function connectLikes(b: Backend = backend): () => void {
  let alive = true;
  let seq = 0;
  const load = () => {
    const mine = ++seq;
    b.likedIds()
      .then((ids) => alive && mine === seq && useLikes.getState().setIds(ids))
      .catch(() => {});
  };
  load();
  reload = load;
  const unsubs: Array<() => void> = [];
  const keep = (p: Promise<() => void>) => p.then((u) => (alive ? unsubs.push(u) : u()));
  keep(b.listen("likes://changed", load));
  keep(b.listen("library://changed", load));
  return () => {
    alive = false;
    if (reload === load) reload = () => {};
    unsubs.forEach((u) => u());
  };
}
