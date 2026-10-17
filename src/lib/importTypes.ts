/** Library import (docs/IPC.md, import_* commands). */
import type { LidarrRequestState, PlayedPeriod, Track } from "./types";

export type ImportSourceId = "streaming" | "transfer" | "lists";

export interface ImportSkipped {
  /** Plays marked private. */
  privatePlays: number;
  /** Podcast, episode and audiobook records. */
  podcasts: number;
  /** History records with no song name. */
  noMetadata: number;
  /** Plays under 30 seconds. */
  shortPlays: number;
  /** Rows with no title or no artist. */
  incompleteRows: number;
  /** Files that weren't anything this source reads. */
  unknownFiles: number;
}

export interface ImportCounts {
  total: number;
  matched: number;
}

/** How one source's import went. */
export interface ImportReport {
  source: ImportSourceId;
  /** Unix ms. */
  importedAt: number;
  fileName: string | null;
  songs: { total: number; matched: number; unsure: number; missing: number };
  liked: ImportCounts;
  playlists: number;
  albums: ImportCounts;
  plays: ImportCounts;
  /** New plays the last import added. */
  playsAdded: number;
  /** The listening history this source brought in. */
  history: ImportHistory;
  skipped: ImportSkipped;
}

/** What a source's imported listening history holds. */
export interface ImportHistory {
  plays: number;
  /** Plays of songs in the library. */
  playsMatched: number;
  /** Different songs played. */
  songs: number;
  /** Of those, songs in the library. */
  songsMatched: number;
  /** Unix ms; null without plays. */
  firstPlayed: number | null;
  lastPlayed: number | null;
}

/** An imported playlist, or a source's liked songs. */
export interface ImportedList {
  /** "pl:<n>" or "liked:<source>". */
  id: string;
  source: ImportSourceId;
  name: string;
  liked: boolean;
  total: number;
  matched: number;
}

export interface ImportOverview {
  reports: ImportReport[];
  lists: ImportedList[];
  reviewCount: number;
  missingCount: number;
}

export type ImportStatus = "matched" | "unsure" | "missing";

export interface ImportedRow {
  songId: number;
  artist: string;
  title: string;
  album: string | null;
  durationMs: number | null;
  status: ImportStatus;
  /** The library song, when matched. */
  track: Track | null;
}

export interface ImportedListPage {
  list: ImportedList;
  rows: ImportedRow[];
}

/** An unsure match waiting for a decision. */
export interface ImportReviewItem {
  songId: number;
  source: ImportSourceId;
  artist: string;
  title: string;
  album: string | null;
  durationMs: number | null;
  /** Library songs it could be, best first. */
  candidates: Track[];
}

export type ImportDecision = { kind: "accept" } | { kind: "pick"; trackId: string } | { kind: "none" };

export interface MissingSong {
  artist: string;
  title: string;
  album: string | null;
  /** How many imported playlists and liked lists want it. */
  wants: number;
  wantedBy: string[];
  sources: ImportSourceId[];
  links: { bandcamp: string; qobuz: string; itunes: string };
}

export type PlayedKind = "song" | "album";

/** `import_played`: one page of "Played a lot, not in your library". */
export interface PlayedQuery {
  kind: PlayedKind;
  offset: number;
  minPlays: number;
  period: PlayedPeriod;
}

export interface PlayedRow {
  kind: PlayedKind;
  /** Its hide key, also a stable id. */
  key: string;
  artist: string;
  /** The song's title, or the album's. */
  title: string;
  /** A song's album, when the history named it. */
  album: string | null;
  plays: number;
  /** Total time played, ms. */
  ms: number;
  /** Unix ms. */
  firstPlayed: number;
  lastPlayed: number;
  /** An album's songs that were played. */
  songs: number | null;
  /** An album's songs already in the library. */
  inLibrary: number | null;
  links: { bandcamp: string; qobuz: string; itunes: string };
  thumb: string | null;
  /** The Lidarr request made for its album, if any. */
  request: LidarrRequestState | null;
}

export interface PlayedPage {
  kind: PlayedKind;
  /** Rows matching the filters, over every page. */
  total: number;
  offset: number;
  rows: PlayedRow[];
  /** Any listening history was imported at all. */
  hasHistory: boolean;
}

/** A row hidden from "Played a lot" (synced). */
export interface PlayedHidden {
  kind: "song" | "album" | "artist";
  artist: string;
  /** The song or album; empty for an artist. */
  name: string;
  /** Unix seconds. */
  hiddenAt: number;
}
