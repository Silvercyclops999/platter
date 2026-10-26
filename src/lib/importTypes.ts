/** Library import (docs/IPC.md, import_* commands). */
import type { LidarrRequestState, PlayedPeriod, Track } from "./types";

export type ImportSourceId = "streaming" | "streaming-account" | "transfer" | "lists" | "apple";

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
  /** Playlists other people own, which a connected account can't read the songs of. */
  othersPlaylists: number;
}

/** Where a connected streaming account stands (streaming_account_* commands). */
export type AccountState = "notSetUp" | "signedOut" | "signingIn" | "connected" | "reading" | "refused" | "expired" | "rateLimited" | "offline";

export interface AccountStatus {
  state: AccountState;
  /** What to register as the developer app's redirect URI. */
  redirectUri: string;
  /** The account's display name, once read. */
  name: string | null;
  /** Why it stopped, or what the last read left out. */
  message: string | null;
  /** Unix ms when a rate limit ends. */
  retryAt: number | null;
  /** Playlists read and to read, while reading. */
  done: number;
  total: number;
  /** Unix ms of the last finished read. */
  lastRead: number | null;
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
/** An artist left out of "Played a lot" as sleep or background music. */
export interface PlayedSleepArtist {
  artist: string;
  plays: number;
  /** Percent of its plays that started late at night. */
  nightPercent: number;
}

export interface PlayedHidden {
  kind: "song" | "album" | "artist";
  artist: string;
  /** The song or album; empty for an artist. */
  name: string;
  /** Unix seconds. */
  hiddenAt: number;
}

// ------------------------------------------------------------------ bringing music over (bring_* commands)

export interface BringCounts {
  total: number;
  found: number;
}

/** What the files brought, against the library. */
export interface BringSummary {
  sources: ImportSourceId[];
  liked: BringCounts;
  /** Every song the likes and playlists name, once each. */
  songs: BringCounts;
  playlists: number;
  missingSongs: number;
  /** Albums to get. */
  missingAlbums: number;
  /** Songs waiting in the review queue. */
  unsure: number;
  /** "812 of 900 liked songs are already in your library. 88 songs from 31 albums aren't." */
  sentence: string;
}

/** A playlist brought over. */
export interface BroughtList {
  name: string;
  /** The imported list ("pl:<n>"), whose page shows the greyed rows. */
  importId: string;
  /** The server playlist it became; null until a song of it is found. */
  playlistId: string | null;
  /** The user deleted it: it isn't filled any more. */
  gone: boolean;
  total: number;
  found: number;
  /** The server playlist's cover. */
  thumb: string | null;
}

export type BringAlbumState = "missing" | "picking" | "asked" | "notFound" | "here";

export interface BringAlbum {
  key: string;
  artist: string;
  album: string;
  songs: number;
  liked: number;
  state: BringAlbumState;
  /** Why it couldn't be found ("Lidarr couldn't find it. ..."). */
  reason: string | null;
  releaseGroupId: string | null;
  /** Its cover, once MusicBrainz has picked it. */
  thumb: string | null;
}

/** "31 albums on the way, 4 here so far". */
export interface BringProgress {
  toGet: number;
  /** Asked for, those here included. */
  onTheWay: number;
  here: number;
  notFound: number;
}

export interface BringStatus {
  summary: BringSummary;
  lists: BroughtList[];
  albums: BringAlbum[];
  progress: BringProgress;
  lidarr: { ready: boolean; practiceUntil: number | null };
  /** What went to the server the last time anything was sent. */
  sent: { liked: number; playlistsMade: number; songsAdded: number };
}
