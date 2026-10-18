/**
 * Every word the library import screens use for the services people import
 * from. Service names appear only here, as plain names of sources and in
 * the steps for getting the file, so they are easy to check in one place.
 */
import type { ImportSourceId } from "./importTypes";

export interface ImportSourceText {
  id: ImportSourceId;
  /** The source's name in Settings and reports. */
  name: string;
  /** How to get the file. */
  help: string;
  /** One line, when `help` is longer (Settings shows `help` behind its "?"). */
  short?: string;
  /** A note under the help, when there's something to know. */
  note?: string;
}

export const IMPORT_SOURCES: ImportSourceText[] = [
  {
    id: "streaming",
    name: "Spotify",
    help:
      'On spotify.com open Account > Privacy settings, tick "Account data" (library and playlists) and "Extended streaming history" (everything you ever played), request them, then pick the zips from the emails (usually a few days, up to 30).',
    short: "Request your account data and streaming history, then pick the zips they email.",
    note: "Private session plays and podcasts are left out. Only the song, when it played and for how long are kept.",
  },
  {
    id: "transfer",
    name: "Tidal, Deezer, Amazon Music, SoundCloud",
    help:
      "Use a free transfer tool (TuneMyMusic or Soundiiz) to export your library to a CSV file, then pick that file here.",
    short: "Export your library to a CSV with a free transfer tool, then pick it.",
    note: "The transfer tool sees your library while it makes the file.",
  },
  {
    id: "lists",
    name: "CSV or M3U list",
    help: "Pick any CSV or M3U with at least artist and title columns.",
  },
];

/** Store search links on the Missing list, by the key the core gives them. */
export const STORE_LINKS = [
  { key: "bandcamp", name: "Bandcamp" },
  { key: "qobuz", name: "Qobuz" },
  { key: "itunes", name: "iTunes" },
] as const;

/** Percent-encodes every byte but letters and digits, as the core does for store links. */
function encodeAll(s: string): string {
  return Array.from(new TextEncoder().encode(s), (b) =>
    (b >= 48 && b <= 57) || (b >= 65 && b <= 90) || (b >= 97 && b <= 122) ? String.fromCharCode(b) : `%${b.toString(16).toUpperCase().padStart(2, "0")}`,
  ).join("");
}

/** Store search pages for a song, the same links the Missing list gets from the core. */
export function storeLinks(artist: string, title: string): Record<(typeof STORE_LINKS)[number]["key"], string> {
  const q = encodeAll(`${artist} ${title}`);
  return {
    bandcamp: `https://bandcamp.com/search?q=${q}&item_type=t`,
    qobuz: `https://www.qobuz.com/search?q=${q}`,
    itunes: `https://music.apple.com/search?term=${q}`,
  };
}

export function importSourceName(id: string): string {
  return IMPORT_SOURCES.find((s) => s.id === id)?.name ?? id;
}
