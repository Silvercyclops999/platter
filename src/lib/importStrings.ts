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
    id: "streaming-account",
    name: "Spotify playlists from your account",
    help:
      "Connect your account to bring in the playlists you made or collaborate on, right away, with each song's ISRC for exact matching. It needs a free developer app of your own (its owner needs Premium): on developer.spotify.com open the Dashboard, create an app with the Web API ticked, add the redirect URI below, add your account under User Management, then paste the app's Client ID here.",
    short: "Connect your account to bring in your own playlists right away.",
    note: "Playlists only. Liked songs, saved albums, your listening history and playlists other people made come from the account data zip above.",
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

/** The connect card's words for the account source. */
export const ACCOUNT_TEXT = {
  service: "Spotify",
  brings: "Your playlists and the ones you collaborate on, right away, with exact song codes for matching.",
  cant: "Liked songs, saved albums, your listening history and other people's playlists can't come this way. Those come from the account data zip.",
} as const;

export function importSourceName(id: string): string {
  return IMPORT_SOURCES.find((s) => s.id === id)?.name ?? id;
}
