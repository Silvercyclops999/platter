/**
 * What a press does on each key: the commands sent to Platter for a
 * gesture, given the key's settings and what's playing. Pure, for tests.
 */
import type { Command, LibraryKind, PlatterState } from "./client.js";
import type { Snapshot } from "./live.js";
import type { SettingsOf } from "./settings.js";
import type { Gesture } from "./timing.js";
import { type KeyActionId, PLAY_KIND } from "./views.js";

/** Keys whose extra gestures mean something (the rest act on any press at once). */
export const GESTURES: Partial<Record<KeyActionId, { double?: boolean; long?: boolean }>> = {
  playpause: { double: true, long: true },
  shuffle: { long: true },
  sleep: { long: true },
};

/** Keys that repeat while held. */
export const REPEATS: KeyActionId[] = ["volumeup", "volumedown", "seekback", "seekforward"];

/** The commands for one gesture on a key; empty when it does nothing. */
export function pressCommands(action: KeyActionId, g: Gesture, raw: Record<string, unknown>, snap: Snapshot): Command[] {
  const s: PlatterState | null = snap.state;
  const st = raw as never;
  switch (action) {
    case "playpause":
      // Double: next; triple or long: previous.
      return [{ command: g === "double" ? "next" : g === "triple" || g === "long" ? "previous" : "playPause" }];
    case "previous":
      return [{ command: "previous" }];
    case "next":
      return [{ command: "next" }];
    case "seekback":
    case "seekforward": {
      const sec = (st as SettingsOf<"seekback">).seconds;
      return [{ command: "seekBy", value: action === "seekback" ? -sec : sec }];
    }
    case "songinfo":
      return [{ command: "nowPlaying" }];
    case "shuffle":
      // Long press: smart shuffle.
      return [g === "long" ? { command: "shuffle", smart: true } : { command: "shuffle" }];
    case "repeat":
      return [{ command: "repeat" }];
    case "crossfade":
      return [{ command: "crossfade" }];
    case "private":
      return [{ command: "privateSession" }];
    case "volumeup":
    case "volumedown": {
      const step = (st as SettingsOf<"volumeup">).step;
      return [{ command: "volumeStep", value: action === "volumeup" ? step : -step }];
    }
    case "mute":
      return [{ command: "mute" }];
    case "volumeset":
      return [{ command: "volumeSet", value: (st as SettingsOf<"volumeset">).level }];
    case "like":
      return [{ command: "like" }];
    case "addtoplaylist": {
      const p = (st as SettingsOf<"addtoplaylist">).playlist;
      if (!p) return [{ command: "addToPlaylist" }];
      // Pressed again on the same song: take it out again.
      return [s?.playlists.includes(p) ? { command: "removeFromPlaylist", playlist: p } : { command: "addToPlaylist", playlist: p }];
    }
    case "removefromplaylist": {
      const p = (st as SettingsOf<"removefromplaylist">).playlist;
      return p ? [{ command: "removeFromPlaylist", playlist: p }] : [];
    }
    case "rate":
      return [{ command: "rate", stars: (st as SettingsOf<"rate">).stars }];
    case "playplaylist":
    case "playalbum":
    case "playartist":
    case "playmix": {
      const p = st as SettingsOf<"playplaylist">;
      if (!p.id) return [];
      const kind = PLAY_KIND[action] as LibraryKind;
      if (p.mode === "queue") return [{ command: "playItem", kind, id: p.id, queue: true }];
      return [{ command: "playItem", kind, id: p.id, shuffle: p.shuffle, ...(p.start > 0 ? { start: p.start } : {}) }];
    }
    case "playlink": {
      const link = (st as SettingsOf<"playlink">).link.trim();
      return link ? [{ command: "playLink", link }] : [];
    }
    case "playsearch": {
      const query = (st as SettingsOf<"playsearch">).query.trim();
      return query ? [{ command: "playSearch", query }] : [];
    }
    case "radio":
      return [{ command: "radio" }];
    case "daylist":
    case "discover":
      return [{ command: "playShortcut", name: action }];
    case "likedsongs":
      return [{ command: "playShortcut", name: "liked" }];
    case "queue":
      return [{ command: "queue" }];
    case "listenon": {
      const d = (st as SettingsOf<"listenon">).device;
      return [d ? { command: "listenOn", device: d } : { command: "listenOn" }];
    }
    case "lyrics":
      return [{ command: "lyrics" }];
    case "miniplayer":
      return [{ command: "miniplayer" }];
    case "window":
      return [{ command: "window" }];
    case "sleep": {
      const sl = st as SettingsOf<"sleep">;
      const start: Command = sl.endOfSong ? { command: "sleep", endOfSong: true } : { command: "sleep", minutes: sl.minutes };
      const running = s?.sleep;
      // Long press: start again from the full time.
      if (g === "long") return [start];
      if (!running) return [start];
      if (sl.whileRunning === "add" && running.remainingMs !== null && !running.endOfSong) {
        const left = Math.max(0, running.remainingMs - Math.max(0, Date.now() - snap.at)) / 60000;
        return [{ command: "sleep", minutes: Math.round((left + sl.addMinutes) * 100) / 100 }];
      }
      return [{ command: "sleepCancel" }];
    }
    case "bookmark":
      return [{ command: "bookmark" }];
    case "request":
      return [{ command: "requestAlbum" }];
    case "listen":
      return [{ command: "listenAlong" }];
  }
}

/** Commands that bring Platter's window up: the key shows OK after them. */
export const SHOWS_OK = new Set<Command["command"]>(["requestAlbum", "listenAlong", "bookmark", "addToPlaylist", "removeFromPlaylist"]);
