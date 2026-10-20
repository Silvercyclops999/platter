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
  {
    id: "apple",
    name: "Apple Music",
    help: "In the Music app on a Mac or iTunes on Windows, choose File > Library > Export Library, then pick the XML file it saves. Loved songs come in as liked songs, with your own playlists.",
    short: "Export your library from the Music app (File > Library > Export Library), then pick the XML file.",
  },
];

/** Bring music over, step 1: where it comes from, and how to get the file. */
export interface BringChoice {
  id: "streaming" | "apple" | "youtube" | "lastfm" | "lists";
  name: string;
  /** One line on how to get the file. */
  how: string;
  /** A page to start from, when there is one. */
  link?: { label: string; url: string };
  /** Said under the steps when it's worth knowing. */
  note?: string;
}

export const BRING_CHOICES: BringChoice[] = [
  {
    id: "streaming",
    name: "Spotify",
    how: 'Request your "Account data" (and "Extended streaming history" if you like) from the privacy page, then drop the zip they email you here.',
    link: { label: "Open the request page", url: "https://www.spotify.com/account/privacy/" },
    note: "The file is usually ready in a few days. Your playlists can come over now while you wait.",
  },
  {
    id: "apple",
    name: "Apple Music",
    how: "In the Music app choose File > Library > Export Library, then drop the XML file here.",
  },
  {
    id: "youtube",
    name: "YouTube Music",
    how: "Export your YouTube Music library and playlists from Google Takeout, unzip it, then drop the CSV files here one at a time.",
    link: { label: "Open Google Takeout", url: "https://takeout.google.com/" },
  },
  {
    id: "lastfm",
    name: "Last.fm",
    how: "Export your loved tracks to a CSV with a free Last.fm export tool, then drop the CSV here.",
  },
  {
    id: "lists",
    name: "Any CSV or M3U",
    how: "Any list with artist and title columns, from a transfer tool (TuneMyMusic, Soundiiz) or a player. Drop it here.",
  },
];

/** Pages bringing music over may open in the browser. */
export const BRING_LINKS = BRING_CHOICES.flatMap((c) => (c.link ? [c.link.url] : []));

/** The playlist name that brings liked songs over through a connected account. */
export const LIKED_LIST_NAME = "Liked Songs for Platter";

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
