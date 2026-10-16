/**
 * Each action's settings (set in its panel in the Stream Deck app), with
 * defaults filled in and odd values brought back in range.
 */

export type TimeShown = "off" | "elapsed" | "remaining";

export interface PlayPauseSettings {
  /** Time in a band at the bottom of the cover. */
  time: TimeShown;
  /** The song's title over the cover. */
  showTitle: boolean;
  /** Paused: a see-through circle with a play symbol, or a small "Paused" label. */
  pausedLook: "circle" | "label";
  /** Part of a group of keys sharing one big cover. */
  group: "off" | "auto" | "manual";
  /** Keys a side (2, 3 or 4) and this key's column and row in it (1 based). */
  groupSize: number;
  groupCol: number;
  groupRow: number;
}

export interface CoverSettings {
  /** Show the cover of the song before / after. */
  cover: boolean;
}

export interface SeekSettings {
  seconds: number;
}

export interface SongInfoSettings {
  /** Also put the song info on the clipboard or in a text file. */
  output: "none" | "clipboard" | "file";
  /** With {title}, {artist}, {album}, {elapsed}, {total}. */
  format: string;
  /** The text file, rewritten on every song change. */
  file: string;
}

export interface VolumeStepSettings {
  step: number;
  /** What the key shows of the level. */
  show: "none" | "bar" | "number";
}

export interface VolumeSetSettings {
  level: number;
}

export interface PlaylistSettings {
  /** Playlist id; empty asks in Platter. */
  playlist: string;
  /** Its name, shown on the key. */
  playlistName: string;
}

export interface RateSettings {
  stars: number;
}

export interface PlayItemSettings {
  id: string;
  name: string;
  shuffle: boolean;
  /** Play now, or add to the queue. */
  mode: "play" | "queue";
  /** Seconds into the first song. */
  start: number;
}

export interface LinkSettings {
  link: string;
}

export interface SearchSettings {
  query: string;
}

export interface DeviceSettings {
  /** Device id from Listen on; empty asks in Platter. */
  device: string;
  deviceName: string;
}

export interface SleepSettings {
  minutes: number;
  endOfSong: boolean;
  /** A press while the timer runs: stop it, or add time. */
  whileRunning: "stop" | "add";
  addMinutes: number;
}

export interface MultimediaSettings {
  /** What turning does. */
  turn: "volume" | "seek" | "skip";
  /** Volume points (or seconds, for seek) per tick. */
  step: number;
  /** The browser view: covers, or a list. */
  browser: "covers" | "list";
  browse: "playlists" | "albums" | "mixes";
}

export interface VolumeDialSettings {
  step: number;
}

/** Every action may use the user's own picture for a state ("playing", "paused", "on", "off"...). */
export interface Common {
  images?: Record<string, string>;
}

export const DEFAULTS = {
  playpause: { time: "off", showTitle: false, pausedLook: "circle", group: "off", groupSize: 2, groupCol: 1, groupRow: 1 } as PlayPauseSettings,
  previous: { cover: false } as CoverSettings,
  next: { cover: false } as CoverSettings,
  seekback: { seconds: 10 } as SeekSettings,
  seekforward: { seconds: 10 } as SeekSettings,
  songinfo: { output: "none", format: "{title} - {artist}", file: "" } as SongInfoSettings,
  volumeup: { step: 5, show: "none" } as VolumeStepSettings,
  volumedown: { step: 5, show: "none" } as VolumeStepSettings,
  volumeset: { level: 50 } as VolumeSetSettings,
  addtoplaylist: { playlist: "", playlistName: "" } as PlaylistSettings,
  removefromplaylist: { playlist: "", playlistName: "" } as PlaylistSettings,
  rate: { stars: 5 } as RateSettings,
  playplaylist: { id: "", name: "", shuffle: false, mode: "play", start: 0 } as PlayItemSettings,
  playalbum: { id: "", name: "", shuffle: false, mode: "play", start: 0 } as PlayItemSettings,
  playartist: { id: "", name: "", shuffle: false, mode: "play", start: 0 } as PlayItemSettings,
  playmix: { id: "", name: "", shuffle: false, mode: "play", start: 0 } as PlayItemSettings,
  playlink: { link: "" } as LinkSettings,
  playsearch: { query: "" } as SearchSettings,
  listenon: { device: "", deviceName: "" } as DeviceSettings,
  sleep: { minutes: 30, endOfSong: false, whileRunning: "stop", addMinutes: 15 } as SleepSettings,
  volume: { turn: "volume", step: 5, browser: "covers", browse: "playlists" } as MultimediaSettings,
  volumedial: { step: 5 } as VolumeDialSettings,
} as const;

