/**
 * Playlist editing in the UI: create, add (with the duplicate check),
 * remove with undo, delete, duplicate, and which dialogs are open. Every
 * change goes to the core (docs/IPC.md "Playlist editing"); the core then
 * sends `library://changed` and pages refetch.
 */
import { create } from "zustand";
import { addToShared, isShared } from "./circle";
import { backend } from "../lib/backend";
import type { AppError, Playlist, RemovedItem } from "../lib/types";
import { useLibraryView } from "./library";
import { nav, useRouter } from "./router";
import { useSettings } from "./settings";
import { toast, useToasts } from "./toasts";

export interface DuplicateAsk {
  playlist: Playlist;
  trackIds: string[];
  /** The songs from `trackIds` the playlist already has. */
  duplicates: string[];
}

export interface RemoveAsk {
  playlist: Playlist;
  itemIds: string[];
  /** Every row id on the page, in order (for undo). */
  order: string[];
}

interface PlaylistUi {
  /** The playlist page shows this playlist's name as a field with focus (just created). */
  renameId: string | null;
  /** Edit details dialog for this playlist. */
  editing: Playlist | null;
  /** Delete confirmation for this playlist. */
  deleting: Playlist | null;
  /** "Already added" dialog. */
  duplicate: DuplicateAsk | null;
  /** "Remove N songs?" confirmation (Delete key). */
  removing: RemoveAsk | null;
  set: (p: Partial<Omit<PlaylistUi, "set">>) => void;
}

export const usePlaylistUi = create<PlaylistUi>()((set) => ({
  renameId: null,
  editing: null,
  deleting: null,
  duplicate: null,
  removing: null,
  set: (p) => set(p),
}));

const UNDO_MS = 8000;

function errorText(e: unknown): string {
  const m = e && typeof e === "object" && "message" in e ? String((e as AppError).message) : "";
  return m || "Something went wrong";
}

const songs = (n: number) => (n === 1 ? "1 song" : `${n} songs`);

/**
 * Make a playlist. Without songs it opens with the name ready to type over;
 * with songs (from "Add to playlist") it stays where you are and says so.
 */
export async function createPlaylist(opts: { trackIds?: string[]; title?: string; open?: boolean } = {}): Promise<Playlist | null> {
  const ids = opts.trackIds ?? [];
  try {
    const p = await backend.playlistCreate(opts.title, ids);
    useLibraryView.getState().touch("playlist", p.id);
    if (opts.open ?? ids.length === 0) {
      usePlaylistUi.getState().set({ renameId: p.id });
      nav.go({ name: "playlist", id: p.id });
    }
    if (ids.length) toast.info(`Added to ${p.title}`, { path: p.thumb, kind: "playlist" });
    return p;
  } catch (e) {
    toast.error(`Couldn't create the playlist: ${errorText(e)}`);
    return null;
  }
}

/** Add songs at the end. Asks first when some are already there (Settings). */
export async function addToPlaylist(playlist: Playlist, trackIds: string[]): Promise<void> {
  if (!trackIds.length) return;
  // Someone else's playlist: the songs go to its owner.
  if (isShared(playlist.id)) return addToShared(playlist, trackIds);
  const ask = useSettings.getState().settings?.playlistAskDuplicates ?? true;
  if (ask) {
    try {
      const duplicates = await backend.playlistDuplicates(playlist.id, trackIds);
      if (duplicates.length) {
        usePlaylistUi.getState().set({ duplicate: { playlist, trackIds, duplicates } });
        return;
      }
    } catch {
      /* the add below says what went wrong */
    }
  }
  await addNow(playlist, trackIds);
}

/** Add without asking. */
export async function addNow(playlist: Pick<Playlist, "id" | "title" | "thumb">, trackIds: string[]): Promise<boolean> {
  if (!trackIds.length) return false;
  try {
    const p = await backend.playlistAdd(playlist.id, trackIds);
    useLibraryView.getState().touch("playlist", p.id);
    toast.info(`Added to ${p.title}`, { path: p.thumb ?? playlist.thumb, kind: "playlist" });
    return true;
  } catch (e) {
    toast.error(`Couldn't add to ${playlist.title}: ${errorText(e)}`);
    return false;
  }
}

