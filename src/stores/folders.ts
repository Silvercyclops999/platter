/**
 * Library folders, kept by the core (`folders_get` and friends) so they
 * travel in the sync file when it's on. Plex has no folders: these only
 * group playlists in Your Library.
 *
 * Opening a folder drills the sidebar into it (`drill`); folders inside it
 * expand in place (`expanded`). Deleting a folder moves what was in it up a
 * level; playlists are never deleted with a folder.
 */
import { create } from "zustand";
import { backend, type Backend } from "../lib/backend";
import type { LibraryFolder, LibraryFolders, SyncStoreChanged } from "../lib/types";
import type { LibEntry } from "./library";
import { loadPins, usePins } from "./pins";
import { toast } from "./toasts";
import { failText } from "../lib/errorText";

const EMPTY: LibraryFolders = { folders: [], items: {} };

interface FolderState {
  tree: LibraryFolders;
  /** The folder the sidebar is drilled into, or null for the whole library. */
  drill: string | null;
  /** Folders open in place inside the drilled-in list. */
  expanded: string[];
  /** The folder whose name is being edited. */
  renaming: string | null;
  /** The folder waiting for a delete confirmation. */
  deleting: string | null;
}

export const useFolders = create<FolderState>()(() => ({ tree: EMPTY, drill: null, expanded: [], renaming: null, deleting: null }));


export async function loadFolders(b: Backend = backend): Promise<void> {
  try {
    const tree = await b.foldersGet();
    const drill = useFolders.getState().drill;
    useFolders.setState({ tree, drill: drill && tree.folders.some((f) => f.id === drill) ? drill : null });
  } catch {
    /* keep what's shown */
  }
}

/** Pins and folders after a change here or from another device. */
export function onStoreChanged(c: SyncStoreChanged, b: Backend = backend) {
  if (c.namespaces.includes("folders") || c.namespaces.includes("folderItems")) void loadFolders(b);
  if (c.remote && c.namespaces.includes("pins")) void loadPins(b);
}

async function run<T>(what: string, f: () => Promise<T>): Promise<T | null> {
  try {
    const out = await f();
    await loadFolders();
    return out;
  } catch (e) {
    toast.error(failText(`${what}`, e));
    return null;
  }
}

export const folders = {
  /** A new folder, named "New Folder", at the top or inside `parent`. */
  create: (parent: string | null = useFolders.getState().drill) =>
    run<LibraryFolder>("Couldn't create the folder", () => backend.folderCreate(null, parent)),
  rename: (id: string, name: string) => run("Couldn't rename the folder", () => backend.folderRename(id, name)),
  remove: async (id: string) => {
    const s = useFolders.getState();
    const gone = s.tree.folders.find((f) => f.id === id);
    const ok = await run("Couldn't delete the folder", () => backend.folderDelete(id).then(() => true));
    // A deleted folder doesn't keep its pin.
    if (ok && gone && usePins.getState().isPinned("folder", id)) usePins.getState().toggle({ kind: "folder", id, title: gone.name, subtitle: "Folder", thumb: null });
    // Back to the top when the open folder (or one above it) went.
    if (ok && (s.drill === id || (s.drill && ancestors(s.tree, s.drill).includes(id)))) useFolders.setState({ drill: gone?.parent ?? null });
  },
  move: (id: string, parent: string | null) => run("Couldn't move the folder", () => backend.folderMove(id, parent)),
  /** Put a playlist in a folder (null: back to the top of the library). */
  place: async (playlistId: string, folderId: string | null, title?: string) => {
    const done = await run("Couldn't move the playlist", () => backend.playlistFolderSet(playlistId, folderId).then(() => true));
    if (done && title) {
      const name = folderId ? useFolders.getState().tree.folders.find((f) => f.id === folderId)?.name : null;
      toast.info(name ? `Moved to ${name}` : "Moved to Your Library");
    }
  },
  open: (id: string) => useFolders.setState({ drill: id, expanded: [] }),
  back: () => {
    const { drill, tree } = useFolders.getState();
    useFolders.setState({ drill: tree.folders.find((f) => f.id === drill)?.parent ?? null, expanded: [] });
  },
  top: () => useFolders.setState({ drill: null, expanded: [] }),
  toggleExpanded: (id: string) => {
    const ex = useFolders.getState().expanded;
    useFolders.setState({ expanded: ex.includes(id) ? ex.filter((x) => x !== id) : [...ex, id] });
  },
  startRename: (id: string) => useFolders.setState({ renaming: id }),
  askDelete: (id: string) => useFolders.setState({ deleting: id }),
};

