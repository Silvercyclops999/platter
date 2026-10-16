/**
 * The Stream Deck + dials: what turning and pressing do, and what the touch
 * strip shows. Kept free of the Stream Deck code so it can be tested.
 *
 * - Multimedia: the player view (title, artist, elapsed left and the song's
 *   length right, a bar) with a volume overlay while turning; double press
 *   switches to the browser (playlists, albums or mixes) and back.
 * - Play / Pause: cover, title, artist, a centred time and a bar.
 * - Volume: a big number and a bar; press mutes and unmutes.
 * - Queue and Listen on: turn through the list, press to pick.
 */
import type { Command, Device, LibraryItem, QueueItem } from "./client.js";
import { clock, elapsed, hasSong, progress, type Snapshot } from "./live.js";
import { browseStrip, listStrip, messageStrip, nowStrip, playerStrip, volumeStrip } from "./render.js";
import type { SettingsOf } from "./settings.js";
import { badgeOf } from "./views.js";

export type DialId = "volume" | "playpausedial" | "volumedial" | "queuedial" | "listenondial";

/** Per dial: the view and where the list is. */
export interface DialModel {
  view: "player" | "browse";
  /** Browser, queue or device list position. */
  index: number;
  /** The volume overlay shows (after a turn). */
  overlay: boolean;
}

export const newDial = (): DialModel => ({ view: "player", index: 0, overlay: false });

/** Lists a dial may show, fetched by the plugin. */
export interface DialLists {
  browse: LibraryItem[];
  queue: QueueItem[];
  devices: Device[];
}

/** Turning: the command to send, or a move in the list (model changed in place). */
export function dialTurn(id: DialId, st: Record<string, unknown>, m: DialModel, ticks: number, lists: DialLists): Command | null {
  const move = (len: number) => {
    m.index = len > 0 ? Math.max(0, Math.min(len - 1, m.index + ticks)) : 0;
    return null;
  };
  switch (id) {
    case "volume": {
      if (m.view === "browse") return move(lists.browse.length);
      const s = st as unknown as SettingsOf<"volume">;
      if (s.turn === "seek") return { command: "seekBy", value: ticks * s.step };
      if (s.turn === "skip") return { command: ticks > 0 ? "next" : "previous" };
      m.overlay = true;
      return { command: "volumeStep", value: ticks * s.step };
    }
    case "playpausedial":
      return { command: "seekBy", value: ticks * 5 };
    case "volumedial":
      return { command: "volumeStep", value: ticks * (st as unknown as SettingsOf<"volumedial">).step };
    case "queuedial":
      return move(lists.queue.length);
    case "listenondial":
      return move(lists.devices.length);
  }
}

/** A press (or a touch): what it sends. A double press on Multimedia switches view instead. */
export function dialPress(id: DialId, st: Record<string, unknown>, m: DialModel, double: boolean, lists: DialLists): Command | null {
  switch (id) {
    case "volume":
      if (double) {
        m.view = m.view === "player" ? "browse" : "player";
        m.overlay = false;
        return null;
      }
      return m.view === "browse" ? browsePlay(st, m, lists) : { command: "playPause" };
    case "playpausedial":
      return { command: "playPause" };
    case "volumedial":
      return { command: "mute" };
    case "queuedial": {
      const q = lists.queue[m.index];
      return q ? { command: "queueJump", itemId: q.itemId } : null;
    }
    case "listenondial": {
      const d = lists.devices[m.index];
      return d ? { command: "listenOn", device: d.id } : null;
    }
  }
}

/** The browser's play command (it needs the list kind from the settings). */
export function browsePlay(st: Record<string, unknown>, m: DialModel, lists: DialLists): Command | null {
  const it = lists.browse[m.index];
  const kind = (st as unknown as SettingsOf<"volume">).browse;
  return it ? { command: "playItem", kind, id: it.id } : null;
}

const HEADINGS = { playlists: "Playlists", albums: "Albums", mixes: "Mixes" } as const;

/** The touch strip picture for a dial. */
export function dialImage(
  id: DialId,
  st: Record<string, unknown>,
  m: DialModel,
  snap: Snapshot,
  now: number,
  lists: DialLists,
  cover: (key: string | null | undefined) => string | null,
): string {
  if (snap.conn === "offline") return messageStrip("Platter not running", "Open Platter to use this");
  if (snap.conn === "approve") return messageStrip("Approve in Platter", "Choose Allow in Platter's window");
  const s = snap.state;
  const b = badgeOf(snap);
  const song = hasSong(s);
  const title = song ? (s.private ? "Private session" : (s.title ?? "")) : "Nothing playing";
  const artist = song ? (s.artist ?? "-") : "-";
  switch (id) {
    case "volume": {
      if (m.view === "browse") {
        const ms = st as unknown as SettingsOf<"volume">;
        return browseStrip({
          heading: HEADINGS[ms.browse],
          index: m.index,
          style: ms.browser,
          items: lists.browse.map((it, i) => ({ title: it.title, subtitle: it.subtitle, cover: Math.abs(i - m.index) <= 1 ? cover(it.coverKey) : null })),
        });
      }
      return playerStrip({
        cover: song ? (s.cover ?? null) : null,
        title,
        artist,
        elapsed: clock(elapsed(snap, now)),
        // The song's whole length, not what's left.
        total: clock(song ? s.durationMs : 0),
        progress: progress(snap, now),
        paused: song && !s.playing,
        overlay: m.overlay && s ? { volume: s.volume, muted: s.muted } : null,
        badge: b,
      });
    }
    case "playpausedial":
      return nowStrip({
        cover: song ? (s.cover ?? null) : null,
        title,
        artist,
        time: song ? clock(elapsed(snap, now)) : "0:00",
        progress: progress(snap, now),
        paused: song && !s.playing,
        badge: b,
      });
    case "volumedial":
      return volumeStrip(s?.volume ?? 0, !!s?.muted, b);
    case "queuedial": {
      const q = lists.queue[m.index];
      return listStrip("Up next", q?.title ?? null, q?.artist ?? null, q ? cover(q.coverKey) : null, lists.queue.length ? `${m.index + 1} / ${lists.queue.length}` : "");
    }
    case "listenondial": {
      const d = lists.devices[m.index];
      return listStrip("Listen on", d?.name ?? null, d ? (d.active ? "Playing here" : d.needsWindow ? "Opens in Platter" : "Press to play here") : null, null, lists.devices.length ? `${m.index + 1} / ${lists.devices.length}` : "", !!d?.active);
    }
  }
}
