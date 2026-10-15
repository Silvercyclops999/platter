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
  /** One line: how to get the file. */
  help: string;
  /** A note under the help, when there's something to know. */
  note?: string;
}

export const IMPORT_SOURCES: ImportSourceText[] = [
  {
    id: "streaming",
    name: "Spotify",
    help:
      'On spotify.com open Account > Privacy settings, tick "Account data" (library and playlists) and "Extended streaming history" (everything you ever played), request them, then pick the zips from the emails (usually a few days, up to 30).',
    note: "Private session plays and podcasts are left out. Only the song, when it played and for how long are kept.",
  },
  {
    id: "transfer",
    name: "Tidal, Deezer, Amazon Music, SoundCloud",
    help:
      "Use a free transfer tool (TuneMyMusic or Soundiiz) to export your library to a CSV file, then pick that file here.",
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

export function importSourceName(id: string): string {
  return IMPORT_SOURCES.find((s) => s.id === id)?.name ?? id;
}