/** The "Already added" dialog's answer. */
export function answerDuplicate(choice: "all" | "new" | "none") {
  const ask = usePlaylistUi.getState().duplicate;
  usePlaylistUi.getState().set({ duplicate: null });
  if (!ask || choice === "none") return;
  const dup = new Set(ask.duplicates);
  const ids = choice === "all" ? ask.trackIds : ask.trackIds.filter((t) => !dup.has(t));
  void addNow(ask.playlist, ids);
}

/**
 * Put removed rows back where they were: each run of neighbouring rows goes
 * back after the row that was before it (`order` is the list before removal).
 */
export async function restoreRows(playlistId: string, removed: RemovedItem[], order: string[]): Promise<void> {
  const sorted = [...removed].sort((a, b) => a.index - b.index);
  const runs: RemovedItem[][] = [];
  for (const r of sorted) {
    const last = runs[runs.length - 1];
    if (last && last[last.length - 1].index === r.index - 1) last.push(r);
    else runs.push([r]);
  }
  for (const run of runs) {
    const before = order[run[0].index - 1];
    const ids = run.map((r) => r.trackId);
    await backend.playlistAdd(playlistId, ids, before ? { after: before } : { top: true });
  }
}

/** Remove rows at once, with Undo in the toast. */
export async function removeRows(playlist: Playlist, itemIds: string[], order: string[]): Promise<void> {
  if (!itemIds.length) return;
  try {
    const removed = await backend.playlistRemove(playlist.id, itemIds);
    if (!removed.length) return;
    useToasts.getState().push(
      {
        kind: "info",
        message: removed.length === 1 ? `Removed from ${playlist.title}` : `Removed ${songs(removed.length)} from ${playlist.title}`,
        action: {
          label: "Undo",
          run: () => {
            restoreRows(playlist.id, removed, order).catch((e) => toast.error(`Couldn't put them back: ${errorText(e)}`));
          },
        },
      },
      UNDO_MS,
    );
  } catch (e) {
    toast.error(`Couldn't remove from ${playlist.title}: ${errorText(e)}`);
  }
}

/** Ask before removing (the Delete key). */
export function askRemove(playlist: Playlist, itemIds: string[], order: string[]) {
  if (itemIds.length) usePlaylistUi.getState().set({ removing: { playlist, itemIds, order } });
}

export async function confirmRemove() {
  const ask = usePlaylistUi.getState().removing;
  usePlaylistUi.getState().set({ removing: null });
  if (!ask) return;
  try {
    await backend.playlistRemove(ask.playlist.id, ask.itemIds);
  } catch (e) {
    toast.error(`Couldn't remove from ${ask.playlist.title}: ${errorText(e)}`);
  }
}

export function askDelete(playlist: Playlist) {
  usePlaylistUi.getState().set({ deleting: playlist });
}

export async function confirmDelete() {
  const p = usePlaylistUi.getState().deleting;
  usePlaylistUi.getState().set({ deleting: null });
  if (!p) return;
  // Leave its page first, so the page doesn't see it vanish.
  const { entries, index } = useRouter.getState();
  const r = entries[index]?.route;
  if (r?.name === "playlist" && r.id === p.id) nav.go({ name: "home" });
  try {
    await backend.playlistDelete(p.id);
    toast.info(`Deleted ${p.title}`);
  } catch (e) {
    toast.error(`Couldn't delete ${p.title}: ${errorText(e)}`);
  }
}

/** A new, ordinary playlist with the same songs (also how a smart playlist is kept as it is now). */
export async function duplicatePlaylist(p: Playlist): Promise<void> {
  try {
    const page = await backend.playlistPage(p.id);
    const copy = await backend.playlistCreate(`${p.title} (copy)`.slice(0, 100), page.tracks.map((t) => t.id));
    useLibraryView.getState().touch("playlist", copy.id);
    toast.info(`Made ${copy.title}`);
  } catch (e) {
    toast.error(`Couldn't copy ${p.title}: ${errorText(e)}`);
  }
}

export function editDetails(p: Playlist) {
  usePlaylistUi.getState().set({ editing: p });
}

/** Rename (from the name field on a new playlist or the dialog). */
export async function savePlaylistDetails(p: Playlist, details: { title?: string; summary?: string }): Promise<boolean> {
  try {
    await backend.playlistUpdate(p.id, details);
    return true;
  } catch (e) {
    toast.error(`Couldn't save: ${errorText(e)}`);
    return false;
  }
}