export type SettingsOf<K extends keyof typeof DEFAULTS> = (typeof DEFAULTS)[K] & Common;

const clamp = (v: unknown, lo: number, hi: number, d: number): number => {
  const n = typeof v === "string" && v.trim() !== "" ? Number(v) : v;
  return typeof n === "number" && Number.isFinite(n) ? Math.min(hi, Math.max(lo, Math.round(n))) : d;
};
const oneOf = <T extends string>(v: unknown, ok: readonly T[], d: T): T => (typeof v === "string" && (ok as readonly string[]).includes(v) ? (v as T) : d);
const bool = (v: unknown, d: boolean): boolean => (typeof v === "boolean" ? v : v === "true" ? true : v === "false" ? false : d);
const str = (v: unknown, d: string): string => (typeof v === "string" ? v : d);

/** Ranges and choices per field. */
const RULES: Record<string, (v: unknown, d: unknown) => unknown> = {
  time: (v, d) => oneOf(v, ["off", "elapsed", "remaining"], d as TimeShown),
  pausedLook: (v, d) => oneOf(v, ["circle", "label"], d as "circle"),
  group: (v, d) => oneOf(v, ["off", "auto", "manual"], d as "off"),
  groupSize: (v, d) => clamp(v, 2, 4, d as number),
  groupCol: (v, d) => clamp(v, 1, 4, d as number),
  groupRow: (v, d) => clamp(v, 1, 4, d as number),
  seconds: (v, d) => clamp(v, 1, 600, d as number),
  output: (v, d) => oneOf(v, ["none", "clipboard", "file"], d as "none"),
  step: (v, d) => clamp(v, 1, 25, d as number),
  show: (v, d) => oneOf(v, ["none", "bar", "number"], d as "none"),
  level: (v, d) => clamp(v, 0, 100, d as number),
  stars: (v, d) => clamp(v, 1, 5, d as number),
  mode: (v, d) => oneOf(v, ["play", "queue"], d as "play"),
  start: (v, d) => clamp(v, 0, 24 * 3600, d as number),
  minutes: (v, d) => clamp(v, 1, 720, d as number),
  addMinutes: (v, d) => clamp(v, 1, 240, d as number),
  whileRunning: (v, d) => oneOf(v, ["stop", "add"], d as "stop"),
  turn: (v, d) => oneOf(v, ["volume", "seek", "skip"], d as "volume"),
  browser: (v, d) => oneOf(v, ["covers", "list"], d as "covers"),
  browse: (v, d) => oneOf(v, ["playlists", "albums", "mixes"], d as "playlists"),
};

/** An action's settings with defaults filled in and values in range. */
export function settingsFor<K extends keyof typeof DEFAULTS>(action: K, raw: unknown): SettingsOf<K> {
  const r = (raw && typeof raw === "object" ? raw : {}) as Record<string, unknown>;
  const d = DEFAULTS[action] as unknown as Record<string, unknown>;
  const out: Record<string, unknown> = {};
  for (const [k, def] of Object.entries(d)) {
    const rule = RULES[k];
    out[k] = rule ? rule(r[k], def) : typeof def === "boolean" ? bool(r[k], def) : typeof def === "number" ? clamp(r[k], -1e9, 1e9, def) : str(r[k], def as string);
  }
  out.images = imagesOf(raw);
  return out as unknown as SettingsOf<K>;
}

/** The user's own pictures, per state; only data images are kept. */
export function imagesOf(raw: unknown): Record<string, string> {
  const r = (raw && typeof raw === "object" ? raw : {}) as Record<string, unknown>;
  const imgs: Record<string, string> = {};
  if (r.images && typeof r.images === "object") {
    for (const [k, v] of Object.entries(r.images as Record<string, unknown>)) {
      if (typeof v === "string" && v.startsWith("data:image/")) imgs[k] = v;
    }
  }
  return imgs;
}

/** Fill a song-info format: {title}, {artist}, {album}, {elapsed}, {total}. */
export function formatInfo(format: string, f: { title: string; artist: string; album: string; elapsed: string; total: string }): string {
  return format.replace(/\{(title|artist|album|elapsed|total)\}/g, (_, k: keyof typeof f) => f[k]);
}