/** The folders above `id`, nearest first. */
export function ancestors(tree: LibraryFolders, id: string): string[] {
  const out: string[] = [];
  let at = tree.folders.find((f) => f.id === id)?.parent ?? null;
  while (at && !out.includes(at)) {
    out.push(at);
    at = tree.folders.find((f) => f.id === at)?.parent ?? null;
  }
  return out;
}

/** "1 playlist", "3 playlists". */
export const playlistCount = (n: number) => (n === 1 ? "1 playlist" : `${n} playlists`);

/** A folder as a library row. */
export function folderEntry(f: LibraryFolder, count: number, depth: number, expanded: boolean, hasChildren: boolean, nested: boolean): LibEntry {
  return {
    kind: "playlist",
    id: f.id,
    title: f.name,
    sortTitle: f.name,
    creator: null,
    creatorId: null,
    thumb: null,
    smart: false,
    format: null,
    addedAt: Math.floor(f.createdAt / 1000),
    playedAt: null,
    pinned: false,
    folder: { id: f.id, count, depth, expanded, hasChildren, nested },
  };
}

export interface FolderView {
  /** Group by folders (no search, and playlists can show). */
  active: boolean;
  drill: string | null;
  expanded: string[];
  /** The sidebar pins, newest first: pinned folders sit among them. */
  pins?: { kind: string; id: string }[];
}

/**
 * The rows with folders: at the top level, pins first, then the top-level
 * folders (newest first), then everything not in a folder. Drilled into a
 * folder: its folders (open ones followed by what's inside, indented),
 * then its playlists. `entries` is already sorted and filtered.
 */
export function withFolders(entries: LibEntry[], tree: LibraryFolders, view: FolderView): LibEntry[] {
  if (!view.active) return entries;
  const known = new Set(tree.folders.map((f) => f.id));
  const playlists = entries.filter((e) => e.kind === "playlist" && !e.liked && !e.folder);
  const inFolder = (folderId: string) => playlists.filter((e) => tree.items[e.id] === folderId && !e.unavailable);
  const children = (parent: string | null) => tree.folders.filter((f) => (f.parent && known.has(f.parent) ? f.parent : null) === parent);
  const contents = (folderId: string, depth: number): LibEntry[] => [
    ...children(folderId).flatMap((f) => {
      const open = view.expanded.includes(f.id);
      const row = folderEntry(f, inFolder(f.id).length, depth, open, children(f.id).length + inFolder(f.id).length > 0, true);
      return [row, ...(open ? contents(f.id, depth + 1) : [])];
    }),
    ...inFolder(folderId).map((e) => (depth > 0 ? { ...e, depth } : e)),
  ];
  const drill = view.drill && known.has(view.drill) ? view.drill : null;
  if (drill) return contents(drill, 0);
  const placed = (e: LibEntry) => e.kind === "playlist" && !e.liked && !e.pinned && !!tree.items[e.id] && known.has(tree.items[e.id]);
  const pinOrder = (view.pins ?? []).map((p) => `${p.kind}:${p.id}`);
  const pinnedFolders = new Set((view.pins ?? []).filter((p) => p.kind === "folder" && known.has(p.id)).map((p) => p.id));
  const folderRow = (f: LibraryFolder): LibEntry => ({ ...folderEntry(f, inFolder(f.id).length, 0, false, true, false), pinned: pinnedFolders.has(f.id) });
  const rank = (e: LibEntry) => {
    const i = pinOrder.indexOf(e.folder ? `folder:${e.id}` : `${e.kind}:${e.id}`);
    return i < 0 ? pinOrder.length : i;
  };
  // Liked Songs and the other built-in rows stay first; then the pins in pin order.
  const builtIn = entries.filter((e) => e.pinned && (e.liked || e.bookmarks || e.stats || e.page));
  const pinned = [...entries.filter((e) => e.pinned && !(e.liked || e.bookmarks || e.stats || e.page)), ...tree.folders.filter((f) => pinnedFolders.has(f.id)).map(folderRow)]
    .map((e, i) => ({ e, i }))
    .sort((a, b) => rank(a.e) - rank(b.e) || a.i - b.i)
    .map(({ e }) => e);
  const top = children(null)
    .filter((f) => !pinnedFolders.has(f.id))
    .map(folderRow);
  const rest = entries.filter((e) => !e.pinned && !placed(e));
  return [...builtIn, ...pinned, ...top, ...rest];
}
