/**
 * Dragging songs onto playlists, Liked Songs and the queue. A drag carries
 * its songs here (HTML drag data can't be read until the drop): track rows
 * and the player bar know their tracks, album and playlist cards and library
 * rows load theirs on drop. Targets read the drag from this store to light
 * up or dim for its whole length.
 */
import type { DragEvent } from "react";
import { create } from "zustand";
import { backend } from "../lib/backend";
import type { Playlist, Track } from "../lib/types";
import { useApp } from "./app";
import { isLiked, setLiked } from "./likes";
import { player } from "./player";
import { addToPlaylist } from "./playlists";
import { useSettings } from "./settings";
import { toast } from "./toasts";

/** The drag data type for songs (anything else, like files, is ignored). */
export const SONGS_TYPE = "application/x-platter-songs";

export interface DragSongs {
  /** "3 songs", or the one song's or the album's title. */
  label: string;
  /** Title shown in the drag picture. */
  title: string;
  /** Songs in the drag, when known before the drop. */
  count: number | null;
  /** The songs, when known now (rows, the player bar). */
  tracks: Track[] | null;
  /** Load the songs (albums and playlists load on drop). */
  load: () => Promise<Track[]>;
  /** The playlist being dragged: it can't take its own songs. */
  playlistId?: string;
}

interface DragState {
  songs: DragSongs | null;
  /** The target under the pointer, as a key ("playlist:12", "liked", "queue"). */
  over: string | null;
  /** Live-region text (what's dragging, the target, the result). */
  message: string;
  setOver: (key: string | null, name?: string) => void;
}

export const useDrag = create<DragState>()((set, get) => ({
  songs: null,
  over: null,
  message: "",
  setOver: (key, name) => {
    if (get().over === key) return;
    set({ over: key, ...(key && name ? { message: `Drop on ${name}` } : null) });
  },
}));

const songsText = (n: number) => (n === 1 ? "1 song" : `${n} songs`);

/** Whether dragging songs onto targets is on (Settings). */
export const dragEnabled = () => useSettings.getState().settings?.dragAndDrop !== false;

/** Songs a list of tracks carries: "3 songs" with the first title. */
export function tracksDrag(tracks: Track[]): DragSongs {
  return {
    label: tracks.length === 1 ? tracks[0].title : songsText(tracks.length),
    title: tracks[0]?.title ?? "",
    count: tracks.length,
    tracks,
    load: () => Promise.resolve(tracks),
  };
}

/** An album's songs, loaded on drop. */
export function albumDrag(albumId: string, title: string): DragSongs {
  return { label: title, title, count: null, tracks: null, load: () => backend.albumPage(albumId).then((p) => p.tracks) };
}

/** A playlist's songs, loaded on drop. */
export function playlistDrag(playlistId: string, title: string, count: number | null = null): DragSongs {
  return { label: title, title, count, tracks: null, playlistId, load: () => backend.playlistPage(playlistId).then((p) => p.tracks) };
}

/** The drag picture: a count badge and the first title. */
function dragImage(songs: DragSongs): HTMLElement {
  const el = document.createElement("div");
  el.className = "drag-ghost";
  if (songs.count !== null && songs.count > 1) {
    const badge = document.createElement("span");
    badge.className = "drag-ghost__count";
    badge.textContent = String(songs.count);
    el.appendChild(badge);
  }
  const text = document.createElement("span");
  text.className = "drag-ghost__title";
  text.textContent = songs.title;
  el.appendChild(text);
  document.body.appendChild(el);
  return el;
}

/**
 * Start carrying songs. Returns false (and carries nothing) while the
 * setting is off; the caller's own drag (a reorder) goes on regardless.
 */
