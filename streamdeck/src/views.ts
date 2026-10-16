/**
 * What each key shows for a state: its picture and its title (the title is
 * only used for a short reason when the key can't run now). Pure, so every
 * action's mapping is tested on its own.
 */
import type { LibraryItem, LibraryKind, PlatterState } from "./client.js";
import { clock, elapsed, hasSong, progress, type Snapshot } from "./live.js";
import { type Badge, coverPic, infoPic, keyPic, type KeyPic, nextPic, type Slice } from "./render.js";
import { DEFAULTS, type SettingsOf } from "./settings.js";

/** Every key action, by the last part of its UUID. */
export const KEY_ACTIONS = [
  "playpause",
  "previous",
  "next",
  "seekback",
  "seekforward",
  "songinfo",
  "shuffle",
  "repeat",
  "crossfade",
  "private",
  "volumeup",
  "volumedown",
  "mute",
  "volumeset",
  "like",
  "addtoplaylist",
  "removefromplaylist",
  "rate",
  "playplaylist",
  "playalbum",
  "playartist",
  "playmix",
  "playlink",
  "playsearch",
  "radio",
  "daylist",
  "discover",
  "likedsongs",
  "queue",
  "listenon",
  "lyrics",
  "miniplayer",
  "window",
  "sleep",
  "bookmark",
  "request",
  "listen",
] as const;
export type KeyActionId = (typeof KEY_ACTIONS)[number];

/** The library list a play key picks from. */
export const PLAY_KIND: Partial<Record<KeyActionId, LibraryKind>> = {
  playplaylist: "playlists",
  playalbum: "albums",
  playartist: "artists",
  playmix: "mixes",
};

/** The command kind for a library list. */
export const ITEM_KIND: Record<LibraryKind, "playlist" | "album" | "artist" | "mix"> = {
  playlists: "playlist",
  albums: "album",
  artists: "artist",
  mixes: "mix",
};

export interface ViewCtx {
  snap: Snapshot;
  now: number;
  /** A cover fetched by key, or null while it isn't here yet. */
  cover: (key: string | null | undefined) => string | null;
  /** An item from a library list (for play keys and playlist targets). */
  item: (kind: LibraryKind, id: string) => LibraryItem | undefined;
  /** This Play / Pause key's part of a big cover. */
  slice?: Slice | null;
}

export interface Shown {
  image: string;
  title: string;
}

/** The badge a connection state puts on keys. */
export function badgeOf(snap: Snapshot): Badge | null {
  return snap.conn === "unpaired" || snap.conn === "approve" ? "unpaired" : snap.conn === "refused" ? "refused" : null;
}

const NOT_RUNNING = "Platter\nnot running";
/** While Platter asks whether to allow this plugin. */
export const APPROVE = "Approve\nin Platter";
const NOTHING = "Nothing\nplaying";

/** Settings for a key action, defaults filled in. */
type Any = Record<string, unknown> & { images?: Record<string, string> };

/** A plain glyph key with the usual states around it. */
function plain(ctx: ViewCtx, p: KeyPic, o: { needsSong?: boolean; reason?: string | null; custom?: string | null } = {}): Shown {
  const b = badgeOf(ctx.snap);
  if (ctx.snap.conn === "offline") return { image: keyPic({ ...p, on: false, off: true, custom: o.custom }), title: NOT_RUNNING };
  const s = ctx.snap.state;
  if (o.needsSong && s && !hasSong(s) && ctx.snap.conn === "ok") return { image: keyPic({ ...p, on: false, off: true, badge: b, custom: o.custom }), title: NOTHING };
  if (o.reason && ctx.snap.conn === "ok") return { image: keyPic({ ...p, on: false, off: true, badge: b, custom: o.custom }), title: o.reason };
  return { image: keyPic({ ...p, badge: b, custom: o.custom }), title: "" };
}

const img = (st: { images?: Record<string, string> }, name: string): string | null => st.images?.[name] ?? null;

function timeText(st: SettingsOf<"playpause">, snap: Snapshot, now: number): string | null {
  const s = snap.state;
  if (st.time === "off" || !hasSong(s)) return null;
  const e = elapsed(snap, now);
  return st.time === "elapsed" ? clock(e) : `-${clock(Math.max(0, s.durationMs - e))}`;
}

/** Play / Pause: full-key cover art, no text unless asked for. */
export function playPauseView(st: SettingsOf<"playpause">, ctx: ViewCtx): Shown {
  const { snap } = ctx;
  const s = snap.state;
  if (snap.conn === "offline") return { image: keyPic({ glyph: "play", off: true, custom: img(st, "paused") }), title: NOT_RUNNING };
  const paused = !s?.playing;
  const custom = img(st, paused ? "paused" : "playing");
  const image = coverPic({
    cover: hasSong(s) ? (s.cover ?? null) : null,
    paused,
    pausedLook: st.pausedLook,
    time: timeText(st, snap, ctx.now),
    title: st.showTitle && hasSong(s) ? s.title : null,
    slice: ctx.slice ?? null,
    badge: badgeOf(snap),
    custom,
  });
  return { image, title: "" };
}

