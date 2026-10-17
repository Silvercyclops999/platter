/**
 * Hidden songs and artists, bookmarks, smart playlists (and the editor's
 * open state), the AI provider and scrobbling, as the UI sees them. Each
 * list follows its core event.
 */
import { create } from "zustand";
import { backend, type Backend } from "../lib/backend";
import { formatDuration } from "../lib/format";
import type {
  AiStatus,
  AppError,
  Bookmark,
  BookmarkPage,
  HiddenItem,
  HideRequest,
  NewBookmark,
  ScrobbleStatus,
  SmartPage,
  SmartRules,
  SmartSummary,
  Track,
} from "../lib/types";
import { nav, useRouter, type Route } from "./router";
import { usePlayer } from "./player";
import { useSettings } from "./settings";
import { toast, useToasts } from "./toasts";
import type { MenuItem } from "./ui";
import { failText } from "../lib/errorText";

/** The smart playlist editor, while it's open. */
export interface SmartEditorState {
  /** Editing this playlist (Platter `smart:` id or a Plex id); null makes a new one. */
  id: string | null;
  title: string;
  rules: SmartRules;
  /** Songs a prompt picked, best first (Platter playlists keep them first). */
  picks: Track[];
  /** The prompt box shows (an AI provider is set). */
  prompt: boolean;
  promptText: string;
  /** The describe box shows: typed words become rules on this computer. */
  describe?: boolean;
}

interface CurateState {
  hidden: HiddenItem[];
  hiddenTracks: ReadonlySet<string>;
  hiddenArtists: ReadonlySet<string>;
  bookmarks: Bookmark[];
  smart: SmartSummary[];
  ai: AiStatus | null;
  scrobble: ScrobbleStatus | null;
  editor: SmartEditorState | null;
}

export const EMPTY_RULES: SmartRules = { matchAll: true, rules: [], limit: null, sort: "none", update: "live" };

export const useCurate = create<CurateState>()(() => ({
  hidden: [],
  hiddenTracks: new Set(),
  hiddenArtists: new Set(),
  bookmarks: [],
  smart: [],
  ai: null,
  scrobble: null,
  editor: null,
}));

const errText = (e: unknown) => (e && typeof e === "object" && "message" in e ? String((e as AppError).message) : "");

function takeHidden(all: HiddenItem[]) {
  useCurate.setState({
    hidden: all,
    hiddenTracks: new Set(all.filter((h) => h.kind === "track").map((h) => h.id)),
    hiddenArtists: new Set(all.filter((h) => h.kind === "artist").map((h) => h.id)),
  });
}

let started: Backend | null = null;
let unsubs: Array<() => void> = [];

/** Load and follow everything (once, at startup). */
export async function startCurate(b: Backend = backend): Promise<void> {
  if (started === b) return;
  started = b;
  unsubs.push(await b.listen("hidden://changed", takeHidden));
  unsubs.push(await b.listen("bookmarks://changed", (all) => useCurate.setState({ bookmarks: all })));
  unsubs.push(await b.listen("smart://changed", (all) => useCurate.setState({ smart: all })));
  unsubs.push(await b.listen("scrobble://status", () => void loadScrobble(b)));
  await Promise.all([
    b.hiddenList().then(takeHidden, () => {}),
    b.bookmarksList().then((all) => useCurate.setState({ bookmarks: all }), () => {}),
    b.smartList().then((all) => useCurate.setState({ smart: all }), () => {}),
    loadAi(b),
  ]);
}

/** For tests: forget the subscriptions and state. */
export function resetCurate() {
  unsubs.forEach((u) => u());
  unsubs = [];
  started = null;
  useCurate.setState({ hidden: [], hiddenTracks: new Set(), hiddenArtists: new Set(), bookmarks: [], smart: [], ai: null, scrobble: null, editor: null });
}

export async function loadAi(b: Backend = backend): Promise<void> {
  try {
    useCurate.setState({ ai: await b.aiStatus() });
  } catch {
    /* stays unknown: the prompt box stays hidden */
  }
}

export async function loadScrobble(b: Backend = backend): Promise<void> {
  try {
    useCurate.setState({ scrobble: await b.scrobbleStatus() });
  } catch {
    /* Settings shows nothing new */
  }
}

// ---------------------------------------------------------------- hide

export const isHiddenTrack = (t: Pick<Track, "id">) => useCurate.getState().hiddenTracks.has(t.id);
export const isHiddenArtist = (id: string) => useCurate.getState().hiddenArtists.has(id);