export function startSongDrag(e: DragEvent, songs: DragSongs): boolean {
  if (!dragEnabled()) return false;
  const dt = e.dataTransfer;
  dt.setData(SONGS_TYPE, songs.label);
  dt.setData("text/plain", songs.label);
  dt.effectAllowed = "copyMove";
  if (typeof dt.setDragImage === "function") {
    const ghost = dragImage(songs);
    dt.setDragImage(ghost, -12, -8);
    // The browser takes its picture now; the element can go next frame.
    setTimeout(() => ghost.remove(), 0);
  }
  useDrag.setState({ songs, over: null, message: `Dragging ${songs.count !== null && songs.count !== 1 ? songsText(songs.count) : songs.label}` });
  document.documentElement.classList.add("is-dragging-songs");
  return true;
}

/** The drag ended (dropped, or Escape / released somewhere else). */
export function endSongDrag() {
  if (!useDrag.getState().songs) return;
  useDrag.setState({ songs: null, over: null });
  document.documentElement.classList.remove("is-dragging-songs");
}

/**
 * A dragleave that really leaves `currentTarget` (not just moves between its
 * children). Some engines leave out relatedTarget, so the pointer decides then.
 */
export function dragLeft(e: DragEvent): boolean {
  const to = e.relatedTarget as Node | null;
  if (to) return !e.currentTarget.contains(to);
  const r = e.currentTarget.getBoundingClientRect();
  if (r.width === 0 && r.height === 0) return true;
  return e.clientX <= r.left || e.clientX >= r.right || e.clientY <= r.top || e.clientY >= r.bottom;
}

/** This drag event carries songs from inside the app. */
export function isSongDrag(e: DragEvent): boolean {
  return !!useDrag.getState().songs && Array.from(e.dataTransfer?.types ?? []).includes(SONGS_TYPE);
}

// ---------------------------------------------------------------- targets

export type DropRule = "ok" | "dim";

/** Whether a library row takes the songs: your playlists do, Liked does unless they're all liked. */
export function libraryDropRule(
  entry: { kind: string; id: string; smart: boolean; liked?: boolean; unavailable?: boolean; bookmarks?: boolean },
  songs: DragSongs,
): DropRule {
  if (entry.unavailable || entry.bookmarks) return "dim";
  if (entry.liked) return songs.tracks && songs.tracks.length > 0 && songs.tracks.every((t) => isLiked(t.id)) ? "dim" : "ok";
  if (entry.kind !== "playlist" || entry.smart || entry.id === songs.playlistId) return "dim";
  return "ok";
}

/**
 * A drop target's state and handlers: `over` while songs it takes are over
 * it, `dimmed` for the whole of a drag it refuses.
 */
export function useSongDrop(key: string, name: string, rule: (songs: DragSongs) => DropRule, drop: (songs: DragSongs) => unknown) {
  const songs = useDrag((s) => s.songs);
  const over = useDrag((s) => s.over === key);
  const state = songs ? rule(songs) : null;
  const onOver = (e: DragEvent) => {
    const s = useDrag.getState().songs;
    if (!s || !isSongDrag(e) || rule(s) !== "ok") return;
    e.preventDefault();
    e.stopPropagation();
    e.dataTransfer.dropEffect = "copy";
    useDrag.getState().setOver(key, name);
  };
  return {
    over: over && state === "ok",
    dimmed: state === "dim",
    handlers: {
      onDragEnter: onOver,
      onDragOver: onOver,
      onDragLeave: (e: DragEvent) => {
        if (!dragLeft(e)) return;
        if (useDrag.getState().over === key) useDrag.getState().setOver(null);
      },
      onDrop: (e: DragEvent) => {
        const s = useDrag.getState().songs;
        if (!s || !isSongDrag(e) || rule(s) !== "ok") return;
        e.preventDefault();
        e.stopPropagation();
        endSongDrag();
        void drop(s);
      },
    },
  };
}

/** Read the result out loud (the live region). */
function say(message: string) {
  useDrag.setState({ message });
}