/** m:ss of the sleep timer at `now`. */
export function sleepLeft(snap: Snapshot, now: number): string | null {
  const sl = snap.state?.sleep;
  if (!sl) return null;
  if (sl.endOfSong || sl.remainingMs === null) return "Song end";
  return clock(Math.max(0, sl.remainingMs - Math.max(0, now - snap.at)) + 999);
}

/** The song info text for an output (clipboard, file). */
export function infoFields(snap: Snapshot, now: number): { title: string; artist: string; album: string; elapsed: string; total: string } {
  const s = snap.state;
  if (!hasSong(s)) return { title: "Nothing playing", artist: "-", album: "", elapsed: "0:00", total: "0:00" };
  return { title: s.title ?? "", artist: s.artist ?? "", album: s.album ?? "", elapsed: clock(elapsed(snap, now)), total: clock(s.durationMs) };
}

/** What the playing context is, to light up the key that started it. */
const playingFrom = (s: PlatterState | null, kind: string, id: string): boolean => !!s && !!id && s.context?.kind === kind && s.context.id === id;

/** Any key but Play / Pause. */
export function keyView(action: KeyActionId, raw: Any, ctx: ViewCtx): Shown {
  const shown = viewOf(action, raw, ctx);
  return ctx.snap.conn === "approve" && !shown.title ? { ...shown, title: APPROVE } : shown;
}

