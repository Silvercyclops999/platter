/** Library import (docs/IPC.md, import_* commands). */
import type { Track } from "./types";

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
  skipped: ImportSkipped;
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