/** Hide or unhide, with a toast that can undo. */
export async function setHidden(item: HideRequest, on: boolean, b: Backend = backend): Promise<void> {
  try {
    takeHidden(await b.hiddenSet(item, on));
    if (!on) {
      toast.info(item.kind === "artist" ? `${item.title} can play in mixes again` : "Unhidden");
      return;
    }
    const message =
      item.kind === "artist"
        ? `Mixes, radio and shuffle won't play ${item.title}`
        : "Hidden. Mixes, radio and shuffle will skip this song.";
    useToasts.getState().push({ kind: "info", message, action: { label: "Undo", run: () => void setHidden(item, false, b) } }, 6000);
  } catch (e) {
    toast.error(errText(e) || "Couldn't change that");
  }
}

/** "Hide song" for context lists like mixes and radio. */
export function hideSongItem(t: Track): MenuItem {
  return {
    get label() {
      return isHiddenTrack(t) ? "Unhide song" : "Hide song";
    },
    onSelect: () => void setHidden({ kind: "track", id: t.id, title: t.title, subtitle: t.trackArtist || t.artistTitle, thumb: t.thumb }, !isHiddenTrack(t)),
  };
}

/** "Don't play this artist" (artist and song menus). */
export function dontPlayArtistItem(a: { id: string; title: string; thumb?: string | null }): MenuItem {
  return {
    get label() {
      return isHiddenArtist(a.id) ? "Allow this artist" : "Don't play this artist";
    },
    onSelect: () => void setHidden({ kind: "artist", id: a.id, title: a.title, thumb: a.thumb ?? null }, !isHiddenArtist(a.id)),
  };
}

// ---------------------------------------------------------------- bookmarks

/** Bookmark a song: the playing one at the current time, any other from its start. */
export async function bookmarkTrack(t: Track, b: Backend = backend): Promise<void> {
  const p = usePlayer.getState();
  const at = p.state?.track?.id === t.id ? Math.floor(p.positionMs) : 0;
  try {
    await b.bookmarkAdd({ kind: "track", target: t.id, title: t.title, subtitle: t.trackArtist || t.artistTitle, thumb: t.thumb, positionMs: at });
    toast.info(at > 0 ? `Bookmarked at ${formatDuration(at)}` : "Bookmarked", { path: t.thumb, kind: "album" });
  } catch (e) {
    toast.error(errText(e) || "Couldn't add the bookmark");
  }
}

/** Bookmark the playing song at the current time (the shortcut). */
export function bookmarkNowPlaying(): void {
  const t = usePlayer.getState().state?.track;
  if (t) void bookmarkTrack(t);
  else toast.info("Play something to bookmark it");
}

export async function bookmarkPage(page: BookmarkPage, target: string, title: string, extra: { subtitle?: string | null; thumb?: string | null } = {}, b: Backend = backend): Promise<void> {
  try {
    await b.bookmarkAdd({ kind: "page", page, target, title, subtitle: extra.subtitle ?? null, thumb: extra.thumb ?? null });
    toast.info(`Bookmarked ${title}`);
  } catch (e) {
    toast.error(errText(e) || "Couldn't add the bookmark");
  }
}

export function bookmarkTrackItem(t: Track): MenuItem {
  return { label: "Bookmark", onSelect: () => void bookmarkTrack(t) };
}

export function bookmarkPageItem(page: BookmarkPage, target: string, title: string, extra: { subtitle?: string | null; thumb?: string | null } = {}): MenuItem {
  return { label: "Bookmark", onSelect: () => void bookmarkPage(page, target, title, extra) };
}

/** Where a page bookmark goes. */
export function bookmarkRoute(bm: Bookmark): Route | null {
  switch (bm.page) {
    case "album":
    case "artist":
      return { name: bm.page, id: bm.target };
    case "playlist":
    case "smart":
      return { name: "playlist", id: bm.target };
    case "mix":
      return { name: "mix", id: bm.target };
    case "radio":
      return { name: "radio", seed: bm.target };
    case "liked":
      return { name: "liked" };
    case "discography":
      return { name: "discography", id: bm.target };
    default:
      return null;
  }
}

/** Open a bookmark: a song plays from its time, a page opens. */
export async function openBookmark(bm: Bookmark, b: Backend = backend): Promise<void> {
  if (bm.kind === "page") {
    const r = bookmarkRoute(bm);
    if (r) nav.go(r);
    return;
  }
  try {
    await b.bookmarkOpen(bm.id);
  } catch (e) {
    toast.error(errText(e) || "Couldn't play that bookmark");
  }
}