function failed(e: unknown) {
  const m = e && typeof e === "object" && "message" in e ? String((e as { message: unknown }).message) : "";
  toast.error(`Couldn't load those songs${m ? `: ${m}` : ""}`);
}

/** Append to a playlist: the usual toast, or the "already added" dialog. */
export async function dropOnPlaylist(playlistId: string, songs: DragSongs): Promise<void> {
  const playlist: Playlist | undefined = useApp.getState().playlists?.find((p) => p.id === playlistId);
  if (!playlist || playlist.smart || playlistId === songs.playlistId) return;
  let tracks: Track[];
  try {
    tracks = await songs.load();
  } catch (e) {
    failed(e);
    return;
  }
  if (!tracks.length) return;
  await addToPlaylist(playlist, tracks.map((t) => t.id));
  say(`Added ${songsText(tracks.length)} to ${playlist.title}`);
}

/** Like the songs that aren't liked yet. Liked ones keep their date: never a re-like. */
export async function dropOnLiked(songs: DragSongs): Promise<void> {
  let tracks: Track[];
  try {
    tracks = await songs.load();
  } catch (e) {
    failed(e);
    return;
  }
  const fresh = tracks.filter((t) => !isLiked(t.id));
  if (!fresh.length) {
    if (tracks.length) toast.info("Already in Liked Songs");
    return;
  }
  if (await setLiked(fresh, true)) say(`Added ${songsText(fresh.length)} to Liked Songs`);
}

/** Into the queue: at a spot in "Next in queue", or at its end. */
export async function dropOnQueue(songs: DragSongs, at: number | "end"): Promise<void> {
  let tracks: Track[];
  try {
    tracks = await songs.load();
  } catch (e) {
    failed(e);
    return;
  }
  if (!tracks.length) return;
  await player.queueAdd(
    tracks.map((t) => t.id),
    at === "end" ? "end" : { index: at },
  );
  if (at === "end") toast.info(tracks.length === 1 ? "Added to queue" : `Added ${songsText(tracks.length)} to queue`, tracks.length === 1 ? { path: tracks[0].thumb } : undefined);
  say(`Added ${songsText(tracks.length)} to the queue`);
}

// ---------------------------------------------------------------- auto-scroll

/** How near a list's top or bottom edge (px) the pointer starts scrolling it. */
export const SCROLL_EDGE = 48;
/** Fastest scroll per drag event (px). */
export const SCROLL_MAX = 18;

/**
 * Scroll step for a pointer at `y` over a scroller spanning `top..bottom`:
 * negative near the top, positive near the bottom, faster nearer the edge.
 */
export function autoScrollStep(top: number, bottom: number, y: number): number {
  if (y < top || y > bottom) return 0;
  const edge = Math.min(SCROLL_EDGE, (bottom - top) / 3);
  if (y < top + edge) return -Math.ceil(((top + edge - y) / edge) * SCROLL_MAX);
  if (y > bottom - edge) return Math.ceil(((y - (bottom - edge)) / edge) * SCROLL_MAX);
  return 0;
}

const SCROLLERS = ".main__scroll, .lib-scroll, .queue-panel__body";

/** While any drag is over a list, scroll it near its edges. */
export function connectDragAutoScroll(): () => void {
  const onOver = (e: globalThis.DragEvent) => {
    const el = (e.target as Element | null)?.closest?.<HTMLElement>(SCROLLERS);
    if (!el) return;
    const r = el.getBoundingClientRect();
    const step = autoScrollStep(r.top, r.bottom, e.clientY);
    if (step) el.scrollTop += step;
  };
  // Escape or a release outside the window never reaches the source's dragend in some cases.
  const onEnd = () => endSongDrag();
  document.addEventListener("dragover", onOver);
  document.addEventListener("dragend", onEnd);
  document.addEventListener("drop", onEnd);
  return () => {
    document.removeEventListener("dragover", onOver);
    document.removeEventListener("dragend", onEnd);
    document.removeEventListener("drop", onEnd);
  };
}