function viewOf(action: KeyActionId, raw: Any, ctx: ViewCtx): Shown {
  if (action === "playpause") return playPauseView(raw as unknown as SettingsOf<"playpause">, ctx);
  const st = raw;
  const { snap, now } = ctx;
  const s = snap.state;
  const song = hasSong(s);
  const custom = img(st, "normal");
  const toggle = (glyph: string, on: boolean, extra: Partial<KeyPic> = {}, o: { needsSong?: boolean; reason?: string | null } = {}) =>
    plain(ctx, { glyph, on, ...extra }, { ...o, custom: img(st, on ? "on" : "off") });
  switch (action) {
    case "previous":
    case "next": {
      const c = st as unknown as SettingsOf<"next">;
      const other = action === "next" ? s?.next : s?.previous;
      const cover = c.cover && other ? ctx.cover(other.coverKey) : null;
      return plain(ctx, { glyph: action, cover, dim: 0.45 }, { custom });
    }
    case "seekback":
    case "seekforward": {
      const sec = (st as unknown as SettingsOf<"seekback">).seconds;
      return plain(ctx, { glyph: action === "seekback" ? "seekBack" : "seekForward", label: `${sec} s` }, { needsSong: true, custom });
    }
    case "songinfo": {
      if (snap.conn === "offline") return { image: keyPic({ glyph: "info", off: true }), title: NOT_RUNNING };
      if (!song) return { image: infoPic({ cover: null, title: "Nothing playing", artist: "-", time: "0:00 / 0:00", progress: 0, badge: badgeOf(snap) }), title: "" };
      const time = `${clock(elapsed(snap, now))} / ${clock(s.durationMs)}`;
      const title = s.private ? "Private session" : (s.title ?? "");
      return { image: infoPic({ cover: s.cover ?? null, title, artist: s.artist ?? "-", time, progress: progress(snap, now), badge: badgeOf(snap) }), title: "" };
    }
    case "shuffle":
      return toggle("shuffle", !!s?.shuffle || !!s?.smartShuffle, s?.smartShuffle ? { label: "Smart" } : {});
    case "repeat":
      return toggle(s?.repeat === "one" ? "repeatOne" : "repeat", !!s && s.repeat !== "off", s && s.repeat !== "off" ? { label: s.repeat === "one" ? "One" : "All" } : {});
    case "crossfade":
      return toggle("crossfade", !!s?.crossfade);
    case "private":
      return toggle("private", !!s?.private);
    case "volumeup":
    case "volumedown": {
      const v = st as unknown as SettingsOf<"volumeup">;
      const level = s ? (s.muted ? 0 : s.volume) : null;
      const extra: Partial<KeyPic> = level === null ? {} : v.show === "bar" ? { bar: level / 100 } : v.show === "number" ? { label: `${level}` } : {};
      return plain(ctx, { glyph: action === "volumeup" ? "volumeUp" : "volumeDown", ...extra }, { custom });
    }
    case "mute":
      return toggle(s?.muted ? "mute" : "volume", !!s?.muted);
    case "volumeset": {
      const level = (st as unknown as SettingsOf<"volumeset">).level;
      return toggle("volumeSet", !!s && !s.muted && s.volume === level, { label: `${level}%` });
    }
    case "like":
      return plain(ctx, { glyph: s?.liked && song ? "heartFilled" : "heart", on: !!s?.liked && song }, { needsSong: true, custom: img(st, s?.liked ? "liked" : "unliked") });
    case "addtoplaylist":
    case "removefromplaylist": {
      const p = st as unknown as SettingsOf<"addtoplaylist">;
      const item = p.playlist ? ctx.item("playlists", p.playlist) : undefined;
      const name = item?.title ?? p.playlistName;
      const isIn = !!p.playlist && !!s?.playlists.includes(p.playlist);
      const cover = item ? ctx.cover(item.coverKey) : null;
      const label = p.playlist ? name || "Playlist" : "Ask";
      if (action === "addtoplaylist") {
        return plain(ctx, { glyph: isIn ? "playlist" : "addToPlaylist", on: isIn, cover, dim: 0.5, label }, { needsSong: true, custom: img(st, isIn ? "on" : "off") });
      }
      const reason = !p.playlist ? "Pick a\nplaylist" : !isIn ? "Not in\nplaylist" : null;
      return plain(ctx, { glyph: "removeFromPlaylist", cover, dim: 0.5, label }, { needsSong: true, reason, custom });
    }
    case "rate": {
      const stars = (st as unknown as SettingsOf<"rate">).stars;
      const on = song && s.stars === stars;
      const reason = song && !s.canRate ? "No star\nratings" : null;
      return plain(ctx, { glyph: on ? "starFilled" : "star", on, label: `${stars} ${stars === 1 ? "star" : "stars"}` }, { needsSong: true, reason, custom: img(st, on ? "on" : "off") });
    }
    case "playplaylist":
    case "playalbum":
    case "playartist":
    case "playmix": {
      const p = st as unknown as SettingsOf<"playplaylist">;
      const kind = PLAY_KIND[action] as LibraryKind;
      const glyphName = { playlists: "playlist", albums: "album", artists: "artist", mixes: "mix" }[kind];
      if (!p.id) return plain(ctx, { glyph: glyphName }, { reason: "Pick\none", custom });
      const item = ctx.item(kind, p.id);
      const cover = item ? ctx.cover(item.coverKey) : null;
      const ring = playingFrom(s, ITEM_KIND[kind], p.id);
      if (cover) return plain(ctx, { cover, dim: 0, ring }, { custom });
      return plain(ctx, { glyph: glyphName, label: item?.title ?? p.name, ring }, { custom });
    }
    case "playlink":
      return plain(ctx, { glyph: "link" }, { reason: (st as unknown as SettingsOf<"playlink">).link.trim() ? null : "Set a\nlink", custom });
    case "playsearch": {
      const q = (st as unknown as SettingsOf<"playsearch">).query.trim();
      return plain(ctx, { glyph: "search", label: q || undefined }, { reason: q ? null : "Type a\nsearch", custom });
    }
    case "radio":
      return plain(ctx, { glyph: "radio" }, { needsSong: true, custom });
    case "daylist":
      return plain(ctx, { glyph: "daylist" }, { custom });
    case "discover":
      return plain(ctx, { glyph: "discover" }, { custom });
    case "likedsongs":
      return plain(ctx, { glyph: "liked", ring: s?.context?.kind === "liked" }, { custom });
    case "queue": {
      if (snap.conn === "offline") return { image: keyPic({ glyph: "queue", off: true }), title: NOT_RUNNING };
      const n = s?.next ?? null;
      return { image: custom ? keyPic({ custom, badge: badgeOf(snap) }) : nextPic(n ? ctx.cover(n.coverKey) : null, n?.title ?? null, badgeOf(snap)), title: "" };
    }
    case "listenon": {
      const d = st as unknown as SettingsOf<"listenon">;
      const here = !s?.device;
      const on = !!s && (d.device === "local" ? here : !!d.device && !here && s.device === d.deviceName);
      return toggle("device", on, { label: d.device ? d.deviceName || "Device" : here ? "Listen on" : (s?.device ?? "") });
    }
    case "lyrics":
      return toggle("lyrics", !!s?.lyrics);
    case "miniplayer":
      return toggle("miniplayer", !!s?.miniplayer);
    case "window":
      return toggle("window", !!s?.window);
    case "sleep": {
      const sl = st as unknown as SettingsOf<"sleep">;
      const left = sleepLeft(snap, now);
      if (left) return toggle("sleep", true, left === "Song end" ? { label: "Song end" } : { big: left });
      return toggle("sleep", false, { label: sl.endOfSong ? "Song end" : `${sl.minutes} min` });
    }
    case "bookmark":
      return plain(ctx, { glyph: "bookmark" }, { needsSong: true, custom });
    case "request":
      return plain(ctx, { glyph: "request" }, { needsSong: true, reason: s && !s.canRequest ? "Set up\nLidarr" : null, custom });
    case "listen":
      return plain(ctx, { glyph: "listen" }, { custom });
  }
}

/** The defaults table has an entry for this action. */
export const hasSettings = (a: string): a is keyof typeof DEFAULTS => a in DEFAULTS;