export async function removeBookmark(id: string, b: Backend = backend): Promise<void> {
  const was = useCurate.getState().bookmarks?.find((x) => x.id === id);
  try {
    useCurate.setState({ bookmarks: await b.bookmarkRemove(id) });
    if (!was) return;
    const { id: _id, createdAt: _at, ...again } = was;
    toast.undo(`Removed the bookmark for ${was.title}`, () => void restoreBookmark(again, b));
  } catch (e) {
    toast.error(failText(was ? `Couldn't remove the bookmark for ${was.title}` : "Couldn't remove the bookmark", e));
  }
}

async function restoreBookmark(bm: NewBookmark, b: Backend): Promise<void> {
  try {
    await b.bookmarkAdd(bm);
    useCurate.setState({ bookmarks: await b.bookmarksList() });
  } catch (e) {
    toast.error(failText(`Couldn't put back the bookmark for ${bm.title}`, e));
  }
}

// ---------------------------------------------------------------- smart playlists

export const isPlatterSmart = (id: string) => id.startsWith("smart:");

/** "Describe a playlist" shows (its setting, on by default). */
export const describeOn = () => useSettings.getState().settings?.showDescribePlaylist !== false;

/** Open the editor for a new smart playlist (or a prompted or described one). */
export function newSmartPlaylist(opts: { prompt?: boolean; describe?: boolean } = {}): void {
  const ai = useCurate.getState().ai;
  const describe = !!opts.describe && describeOn();
  useCurate.setState({
    editor: {
      id: null,
      title: "",
      rules: { ...EMPTY_RULES, rules: describe ? [] : [{ field: "genre", op: "is", value: "" }] },
      picks: [],
      prompt: !!opts.prompt && !!ai?.ready,
      promptText: "",
      describe,
    },
  });
}

/** Open the editor on an existing smart playlist. A Plex one opens when its rules map to the editor. */
export async function editSmartPlaylist(id: string, title: string, b: Backend = backend): Promise<void> {
  try {
    if (isPlatterSmart(id)) {
      const page = await b.smartPage(id);
      const picks = page.tracks.filter((t) => page.picks.includes(t.id));
      useCurate.setState({ editor: { id, title: page.playlist.title, rules: page.rules, picks, prompt: false, promptText: page.prompt ?? "" } });
    } else {
      const rules = await b.smartPlexRules(id);
      useCurate.setState({ editor: { id, title, rules, picks: [], prompt: false, promptText: "" } });
    }
  } catch (e) {
    toast.error(errText(e) || "Couldn't open this smart playlist's rules");
  }
}

export function closeSmartEditor() {
  useCurate.setState({ editor: null });
}

export async function deleteSmartPlaylist(id: string, b: Backend = backend): Promise<void> {
  // Kept so Undo can make it again from its rules.
  const page = await b.smartPage(id).catch(() => null);
  const name = page?.playlist.title ?? "the smart playlist";
  try {
    await b.smartDelete(id);
    const r = useRouter.getState().entries[useRouter.getState().index]?.route;
    if (r?.name === "playlist" && r.id === id) nav.back();
    if (!page) {
      toast.info(`Deleted ${name}`);
      return;
    }
    toast.undo(`Deleted ${name}`, () => void restoreSmart(page, isPlatterSmart(id), b));
  } catch (e) {
    toast.error(failText(`Couldn't delete ${name}`, e));
  }
}

async function restoreSmart(page: SmartPage, platter: boolean, b: Backend): Promise<void> {
  try {
    const saved = await b.smartSave({ title: page.playlist.title, summary: page.playlist.summary, rules: page.rules, picks: page.picks, prompt: page.prompt, platter });
    toast.info(`${page.playlist.title} is back`);
    nav.go({ name: "playlist", id: saved.id });
  } catch (e) {
    toast.error(failText(`Couldn't bring back ${page.playlist.title}`, e));
  }
}

/** The create menu's smart playlist items (a prompted one when an AI provider is set). */
export function smartCreateItems(): MenuItem[] {
  const items: MenuItem[] = [{ label: "Smart playlist", onSelect: () => newSmartPlaylist() }];
  if (describeOn()) items.push({ label: "Describe a playlist", onSelect: () => newSmartPlaylist({ describe: true }) });
  if (useCurate.getState().ai?.ready) items.push({ label: "Prompted playlist", onSelect: () => newSmartPlaylist({ prompt: true }) });
  return items;
}
