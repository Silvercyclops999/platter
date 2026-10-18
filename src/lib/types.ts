/**
 * TypeScript mirror of `src-tauri/src/model.rs` (canonical) and every IPC
 * type in `docs/IPC.md`. Wire format is camelCase JSON. Rust `Option<T>`
 * serializes as `null`, so model fields are `T | null`; IPC types documented
 * with `?` are optional and may also be null.
 */
import { CLASSIC_LAYOUT, type Layout, type UserTheme } from "../design/themes/validate";

// ---------------------------------------------------------------- model.rs

export type ReleaseSection =
  | "albums"
  | "singlesEps"
  | "live"
  | "compilations"
  | "soundtracks"
  | "remixes"
  | "demos"
  | "mixtapes"
  /** Unreleased material, by release status (artist pages only). */
  | "unreleased";

export interface Artist {
  id: string;
  title: string;
  sortTitle: string;
  thumb: string | null;
  art: string | null;
  viewCount: number;
}

export interface Album {
  id: string;
  title: string;
  sortTitle: string;
  artistId: string;
  artistTitle: string;
  year: number | null;
  thumb: string | null;
  /** Lower-case Plex format, e.g. "album", "ep", "single". null when untagged. */
  format: string | null;
  /** Lower-case Plex subformats, e.g. ["live"], ["soundtrack"]. */
  subformats: string[];
  section: ReleaseSection;
  genres: string[];
  trackCount: number;
  addedAt: number;
  viewCount: number;
  lastViewedAt: number | null;
  /** Server-computed header tint (hex without '#'), if present. */
  tint: string | null;
}

export interface Track {
  id: string;
  title: string;
  albumId: string;
  albumTitle: string;
  /** Album artist (Plex grandparent). */
  artistId: string;
  artistTitle: string;
  /** Track artist(s) when different from the album artist. */
  trackArtist: string | null;
  trackNumber: number;
  discNumber: number;
  durationMs: number;
  thumb: string | null;
  partKey: string;
  codec: string | null;
  container: string | null;
  bitrate: number | null;
  bitDepth: number | null;
  sampleRate: number | null;
  userRating: number | null;
  viewCount: number;
  lastViewedAt: number | null;
  /** Plex ratingCount; absent when it has none. Not on one scale: only breaks ties in Popular. */
  ratingCount?: number;
  /** When the user last rated it, unix seconds (for a liked song: when it was liked). */
  ratedAt?: number | null;
  /** Plex genre and mood tags on the track itself. */
  genres?: string[];
  /** How many other versions of the song the library has (pages that fill it in). */
  otherVersions?: number | null;
  /** The song's own release status (pages that fill it in). */
  songStatus?: SongStatus | null;
  /** What the song's file tags say about where it came from (pages that fill it in). */
  fileRelease?: TrackRelease | null;
}

/**
 * One song's release status. probablyAlternate: on an album that's probably
 * unreleased, with the title of a released song (still unreleased).
 */
export type SongStatus = "released" | "unreleased" | "probablyUnreleased" | "probablyAlternate";

/** A library artist's name and id (`artist_names`). */
export interface ArtistName {
  id: string;
  title: string;
}

/** Where queued songs go: before the rest of the queue, at its end, or at a spot in it. */
export type QueueAt = "next" | "end" | { index: number };

export interface Playlist {
  id: string;
  title: string;
  smart: boolean;
  trackCount: number;
  durationMs: number;
  thumb: string | null;
  /** The playlist's description (Plex summary). */
  summary?: string | null;
  /** Last change, unix seconds. */
  updatedAt?: number | null;
}

export type LibraryItemKind = "playlist" | "album" | "artist";

/** One row of the library sidebar (`library_items`). Times are unix seconds. */
export interface LibraryItem {
  kind: LibraryItemKind;
  id: string;
  title: string;
  sortTitle: string;
  /** Album artist (albums only). */
  creator?: string | null;
  /** Album artist id (albums only). */
  creatorId?: string | null;
  thumb?: string | null;
  smart: boolean;
  /** Lower-case Plex album format ("ep", "single"), albums only. */
  format?: string | null;
  addedAt?: number | null;
  playedAt?: number | null;
  /** A Platter smart playlist (kept on this computer, filled here). */
  platter?: boolean;
  /** A playlist someone shared with you (`shared:<group>`): its owner and your role. */
  shared?: { owner: string; role: CollabRole; gone: boolean };
  /** A Blend (`blend:<group>`). */
  blend?: boolean;
  /** The album's cover colour from the server (albums only). */
  tint?: string | null;
  /** Songs on the album (albums only). */
  trackCount?: number | null;
}

// ---------------------------------------------------------------- errors

export type AppErrorKind =
  | "network"
  | "auth"
  | "notFound"
  | "server"
  | "io"
  | "decode"
  | "invalid"
  | "internal";

export interface AppError {
  kind: AppErrorKind;
  message: string;
}

// ---------------------------------------------------------------- auth & servers

export type ConnectionKind = "local" | "remote" | "relay";

export interface ServerInfo {
  machineId: string;
  name: string;
  owned: boolean;
  online: boolean;
  version?: string | null;
  connection?: ConnectionKind | null;
}

/** A music library on one of the account's servers (Settings, Library). */
export interface SourceLibrary {
  id: string;
  title: string;
  included: boolean;
  /** The connected server's main library: always included. */
  main: boolean;
}

/** A server on the account, owned or shared, and its music libraries. */
export interface SourceServer {
  machineId: string;
  /** The server's tag, which its item ids carry (see lib/ids.ts). */
  tag: string;
  name: string;
  owned: boolean;
  /** The server Platter is connected to. */
  current: boolean;
  online: boolean;
  connection?: ConnectionKind | null;
  libraries: SourceLibrary[];
}

export interface LibrarySources {
  servers: SourceServer[];
}

/** The servers the library has music from, for the sidebar switcher. */
export interface LibraryServers {
  servers: { tag: string; name: string }[];
  /** "all" or a server tag. */
  view: string;
}

export interface AuthStatus {
  state: "signedOut" | "pending" | "signedIn";
  username?: string | null;
  /** The account's profile picture as an art path (fetched through the art cache). */
  avatar?: string | null;
  server?: ServerInfo | null;
  needsServerPick: boolean;
  /** Plex rejected the saved sign-in: keep the app open with a sign-in banner. */
  reauthRequired?: boolean;
}

/** One Plex Home user (home_users). */
export interface HomeUser {
  uuid: string;
  name: string;
  /** Profile picture as an art path, or null. */
  avatar: string | null;
  admin: boolean;
  restricted: boolean;
  /** Switching to them needs their PIN. */
  protected: boolean;
  /** The user Platter is running as. */
  current: boolean;
}

export interface HomeUsers {
  /** Empty when the account isn't a Plex Home. */
  users: HomeUser[];
  /** Settings: remember the last user on this device. */
  remember: boolean;
}

export interface PinInfo {
  code: string;
  authUrl: string;
  expiresAt: number;
}

// ---------------------------------------------------------------- library

export interface Page<T> {
  items: T[];
  total: number;
}

export interface SyncStatus {
  phase: "idle" | "artists" | "albums" | "tracks" | "incremental";
  done: number;
  total: number;
  lastSyncAt?: number | null;
}

/** One album, artist or playlist on Home (shortcut tile or mixed shelf). */
export type HomeItem =
  | { kind: "album"; album: Album }
  | { kind: "artist"; artist: Artist }
  | { kind: "playlist"; playlist: Playlist };

/** An album stopped partway, and where Resume picks it up (`home_records`). */
export interface RecordProgress {
  album: Album;
  nextTrackId: string;
  nextTitle: string;
  /** The song's place in the album, from 1. */
  nextNumber: number;
  /** Songs on the album. */
  total: number;
  /** The session's last play, unix ms. */
  playedAt: number;
}

export interface RecordSpin {
  album: Album;
  /** Plays in the last 7 days. */
  plays: number;
}

export interface RecordLapsed {
  album: Album;
  plays: number;
  /** The last play anywhere, unix ms. */
  lastPlayed: number;
}

/** Home led by records in the Studio looks (`home_records`; `shown` is false while the setting is off). */
export interface RecordHome {
  shown: boolean;
  resume: RecordProgress | null;
  newArrivals: Album[];
  spunThisWeek: RecordSpin[];
  unfinished: RecordProgress[];
  notInAYear: RecordLapsed[];
}

export interface HomeData {
  /** Up to 8 shortcut tiles: what's played most lately first, then long-time favourites, de-duplicated (Liked Songs is added by the UI). */
  shortcuts: HomeItem[];
  /** Albums, artists and playlists by most recent play. */
  recentlyPlayed: HomeItem[];
  jumpBackIn: Album[];
  /** Playlists, most recently played first, then by title. */
  playlists: Playlist[];
  recentlyAdded: Album[];
  /** Artists by play count. */
  topArtists: Artist[];
}

/** What a Recents row is: a listening session from an album, artist or
 *  playlist (`item`), from Liked Songs, of loose songs, or songs liked. */
export type RecentsKind = "context" | "liked" | "songs" | "likedAdded";

/** One row on the Recents page (`recents`). */
export interface RecentsEntry {
  kind: RecentsKind;
  item?: HomeItem | null;
  /** Newest play or like in the row, unix ms. */
  at: number;
  /** Newest first; a song played twice shows twice. */
  tracks: Track[];
}

/** A Home shelf, for its "Show all" page (`home_shelf`). */
export type HomeShelfId = "recentlyPlayed" | "jumpBackIn" | "playlists" | "recentlyAdded" | "topArtists";

export type ArtistSort = "name" | "recent" | "plays";
export type AlbumSort = "name" | "artist" | "year" | "recent" | "plays";

export interface ArtistsArgs {
  sort: ArtistSort;
  offset: number;
  limit: number;
}

export interface AlbumsArgs {
  sort: AlbumSort;
  offset: number;
  limit: number;
  section?: ReleaseSection;
}

export interface ArtistPageSection {
  section: ReleaseSection;
  albums: Album[];
  /** A section of the user's own, by name (`section` then means nothing). */
  custom?: string | null;
}

/** One of an artist's eras (ARCHIVE_ERA). Dates are ARCHIVE_DATE as tagged, else album years. */
export interface Era {
  name: string;
  start?: string | null;
  end?: string | null;
  trackCount: number;
  albumCount: number;
  cover?: string | null;
  coverAlbumId?: string | null;
  albumIds: string[];
  trackIds: string[];
}

/** `era_page`: an era's songs by album or build, newest first. */
export interface EraPage {
  artist: Artist;
  era: Era;
  groups: { album: Album; date?: string | null; edition?: string | null; tracks: Track[] }[];
}

/** An unreleased build's era banner on its album page. */
export interface AlbumArchive {
  era?: string | null;
  date?: string | null;
  edition?: string | null;
  source?: string | null;
}

/** One version in the Other versions panel (`track_versions`). */
export interface VersionItem {
  track: Track;
  status: SongStatus;
  version?: string | null;
  quality?: string | null;
  date?: string | null;
  era?: string | null;
  edition?: string | null;
  year?: number | null;
  /** The song the panel was opened from. */
  current: boolean;
}

/** Where a version sits in a song's life. */
export type LineageStage = "demo" | "session" | "leak" | "alternate" | "unreleased" | "release" | "remaster" | "live";

/** One version on a song's lineage timeline. */
export interface LineageNode extends VersionItem {
  stage: LineageStage;
  /** Sort key from the date (yyyymmdd); null when the version has no date. */
  when: number | null;
  /** Released versions only: unreleased ones never leave this computer. */
  shareable: boolean;
}

/** `track_lineage`: every version of a song, oldest first, undated last. */
export interface Lineage {
  title: string;
  artistId: string;
  artistTitle: string;
  nodes: LineageNode[];
}

export type CompareSide = "a" | "b";

/** `compare_prepare`: both files are on this computer. */
export interface CompareReady {
  aId: string;
  bId: string;
  /** Lengths differ by more than 5%: switches map by proportion. */
  approximate: boolean;
  levelMatched: boolean;
  /** The server has loudness data for both. */
  levelAvailable: boolean;
}

/** `compare_switch`: which version plays and where. */
export interface CompareNow {
  side: CompareSide;
  positionMs: number;
  abLoop: AbLoop | null;
}

export interface ArtistPage {
  artist: Artist;
  popular: Track[];
  sections: ArtistPageSection[];
  /** Sum of play counts over the artist's own tracks. */
  plays: number;
  /** Plex artist biography; null when it has none. */
  about: string | null;
  /** Plex similar artists that are in this library. */
  similar: Artist[];
  /** Albums by others with tracks that credit this artist, newest first. */
  appearsOn: Album[];
  /**
   * Popular ranks by the global list and none has been fetched yet: the
   * page shows placeholder rows until it lands, the fetch fails, or a
   * moment passes, instead of plays order that reshuffles.
   */
  popularPending?: boolean;
  /** The artist's eras, oldest first. */
  eras?: Era[];
}

export interface AlbumPage {
  album: Album;
  tracks: Track[];
  totalDurationMs: number;
  moreByArtist: Album[];
  /** An unreleased build's era banner. */
  archive?: AlbumArchive | null;
  /** The album artist's photo, for the header. */
  artistThumb?: string | null;
  /** Release date from Plex, YYYY-MM-DD, when it has one. */
  released?: string | null;
  /** Record label from Plex (its studio field), when it has one. */
  studio?: string | null;
}

export interface PlaylistPage {
  playlist: Playlist;
  tracks: Track[];
  /** One per track, same order: names a row for edits (a song can be in twice). */
  itemIds: string[];
}

/** A row taken out of a playlist and where it was (`playlist_remove`). */
export interface RemovedItem {
  itemId: string;
  trackId: string;
  index: number;
}

/** `liked_page`: liked songs, newest like first. */
export interface LikedPage {
  tracks: Track[];
  totalDurationMs: number;
}

/** Where `playlist_add` puts songs: the end (default), the top, or after a row. */
export interface AddPlace {
  after?: string;
  top?: boolean;
}

// ---------------------------------------------------------------- now playing

export interface ArtistAbout {
  id: string;
  title: string;
  summary?: string | null;
  thumb?: string | null;
  art?: string | null;
}

export type CreditRole = "composer" | "writer" | "lyricist" | "producer";

export interface Contributor {
  name: string;
  roles: CreditRole[];
}

export interface Lyrics {
  /** The source had timestamps; `timed` holds them. */
  synced: boolean;
  /** Lines without timestamps, in file order; "" marks a break between verses. */
  lines: string[];
  /** Timed lines in play order (empty text: an instrumental gap). Absent when not synced. */
  timed?: LyricLine[];
  /** Writer and provider credit to show under the lyrics. */
  credit?: string;
  /** Where the lyrics came from (absent from older builds: Plex). */
  source?: LyricsSource;
  /**
   * A live version showing lyrics found by artist and title: their timing is
   * another version's. Unsynced unless lyricsLiveTiming is on.
   */
  borrowedTiming?: boolean;
  /** What the check against the recording said about the lyrics shown. */
  check?: LyricsCheck;
  /** Nothing is shown, and why (lines and timed are empty). */
  notice?: LyricsNotice;
  /** The candidate shown (or held back with notice versionHeld). */
  pick?: string;
  /** The user picked these lyrics for the song. */
  chosen?: boolean;
  /** The user's timing nudge in ms, already applied (positive: later). */
  offsetMs?: number;
  /** The shift measured in the song, already applied. */
  autoOffsetMs?: number;
  /** The timing was stretched to follow the song. */
  retimed?: boolean;
  /** Other lyrics found for the song say something quite different. */
  disagree?: boolean;
}

/** checked: matched against the song; notChecked: not compared with it; checking: being compared now. */
export type LyricsCheck = "checked" | "notChecked" | "checking";

/** Where a song's lyrics check is (`lyrics://check-status`, `lyricsCheckStatus`). */
export interface LyricsCheckStatus {
  trackId: string;
  state: "queued" | "waiting" | "reading" | "listening" | "done" | "failed";
  /** Why it waits or couldn't check: busy (waiting), noModel, unreadable, later or stuck (failed). */
  reason?: "busy" | "noModel" | "unreadable" | "later" | "stuck";
}

/**
 * versionHeld: lyrics found for the released version, waiting for "Use anyway";
 * checking: being compared with the song and nothing safe to show yet;
 * noneMatched: every set found disagreed with the song; chosenNone: the user chose no lyrics.
 */
export type LyricsNotice = "versionHeld" | "checking" | "noneMatched" | "chosenNone" | "unavailable" | "instrumental";

/** What the check said about one set of lyrics. */
export type LyricsVerdict = "match" | "matchOffset" | "noMatch" | "cantTell";

/** Where a set of lyrics was found. */
export type LyricsOrigin = "user" | "server" | "lrclibExact" | "lrclibSearch" | "auto";

/** One set of lyrics found for a song ("Wrong lyrics?"). */
export interface LyricsCandidate {
  /** "user", "server", "auto" or "lrclib:<hash>": what lyricsChoose takes. */
  key: string;
  origin: LyricsOrigin;
  source: LyricsSource;
  album?: string;
  durationMs?: number;
  synced: boolean;
  check?: LyricsVerdict;
  /** Found for the released version while this song is another version. */
  otherVersion?: boolean;
  /** The first few lines. */
  preview: string[];
  /** Shown now. */
  shown?: boolean;
}

/** `lyrics_candidates`. */
export interface LyricsCandidates {
  items: LyricsCandidate[];
  /** The user's pick: a key, "none", or absent for automatic. */
  pick?: string;
  offsetMs: number;
  /** The check against the song can run (setting on, speech model installed). */
  canCheck: boolean;
}

/**
 * plex: Plex's own (an agent such as LyricFind); file: a .lrc or .txt next to
 * the song; user: written in Platter; auto: transcribed on this computer and
 * not edited yet (may be wrong).
 */
export type LyricsSource = "plex" | "file" | "user" | "lrclib" | "auto";

/** `lyrics_own_get`: lyrics the user wrote for a song. */
export interface OwnLyrics {
  trackId: string;
  /** LRC text, or plain lines when not synced. */
  text: string;
  /** Shown ahead of Plex's lyrics for this song. */
  prefer: boolean;
  updatedAt: number;
  /** Transcribed on this computer by this model and not edited yet. Saving clears it. */
  auto?: TranscribeModel;
}

export interface LyricLine {
  startMs: number;
  text: string;
  /** Word timing; joined, the words make `text`. Absent when only lines are timed. */
  words?: LyricWord[];
}

export interface LyricWord {
  startMs: number;
  text: string;
}

export type LyricsMode = "autoShow" | "off";
export type DiscordStatusLine = "app" | "artist" | "song";
export type DiscordCoverHost = "auto" | "litterbox" | "x0" | "uguu" | "catbox";

/** `now_playing_details`: extras for the now-playing panel. */
export interface NowPlayingDetails {
  trackId: string;
  artist?: ArtistAbout | null;
  /** People credited beyond the track's artists. Often empty. */
  credits: Contributor[];
  lyrics?: Lyrics | null;
}

// ---------------------------------------------------------------- search

export type SearchFilter = "all" | "songs" | "artists" | "albums" | "playlists";

export interface SearchArgs {
  query: string;
  filter: SearchFilter;
  limit?: number;
}

export type SearchKind = "artist" | "album" | "track" | "playlist";

export interface SearchHit {
  kind: SearchKind;
  artist?: Artist | null;
  album?: Album | null;
  track?: Track | null;
  playlist?: Playlist | null;
}

export interface SearchResults {
  top?: SearchHit | null;
  songs: Track[];
  artists: Artist[];
  albums: Album[];
  playlists: Playlist[];
  tookMs: number;
  source: "local" | "server";
  /** Other versions of songs in `songs`, under the song's id. */
  versions?: Record<string, Track[]>;
  /** The query with a typo corrected, when the typo fallback found the results (a dropdown suggestion). */
  corrected?: string | null;
}

// ---------------------------------------------------------------- playback & queue

/** "liked" is Liked Songs (id "liked"). */
export type PlayContextKind = "album" | "playlist" | "artist" | "tracks" | "search" | "liked" | "mix" | "radio";

export interface PlayContext {
  kind: PlayContextKind;
  id?: string;
  trackIds?: string[];
}

export interface PlayContextArgs {
  context: PlayContext;
  startTrackId?: string;
  shuffle?: boolean;
}

export type RepeatMode = "off" | "all" | "one";
/** "spread" keeps an artist's songs apart; "standard" is plain random. */
export type ShuffleStyle = "spread" | "standard";
export type PlayerStatus = "idle" | "loading" | "playing" | "paused" | "buffering" | "error";

export interface QueueItem {
  itemId: string;
  track: Track;
  /** "autoplay": similar music after the context ran out; "smart": put in by smart shuffle. */
  source: "user" | "context" | "autoplay" | "smart";
}

export interface QueueContextInfo {
  kind: PlayContextKind;
  id?: string | null;
  title?: string | null;
}

export interface QueueView {
  current?: QueueItem | null;
  userQueue: QueueItem[];
  nextUp: QueueItem[];
  history: QueueItem[];
  context?: QueueContextInfo | null;
}

export interface PlayerState {
  status: PlayerStatus;
  track?: Track | null;
  positionMs: number;
  durationMs: number;
  volume: number;
  muted: boolean;
  /** Shuffle for the playing context. */
  shuffle: boolean;
  /** Context ids (albums, playlists, artists) whose shuffle preference is on. */
  shuffledContexts: string[];
  repeat: RepeatMode;
  queue: QueueView;
  /** The A-B loop on the playing track, if one is set. */
  abLoop?: AbLoop | null;
  /** Smart shuffle is on for the playing playlist or Liked Songs: suggestions mixed in. */
  smartShuffle?: boolean;
  /** Context ids (playlists, Liked Songs) whose smart shuffle preference is on. */
  smartContexts?: string[];
  /** Crossfade on for this playback only (a mix's smooth transitions). */
  smoothTransitions?: boolean;
}

/** An A-B loop: playback that reaches B goes back to A (ms, A before B). */
export interface AbLoop {
  aMs: number;
  bMs: number;
}

export interface PlayerPosition {
  positionMs: number;
  durationMs: number;
  bufferedMs: number;
}

export interface PlayerError {
  trackId?: string | null;
  kind: string;
  message: string;
}

// ---------------------------------------------------------------- health

export type HealthStatus =
  | "local"
  | "remote"
  | "relay"
  | "offline"
  | "starting"
  | "updating"
  | "noAuth";

/** Banner kinds the core sends (see healthBanner in MainView). */
export type HealthBannerKind =
  | "offline"
  | "noInternet"
  | "starting"
  | "updating"
  | "startTimeout"
  | "relay"
  | "signIn"
  | "accessRemoved"
  | "filesUnavailable"
  | "noMusicLibrary"
  | "diskFull"
  | "localNetwork"
  | "goneOffline";

/** What can still play without the server (`cached_library`). */
export interface CachedLibrary {
  /** Albums with a song on this computer, most recently played first. */
  albums: Album[];
  artistIds: string[];
  playlistIds: string[];
}

export interface HealthBanner {
  kind: HealthBannerKind | (string & {});
  message: string;
}

export interface Health {
  status: HealthStatus;
  serverName?: string | null;
  connection?: ConnectionKind | null;
  latencyMs?: number | null;
  serverVersion?: string | null;
  /** Unix time in milliseconds of the last successful ping. */
  lastSeenAt?: number | null;
  scan?: { done: number; total: number } | null;
  banner?: HealthBanner | null;
  filesAvailable: boolean;
  /** Only music already on this computer can play; the UI dims the rest. */
  cachedOnly?: boolean;
}

export interface ToastEvent {
  kind: "info" | "error";
  message: string;
}

// ---------------------------------------------------------------- settings

/** Streaming quality: the original file or a cap in kbps. */
export type StreamQuality = "original" | "320" | "256" | "192" | "128";
export type PopularSource = "global" | "myPlays";
export type ReduceMotion = "system" | "on" | "off";

/** The Theme setting: a built-in theme, or follow the computer's light or dark mode. */
export type ThemeSetting = "dark" | "light" | "system" | "midnight" | "forest" | "sand" | "studio" | "studioBlack" | "sleeve";

/** Record-player motion: full screen now playing only, the side panel too, or none. */
export type RecordMotion = "full" | "both" | "off";

/** A theme or look file that passed the checks (also what the `themeFile` setting holds). */
export type ThemeFile = UserTheme;
export type { Layout };

/** The themes folder: the theme files in it, and the ones that can't be used. */
export interface ThemeFolder {
  folder: string;
  themes: ThemeFile[];
  invalid: { file: string; reason: string }[];
}
/** Which Plex star ratings count as liked ("five": rating 10; "fourUp": 8 and up). */
export type LikedMeans = "five" | "fourUp";
/** When track rows show the like control. */
export type LikeControlRows = "hover" | "always" | "off";

export interface Settings {
  privateSession: boolean;
  /** Report plays to each song's server (was reportToPlex). */
  reportPlays: boolean;
  volume: number;
  shuffleStyle: ShuffleStyle;
  qualityHome: StreamQuality;
  qualityAway: StreamQuality;
  showFormatBadge: boolean;
  /** The player bar's badge that opens the signal path. */
  signalPathBadge: boolean;
  resumeContexts: boolean;
  /** Upcoming tracks kept on disk, 1 to 50. */
  precacheCount: number;
  popularSource: PopularSource;
  /** Artwork cache size in MB. */
  artCacheMb: number;
  artworkColours: boolean;
  reduceMotion: ReduceMotion;
  /** Chosen output device id; null follows the system default. Read only:
   *  change it with `outputSelect`. */
  outputDevice?: string | null;
  /** Home shows the grid of recent and most played shortcuts. */
  homeShortcuts: boolean;
  /** Home's top takes a colour wash from the shortcuts' artwork (while artworkColours is on). */
  homeColourWash: boolean;
  /** Completions and direct results under the search box while typing. */
  searchSuggestions: boolean;
  /** Search forgives typos when the words as typed find little. */
  searchTypos: boolean;
  /** Remember items opened from search (shown when the empty box is focused). */
  searchRecents: boolean;
  /** Media keys work while Platter is in the background. This device only. */
  mediaKeys: boolean;
  /** Show in the system media controls (media flyout, lock screen, Now Playing). This device only. */
  mediaSession: boolean;
  /** Closing the window hides it to the tray / menu bar. This device only. */
  closeToTray: boolean;
  /** Start Platter when you sign in to the computer. This device only. */
  launchAtLogin: boolean;
  /** Right-click or Alt+click on the seek bar sets A-B loop points. */
  abLoopHandles: boolean;
  /** Near a track's end the player bar shows what plays next. */
  upNextLine: boolean;
  /** Show the volume percentage while it changes. */
  volumePercent: boolean;
  /** Desktop notification on each new track (this device only). */
  notifyTrackChange: boolean;
  /** Ask before adding a song that's already in the playlist. */
  playlistAskDuplicates: boolean;
  /** Name for new playlists; "#N" becomes the next free number. */
  playlistNewName: string;
  /** Artist info from Wikipedia and Wikidata. */
  infoWikipedia: boolean;
  /** Artist info from Last.fm (needs a key, kept in the keychain). */
  infoLastfm: boolean;
  /** Artist images from Fanart.tv (needs a key). */
  infoFanart: boolean;
  /** Artist photo and bio from TheAudioDB (needs a key). */
  infoAudiodb: boolean;
  /** Popular songs from ListenBrainz (a token is optional). */
  infoListenbrainz: boolean;
  /** MusicBrainz: album dates and credits, members with years, related artists. */
  infoMusicbrainz: boolean;
  /** ListenBrainz listener numbers when Last.fm has none. */
  infoListenbrainzStats: boolean;
  /** Every name in a track's artist line links to a page, featured artists included. */
  linkFeatured: boolean;
  /** Which star ratings count as liked. Liking always writes five stars. */
  likedMeans: LikedMeans;
  /** When track rows show the like control (liked songs always show it unless "off"). */
  likeControlRows: LikeControlRows;
  /** List Plex players in the device picker. */
  showPlexPlayers: boolean;
  /** Answer as a Plex player so other Plex apps can play here. This device only. */
  actAsPlexPlayer: boolean;
  /** Moving playback stops it on the device it leaves. */
  moveStopsOldDevice: boolean;
  /** How one song moves into the next. */
  crossfade: CrossfadeMode;
  /** Length of a fixed crossfade, 1 to 12 s. */
  crossfadeSeconds: number;
  /** Songs of one album in album order never crossfade. */
  crossfadeSkipAlbums: boolean;
  /** DJ-style transitions where Automix applies. */
  automix: boolean;
  /** Where Automix plays transitions. */
  automixWhere: AutomixWhere;
  /** Longer, gentler blends or shorter ones on the bar. */
  automixStyle: AutomixStyle;
  /** Loudness leveling with Plex's gain values. */
  leveling: Leveling;
  /** dB added to every track, -12 to 12. This device only. */
  preampDb: number;
  /** Soft limiter so boosts never clip. This device only. */
  limiter: boolean;
  /** The equalizer (this device only, like the rest of the EQ). */
  eqEnabled: boolean;
  /** Active curve in dB at EQ_FREQS. Setting it makes the preset "custom". */
  eqBands: number[];
  /** Preset id, "profile:<name>" or "custom". Setting it also sets eqBands. */
  eqPreset: string;
  /** Saved curves. Read only: use the eqProfile* calls. */
  eqProfiles: EqProfile[];
  /** Presets or profiles bound to output devices. Read only: use eqUseForOutput. */
  eqOutputs: EqOutput[];
  /** The choice for output devices without their own. Read only. */
  eqFallback?: { preset: string; bands: number[] } | null;
  /** The output device the EQ last followed. Read only. */
  eqLastOutput?: string | null;
  /** Headphone correction per output device. Read only: use headphoneCorrectionSet. */
  headphoneCorrection: DeviceCorrection[];
  /** Output devices that play bit-perfect. Read only: use bitPerfectSet. */
  bitPerfectOutputs: BitPerfectOutput[];
  /** Share menus find public links for released music (iTunes Search and song.link). */
  sharePublicLinks: boolean;
  /** Share cards include a QR code with the Platter link. */
  shareCardQr: boolean;
  /** Share cards for released music include the song.link line. */
  shareCardSongLink: boolean;
  /** List Cast devices and AirPlay receivers under Listen on (mDNS stays off when off). */
  showCastDevices: boolean;
  /** Show what's playing on Discord (this device only). */
  discordStatus: boolean;
  /** Unreleased songs show on Discord (off: nothing while one plays). */
  discordShowUnreleased: boolean;
  /** Unreleased songs show their cover on Discord (off: the app icon). */
  discordUnreleasedArt: boolean;
  /** Pausing clears the Discord status (off: it shows as paused). */
  discordClearOnPause: boolean;
  /** A Find this song button (song.link) for released songs. */
  discordFindSong: boolean;
  /** Covers from Cover Art Archive, by MusicBrainz id or a MusicBrainz search (released albums). */
  discordCoverArchive: boolean;
  /** Covers uploaded to Imgur with the user's client ID. */
  discordUploadArt: boolean;
  /** Covers Cover Art Archive lacks (mostly unreleased songs) uploaded to keyless image hosts. */
  discordUploadCovers: boolean;
  /** Which keyless host: automatic (in order) or one. */
  discordCoverHost: DiscordCoverHost;
  /** What Discord's member list shows after "Listening to". */
  discordStatusLine: DiscordStatusLine;
  /** The top bar shows the Plex profile picture. This device only. */
  showPlexAvatar: boolean;
  /** Libraries included besides the main one, as `<machine id>/<section id>`. This device only. */
  libraries: string[];
  /** What the library shows with more than one server: "all" or a server tag. This device only. */
  serverView: string;
  /** The lyrics view opens by itself for songs with synced lyrics. */
  lyricsMode: LyricsMode;
  /** Look lyrics up on LRCLIB when Plex has none. */
  lyricsLrclib: boolean;
  /** The user's own lyrics show ahead of Plex's for every song. */
  lyricsPreferOwn: boolean;
  /** Songs without lyrics offer "Add lyrics" in the lyrics view. */
  lyricsShowAdd: boolean;
  /** Live versions keep the timing of lyrics found by artist and title. */
  lyricsLiveTiming: boolean;
  /** The full-window view hides its controls after 3 s without movement. */
  fullWindowHideControls: boolean;
  /** Read explicit, version and edition tags from the files (this device only). */
  readFileTags: boolean;
  /** Where this computer sees the server's music files. "" until set. This device only. */
  musicFolder: string;
  /** The start of Plex's file paths that stands for the music folder. This device only. */
  musicFolderServer: string;
  /** Ask Plex to rescan a folder after its tags are saved. */
  rescanAfterTagEdit: boolean;
  /** Ask before saving tags to more than 50 files. */
  askBeforeManyFiles: boolean;
  /** Explicit songs play, or are left out of what plays. */
  explicitContent: ExplicitContent;
  /** Show fan and AI edits. */
  showFanEdits: boolean;
  /** Albums Plex couldn't match count as unreleased until they're tagged. */
  countUnmatchedUnreleased: boolean;
  /** Look albums Plex couldn't match up on MusicBrainz (needs MusicBrainz on). */
  checkUnmatchedMusicbrainz: boolean;
  /** Albums sharing a group tag show as one card with an edition picker. */
  groupEditions: boolean;
  /** Edit in Plex (covers, matches, details, merge) on servers this account owns. */
  plexEditShow: boolean;
  /** Fields edited in Plex are locked so a refresh doesn't undo them. */
  plexEditLock: boolean;
  /** Fix match shows on unreleased albums (always with a warning). */
  plexEditMatchUnreleased: boolean;
  /** Drag songs, albums and playlists onto playlists, Liked Songs and the queue. */
  dragAndDrop: boolean;
  /** Transcribe songs without lyrics on this computer (this device only). */
  transcribeLyrics: boolean;
  /** The speech model transcription uses (this device only). Null: the default for this computer (Turbo on a graphics card, else Small). */
  transcribeModel: TranscribeModel | null;
  /** Speech-model work on the graphics card when there's a usable one (this device only). */
  transcribeGpu: TranscribeGpu;
  /** How hard speech-model work may use the processor (this device only). */
  transcribeProcessor: TranscribeProcessor;
  /** Background speech work waits while the computer is busy (this device only). */
  transcribePauseBusy: boolean;
  /** Background speech work uses more threads while nobody is at the computer and it's plugged in (this device only). */
  transcribeFasterAway: boolean;
  /** Show lyrics transcribed on this computer. */
  showAutoLyrics: boolean;
  /** Transcriptions travel to your other devices through the sync file. */
  syncTranscriptions: boolean;
  /** Offer to share a checked transcription to LRCLIB (asks every time). */
  shareTranscriptions: boolean;
  /** ...including unreleased songs (with an extra warning). */
  shareTranscriptionsUnreleased: boolean;
  /** Check lyrics against the song with the speech model, once it's installed. This device only. */
  lyricsCheckAudio: boolean;
  /** Move or stretch checked lyrics to the timing measured in the song. This device only. */
  lyricsFixTiming: boolean;
  /** Unreleased songs, alternate versions and live takes take the released version's lyrics without asking. */
  lyricsReleasedForVersions: boolean;
  /** Before transcribing a song, look for official synced lyrics and check them against it; a match is shown instead. */
  lyricsOfficialFirst: boolean;
  /** Announce this computer to your other Platter devices and let them play here. This device only. */
  showToMyDevices: boolean;
  /** The name your other devices see; empty uses the computer's name. This device only. */
  computerName: string;
  /** List AirPlay devices that need a password or pairing. */
  showAirplayLocked: boolean;
  /** Downloads: the file as it is, or an MP3. This device only. */
  downloadQuality: DownloadQuality;
  /** Downloads stop at this size, in GB. This device only. */
  downloadLimitGb: number;
  /** Where downloads go; empty is the app's own folder. Changed with downloadsChooseFolder. */
  downloadFolder: string;
  /** Download only over the local connection to your server. This device only. */
  downloadHomeOnly: boolean;
  /** Keep Liked Songs downloaded; new likes download too. This device only. */
  downloadLiked: boolean;
  /** No network use at all; only music on this computer plays. This device only. */
  goOffline: boolean;
  /** Sync to a device: what songs become on a device without its own format. Synced. */
  deviceFormat: DeviceFormat;
  /** Sync to a device: remove Platter's own files for songs no longer chosen. Synced. */
  deviceDeleteRemoved: boolean;
  /** Home shows the made-for-you and mood and genre mix shelves. */
  homeMixes: boolean;
  /** Home's On this day shelf: what you played on this date in earlier years. */
  homeOnThisDay: boolean;
  /** Home's Time travel shelf: a mix per decade and your year. */
  homeTimeTravel: boolean;
  /** In the Studio looks, Home leads with records: the album you were in the middle of, then shelves of albums. */
  homeRecordsFirst: boolean;
  /** How much of a mix is favourites. */
  mixVariety: MixVariety;
  /** Artist ids left out of mixes and radio. */
  mixExcludeArtists: string[];
  /** Genre, style or mood names left out of mixes and radio. */
  mixExcludeGenres: string[];
  /** Album ids left out of mixes and radio. */
  mixExcludeAlbums: string[];
  /** Unreleased music can be in mixes. */
  mixUnreleased: boolean;
  /** Hours between daylist refreshes: 1, 3 or 6. */
  daylistHours: number;
  /** When the queue ends: keep playing similar music, or stop. */
  queueEnd: QueueEnd;
  /** The shuffle button has a third state, smart shuffle. */
  smartShuffle: boolean;
  /** The top bar's friend activity button and panel. */
  showFriendActivity: boolean;
  /** Tell Plex what's playing now, so friends on the server see it live. */
  shareActivity: boolean;
  /** The bell's dot for unseen releases on What's New. */
  whatsNewAnnouncements: boolean;
  /** A desktop notification when a followed artist gets new music. */
  notifyNewReleases: boolean;
  /** "Let others change what's playing": guests can play, pause, skip, seek and reorder. Off, they can only add songs. */
  listenGuestsControl: boolean;
  /** The listen along relay (an ntfy server); empty means ntfy.sh. */
  listenRelay: string;
  /** "Invites expire after", hours: 1, 24 or 168. */
  listenInviteHours: number;
  /** The name others in a listening session see; empty means the Plex display name. */
  listenName: string;
  /** In a session you host, unreleased songs go to guests without their title and artist. */
  listenHideUnreleased: boolean;
  /** Collaborative playlists: invite people to your playlists and join theirs (needs an invite). */
  collabPlaylists: boolean;
  /** Blends: playlists made from friends' tastes (needs an invite). */
  blends: boolean;
  /** Unreleased songs go into shared playlists and Blend taste summaries. */
  shareUnreleased: boolean;
  /** Offer "Let them listen from my server" to a guest who can't hear the song (host only). */
  shareOfferServer: boolean;
  /** "Share unreleased music": the rule; choosing it sets each feature's own unreleased switch. */
  unreleasedSharing: UnreleasedSharing;
  /** "Libraries I share by default": section ids on your server, music libraries only. None chosen at first. */
  shareDefaultLibraries: string[];
  /** "Hide my unreleased music from people I share with" (a Plex label the share leaves out; needs Plex Pass). */
  shareHideUnreleased: boolean;
  /** "Allow downloads for people I share with". */
  shareAllowDownloads: boolean;
  /** "Accept invites inside Platter": a friend's server invite shows here, one click to accept. */
  shareAcceptInvites: boolean;
  /** Discord shows a listening session you host. */
  discordListenAlong: boolean;
  /** Keep app-only state in a sync file in a folder. This device only. */
  syncEnabled: boolean;
  /** The folder the sync file goes in. Changed with syncFileUseFolder / syncFileChooseFolder. This device only. */
  syncFolder: string;
  /** What to sync: settings marked Synced. This device only. */
  syncSettings: boolean;
  /** What to sync: library folders and pins. This device only. */
  syncLibrary: boolean;
  /** What to sync: followed artists and hidden items. This device only. */
  syncFollows: boolean;
  /** What to sync: lyrics you wrote. This device only. */
  syncLyrics: boolean;
  /** Bookmarks has a row in the library. */
  bookmarksInLibrary: boolean;
  showDlnaDevices: boolean;
  allowAutomation: boolean;
  translateTo: string;
  translationService: string;
  showPronunciation: boolean;
  pinsInLibrary: boolean;
  /** Shelf view: an album's spine is wider the more songs it has. This device only. */
  spineWidthByTracks: boolean;
  /** Stats, mixes and recaps count your play history on your Plex server. */
  includePlexHistory: boolean;
  /** The same for Tautulli's history, once Tautulli is set up. */
  includeTautulliHistory: boolean;
  /** The same for your Last.fm scrobbles, once Last.fm is connected. */
  includeLastfmHistory: boolean;
  /** Tautulli's address (empty: not set up). Its API key is in the keychain. */
  tautulliUrl: string;
  /** Offer a setup guide when a feature that has one is first used. */
  showSetupGuides: boolean;
  /** Show the "?" buttons that open help next to complex screens. */
  showHelpButtons: boolean;
  /** The Stats page (profile menu and library row). */
  statsPage: boolean;
  /** The stats strip on song, artist and album pages (and "Your stats" in menus). */
  statsStrip: boolean;
  /** Monthly recaps on the Stats page (saved as `monthlyStatsOnProfile` by earlier builds). */
  monthlyStats: boolean;
  /** Compare stats with friends: a summary only, never the history. */
  statsCompare: boolean;
  /** Wrapped and the yearly Top Songs lists. */
  wrappedTopSongs: boolean;
  /** "N plays" under titles on yearly Top Songs lists (All-Time always shows them). */
  topSongsShowPlays: boolean;
  /** Song lineage: "Lineage" in song menus, the versions panel and Your stats. */
  showLineage: boolean;
  /** "Describe a playlist" in the create menu and the smart playlist editor. */
  showDescribePlaylist: boolean;
  /** The sound map page and its entries (shown once songs are analysed). */
  showSoundMap: boolean;
  /** Compare plays both versions at matched loudness when the server has it. */
  compareLevelMatch: boolean;
  /** Scrobble to Last.fm (once signed in). */
  scrobbleLastfm: boolean;
  /** Scrobble to ListenBrainz (with the user token). */
  scrobbleListenbrainz: boolean;
  /** Unreleased music scrobbles too. */
  scrobbleUnreleased: boolean;
  /** The AI provider for prompted playlists. */
  aiProvider: AiProvider;
  /** The provider's model. */
  aiModel: string;
  /** Lidarr's home address (empty: Lidarr isn't set up). Its API key is in the keychain. */
  lidarrUrl: string;
  /** Lidarr's away address, tried when the home one doesn't answer (empty: none). */
  lidarrAwayUrl: string;
  /** Artist pages list official releases the library lacks, with Request (when Lidarr is set up and its check passed). */
  showMissingReleases: boolean;
  /** Lidarr's staging folder for upgrades, as Lidarr sees it (empty: upgrades are off). */
  lidarrStagingFolder: string;
  /** Upgrades ask before each swap (off swaps on their own, under the same rules). */
  upgradesAsk: boolean;
  /** Ask Plex to scan just the new folder when a Managed import doesn't show up within about two minutes. */
  lidarrScanAfterImport: boolean;
  /** Free a protected album's staging copy once it's in the library and unmonitored. */
  lidarrFreeStaging: boolean;
  /** Practice mode until (unix seconds); -1 the first week after setup; 0 off. */
  lidarrPracticeUntil: number;
  /** A provenance note starting with this marks an official source (upgrades without ISRCs need it). */
  upgradeOfficialPrefix: string;
  /** The miniplayer window stays above other windows (this device only). */
  miniplayerOnTop: boolean;
  /** The miniplayer's card takes a colour from the cover (this device only). */
  miniplayerTint: boolean;
  /** The miniplayer lists what's up next when it's tall enough (this device only). */
  miniplayerQueue: boolean;
  /** A wide miniplayer puts the title and queue beside the art (this device only). */
  miniplayerSideBySide: boolean;
  /** Where the miniplayer was last (logical pixels). Kept by the core; read only. */
  miniplayerBounds?: WindowBounds | null;
  /** Keyboard power mode: jump hints, j / k, g g, G and / in the main window. */
  powerKeys: boolean;
  /** The key that shows the jump hints. */
  hintKey: HintKey;
  /** Serve what's playing on 127.0.0.1 for widgets (this device only). */
  nowPlayingFeed: boolean;
  /** The feed's port, 1024 to 65535 (this device only). */
  nowPlayingFeedPort: number;
  /** Let a paired Stream Deck see what's playing and control it, on 127.0.0.1 (this device only). */
  allowStreamDeck: boolean;
  /** Library import sources turned on ("streaming", "transfer", "lists"). All off until picked. */
  importSources: string[];
  /** The Missing page lists songs and albums from imported listening history that aren't in the library. Synced. */
  showPlayedALot: boolean;
  /** Fewest plays a song or album needs to be on that list (1 to 1000). Synced. */
  playedMinPlays: number;
  /** Which plays that list counts. Synced. */
  playedPeriod: PlayedPeriod;
  /** "Bring in my listening": request albums from Played a lot in waves through Lidarr. Synced. */
  bringIn: boolean;
  /** The waves' fewest plays, highest first (1 to 5 of them). Synced. */
  bringInWaves: number[];
  /** Most albums asked for in a day. Synced. */
  bringInPerDay: number;
  /** Pause while less than this many GB are free where Lidarr downloads. Synced. */
  bringInMinFreeGb: number;
  /** Pause while Lidarr's queue holds more than this. Synced. */
  bringInMaxQueue: number;
  /** Folder names on a song's path that hint it's unreleased (they only add weight). */
  archiveFolderKeywords: string[];
  /** The tag a file's release status is read from: "" for Auto (ARCHIVE_VERSION, STATUS, RELEASESTATUS, RELEASETYPE), else a tag name. */
  releaseStatusTag: string;
  /** Custom release status values and what they mean, checked before the built-in ones. */
  releaseStatusValues: ReleaseValueMap[];
  /** Era cards on artist pages. */
  showEraCards: boolean;
  /** "+N versions" on song rows. */
  showVersionCounts: boolean;
  /** The library doctor checks the library in the background (a count shows in the app menu). */
  libraryDoctor: boolean;
  /** Which library doctor checks run. */
  doctorChecks: DoctorChecks;
  /** Covers smaller than this on their longest side count as small (300, 500, 600, 800, 1000 or 1400). */
  doctorCoverPx: number;
  /** Release radar: new releases from followed and most played artists, on Home and its own page. */
  releaseRadar: boolean;
  /** A notification when the release radar finds new releases. */
  radarNotify: boolean;
  /** Auto-request new albums for the artists it's on for (this device). */
  autoRequest: boolean;
  /** Most automatic requests a day: 1, 2, 3, 5 or 10 (this device). */
  autoRequestCap: number;
  /** Fingerprint songs on this computer and look them up on AcoustID (this device). */
  identifyBySound: boolean;
  /** Which songs: untagged only, or all (this device). */
  soundIdScope: SoundIdScope;
  /** Minutes between checks of a Subsonic server for library changes: 1, 5, 15, 30 or 60 (this device). */
  subsonicCheckMinutes: number;
  /** Keep the queue on servers that hold one, so another app can pick it up. */
  saveQueueOnServer: boolean;
  /** Search also shows music from MusicBrainz that isn't in the library. */
  catalogueSearch: boolean;
  /** "Fans also like" includes artists that aren't in the library. */
  fansAlsoLikeCatalogue: boolean;
  /** The "New to you" shelf on Home. */
  homeNewToYou: boolean;
  /** Artists marked "Not interested" (never suggested again). */
  dismissedArtists: DismissedArtist[];
  /** Where search finds music you don't have: Lidarr when it's set up, or MusicBrainz. */
  catalogueSource: "lidarr" | "musicbrainz";
  /** Get opens the request dialog every time. */
  askBeforeGet: boolean;
  /** The kinds "Get albums" offers ticked. */
  getAlbumsKinds: string[];
  /** The discography chip an artist page opens on: the last one picked (this device). */
  discographyChip: string;
  /** The full discography's order, whether it's reversed, and its view (this device). */
  discographySort: "date" | "name";
  discographyReverse: boolean;
  discographyView: "list" | "grid";
  /** Badges, the On the shelf filter and the Stats shelf for albums owned on vinyl, CD, cassette or another format. */
  showPhysicalCollection: boolean;
  /** "Missing from the shelf": albums played most that aren't owned. */
  shelfMissingList: boolean;
  /** A Sleeve notes tab on album pages and Credits in song menus. */
  showSleeveNotes: boolean;
  /** Sleeve notes look credits up on MusicBrainz (and Discogs with a token). */
  sleeveLookups: boolean;
  /** Album tracklists show a record's sides (from a vinyl edition, or split by hand). */
  showVinylSides: boolean;
  /** The record player pauses at the end of a side until Flip. */
  flipPause: boolean;
  /** A Crates page: the library by label, producer, studio, era and genre. */
  showCrates: boolean;
  /** Album pages show "More from <label>" and other crate shelves. */
  crateShelves: boolean;
  /** Keep a listening diary of albums played start to finish. */
  keepDiary: boolean;
  /** Albums played start to finish go in the diary on their own. */
  diaryAuto: boolean;
  /** The diary travels in the sync file (this device only; off by default). */
  syncDiary: boolean;
  /** Studio (the default), Studio Black, Sleeve, Classic or another built-in look. */
  theme: ThemeSetting;
  /** A preset id ("brass" is the default) or a custom colour as #RRGGBB. */
  accent: string;
  /** Swap the look for its light or dark partner with the computer's mode. */
  followSystem: boolean;
  /** The one-time "Try Platter's new look" offer was shown or not needed. */
  newLookOffered: boolean;
  /** Spinning record and tonearm: full screen only, the side panel too, or off. */
  recordMotion: RecordMotion;
  /** The seek bar is a needle on a groove line. */
  tonearmSeek: boolean;
  /** The theme file in use, carried whole so it travels in the sync file. */
  themeFile: ThemeFile | null;
  /**
   * Layout options. New installs start on Index (the core's default);
   * settings saved before frames load as Classic.
   */
  layout: Layout;
  /** The one-time "Try the Index layout" offer was shown or not needed. */
  layoutOffered: boolean;
  /** Appearance shows the "Make your own" look builder. */
  showLookBuilder: boolean;
  /** The "Pick a look" step was shown or skipped on this computer. */
  lookStepDone: boolean;
  [key: string]: unknown;
}

/** `feed_status` and the `feed://status` event: the now-playing feed for widgets. */
export type FeedStatus =
  | { state: "off" }
  | { state: "on"; port: number; url: string }
  | { state: "portInUse"; port: number }
  | { state: "failed"; port: number; message: string };

/** The Stream Deck control API: the port's state, the pairing code while it's on, and whether one is paired. */
export type DeckStatus = (
  | { state: "off" }
  | { state: "on"; port: number }
  | { state: "portInUse"; port: number }
  | { state: "failed"; port: number; message: string }
) & {
  /** The Stream Deck pairing code (while Allow Stream Deck is on). */
  code: string | null;
  paired: boolean;
  /** The automation pairing code (while Allow automation is on). */
  automationCode: string | null;
  automationPaired: boolean;
};

/** The Platter plugin for the Stream Deck app (`deckPluginStatus`). */
export type DeckPluginStatus =
  /** The Stream Deck app isn't on this computer. */
  | { state: "noApp" }
  /** This copy of Platter has no plugin inside (a development build). */
  | { state: "notBundled" }
  | { state: "install"; version: string }
  /** The same version, or a newer one, is installed. */
  | { state: "installed"; version: string; installed: string }
  /** An older version is installed. */
  | { state: "update"; version: string; installed: string };

/** What a Stream Deck key asked the window to show. */
export type DeckAction =
  | { kind: "listenAlong" }
  | { kind: "requestAlbum"; input: LidarrRequestInput }
  /** A command link or script asked for a page. */
  | { kind: "open"; route: import("../stores/router").Route }
  /** A command link didn't run (automation off, no such playlist...). */
  | { kind: "notice"; message: string }
  /** A Stream Deck key: pick a playlist for these songs. */
  | { kind: "addToPlaylist"; trackIds: string[] }
  /** A Stream Deck key: show Listen on, or move to this device (it needs the window). */
  | { kind: "listenOn"; device?: string }
  /** A Stream Deck key: open or close the lyrics. */
  | { kind: "lyrics"; open: boolean }
  /** A Stream Deck key: the expanded now playing view, or the queue. */
  | { kind: "nowPlaying" }
  | { kind: "queue" };

/** The feed's default port. */
export const FEED_DEFAULT_PORT = 32540;

/** Keys that can show the jump hints in keyboard power mode. */
export type HintKey = "f" | "h" | ";" | "'";

/** A window's place and size in logical pixels. */
export interface WindowBounds {
  x: number;
  y: number;
  width: number;
  height: number;
}

/** A share card's QR code: rows of modules ('1' dark) and the link it holds. */
export interface CardQr {
  size: number;
  rows: string[];
  text: string;
}

/** Whether the miniplayer window is open. */
export interface MiniplayerState {
  open: boolean;
}

/** A page the miniplayer asks the main window to open (`miniplayer_go`). */
export interface MiniplayerGo {
  kind: "album" | "artist";
  id: string;
}

export type DownloadQuality = "original" | "320" | "192" | "128";

export type MixVariety = "balanced" | "familiar" | "adventurous";
export type QueueEnd = "keepPlaying" | "stop";

export type ExplicitContent = "allowed" | "filtered";

export type CrossfadeMode = "smart" | "fixed" | "off";
export type AutomixWhere = "playlistsAndMixes" | "allButAlbums";
export type AutomixStyle = "smooth" | "energetic";

/** One transition's style on a mixed playlist ("auto": Platter picks). */
export type MixStyle = "auto" | "none" | "blend" | "lift" | "drop" | "echo";

/** A playlist with Mix on, and the transitions picked for its gaps ("<fromId>><toId>"). */
export interface MixedPlaylist {
  styles: Record<string, MixStyle>;
}

/** How far along the sound map is (`soundmap_info`). */
export interface SoundMapInfo {
  /** Songs on the map. */
  placed: number;
  /** Songs with an analysis. */
  analysed: number;
  /** Songs that can be analysed. */
  total: number;
  /** Song analysis is running (Automix on), so more songs will come. */
  analysing: boolean;
  /** The map is being updated right now. */
  building: boolean;
  /** Goes up each time the map changes; the same number means the loaded points are current. */
  revision: number;
}

/** Part of the map, in map units (most songs sit within -1 to 1). */
export interface SoundMapArea {
  minX: number;
  minY: number;
  maxX: number;
  maxY: number;
}

/**
 * Points in columns, one entry per song in each (`soundmap_points`).
 * `genre` indexes `genres` (-1: none); `year` 0 is unknown.
 */
export interface SoundMapChunk {
  ids: string[];
  x: number[];
  y: number[];
  genre: number[];
  genres: string[];
  year: number[];
  unreleased: boolean[];
  /** Pass back as `after` for the next chunk; null after the last. */
  next: string | null;
}

/** A song or an artist to fly to (`soundmap_find`). */
export interface SoundMapFound {
  kind: "song" | "artist";
  id: string;
  title: string;
  /** The artist for a song, "N songs" for an artist. */
  sub: string;
  x: number;
  y: number;
}

/** The hover card (`soundmap_song`). */
export interface SoundMapSong {
  id: string;
  title: string;
  artist: string;
  albumTitle: string;
  thumb: string | null;
}

/** Mixed playlists by id (`automix_mixed_get`). */
export type MixedPlaylists = Record<string, MixedPlaylist>;

/** Tempo and transitions for songs in the order shown (`automix_view`). */
export interface AutomixView {
  songs: { id: string; bpm?: number | null; key?: string | null }[];
  gaps: { fromId: string; toId: string; reason?: string | null; style: MixStyle; beatmatched: boolean }[];
}
export type Leveling = "smart" | "track" | "album" | "off";
export interface EqProfile {
  name: string;
  bands: number[];
}
export interface EqOutput {
  deviceId: string;
  deviceName: string;
  preset: string;
}
/** EQ band centres in Hz. */
export const EQ_FREQS = [31, 62, 125, 250, 500, 1000, 2000, 4000, 8000, 16000] as const;
/** Built-in EQ presets (the same values as the core). */
export const EQ_PRESETS: { id: string; name: string; bands: number[] }[] = [
  { id: "flat", name: "Flat", bands: [0, 0, 0, 0, 0, 0, 0, 0, 0, 0] },
  { id: "bassBoost", name: "Bass boost", bands: [6, 5, 4, 2, 0.5, 0, 0, 0, 0, 0] },
  { id: "trebleBoost", name: "Treble boost", bands: [0, 0, 0, 0, 0, 0.5, 2, 4, 5, 6] },
  { id: "vocal", name: "Vocal", bands: [-2, -2, -1, 0, 2, 3.5, 3.5, 2, 0, -1] },
  { id: "loudness", name: "Loudness", bands: [5, 4, 2, 0, -1, -1, 0, 1.5, 3.5, 4.5] },
  { id: "smallSpeakers", name: "Small speakers", bands: [-8, -4, 2, 3, 1.5, 0, 0, 1, 2, 1] },
  { id: "lateNight", name: "Late night", bands: [-5, -4, -2, -0.5, 0, 1, 1.5, 1, 0, -1] },
];

/** What importing a settings file changed. */
export interface SettingsImportReport {
  /** How many settings the file set. */
  applied: number;
  /** Settings in the file this version can't use. */
  skipped: string[];
}

/** "Share unreleased music": Never, Ask each time or Always. */
export type UnreleasedSharing = "never" | "ask" | "always";

export const DEFAULT_SETTINGS: Settings = {
  privateSession: false,
  reportPlays: true,
  volume: 0.7,
  shuffleStyle: "spread",
  qualityHome: "original",
  qualityAway: "320",
  showFormatBadge: true,
  signalPathBadge: true,
  resumeContexts: true,
  precacheCount: 5,
  popularSource: "global",
  artCacheMb: 2048,
  artworkColours: true,
  reduceMotion: "system",
  outputDevice: null,
  homeShortcuts: true,
  homeColourWash: true,
  searchSuggestions: true,
  searchTypos: true,
  searchRecents: true,
  mediaKeys: true,
  mediaSession: true,
  closeToTray: false,
  launchAtLogin: false,
  abLoopHandles: true,
  upNextLine: true,
  volumePercent: true,
  notifyTrackChange: false,
  playlistAskDuplicates: true,
  playlistNewName: "My playlist #N",
  infoWikipedia: false,
  infoLastfm: false,
  infoFanart: false,
  infoAudiodb: false,
  infoListenbrainz: false,
  infoMusicbrainz: false,
  infoListenbrainzStats: false,
  linkFeatured: true,
  likedMeans: "five",
  likeControlRows: "hover",
  showPlexPlayers: true,
  actAsPlexPlayer: false,
  moveStopsOldDevice: true,
  crossfade: "smart",
  crossfadeSeconds: 6,
  crossfadeSkipAlbums: true,
  automix: true,
  automixWhere: "playlistsAndMixes",
  automixStyle: "smooth",
  leveling: "smart",
  preampDb: 0,
  limiter: true,
  eqEnabled: false,
  eqBands: [0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
  eqPreset: "flat",
  eqProfiles: [],
  eqOutputs: [],
  eqFallback: null,
  eqLastOutput: null,
  headphoneCorrection: [],
  bitPerfectOutputs: [],
  sharePublicLinks: false,
  shareCardQr: true,
  shareCardSongLink: true,
  showCastDevices: true,
  discordStatus: false,
  discordShowUnreleased: true,
  discordUnreleasedArt: true,
  discordClearOnPause: true,
  discordFindSong: true,
  discordCoverArchive: true,
  discordUploadArt: false,
  discordUploadCovers: false,
  discordCoverHost: "auto",
  discordStatusLine: "app",
  showPlexAvatar: true,
  libraries: [],
  serverView: "all",
  lyricsMode: "autoShow",
  lyricsLrclib: false,
  lyricsPreferOwn: false,
  lyricsShowAdd: true,
  lyricsLiveTiming: false,
  fullWindowHideControls: true,
  readFileTags: true,
  musicFolder: "",
  musicFolderServer: "",
  rescanAfterTagEdit: true,
  askBeforeManyFiles: true,
  explicitContent: "allowed",
  showFanEdits: false,
  countUnmatchedUnreleased: true,
  checkUnmatchedMusicbrainz: false,
  groupEditions: true,
  plexEditShow: true,
  plexEditLock: true,
  plexEditMatchUnreleased: true,
  dragAndDrop: true,
  transcribeLyrics: false,
  transcribeModel: null,
  transcribeGpu: "auto",
  transcribeProcessor: "light",
  transcribePauseBusy: true,
  transcribeFasterAway: true,
  showAutoLyrics: true,
  syncTranscriptions: true,
  shareTranscriptions: true,
  shareTranscriptionsUnreleased: false,
  lyricsCheckAudio: true,
  lyricsFixTiming: true,
  lyricsReleasedForVersions: false,
  lyricsOfficialFirst: true,
  showToMyDevices: false,
  computerName: "",
  showAirplayLocked: false,
  downloadQuality: "original",
  downloadLimitGb: 20,
  downloadFolder: "",
  downloadHomeOnly: true,
  downloadLiked: false,
  goOffline: false,
  deviceFormat: "original",
  deviceDeleteRemoved: false,
  homeMixes: true,
  homeOnThisDay: true,
  homeTimeTravel: true,
  homeRecordsFirst: true,
  mixVariety: "balanced",
  mixExcludeArtists: [],
  mixExcludeGenres: [],
  mixExcludeAlbums: [],
  mixUnreleased: true,
  daylistHours: 3,
  queueEnd: "keepPlaying",
  smartShuffle: true,
  showFriendActivity: true,
  shareActivity: false,
  whatsNewAnnouncements: true,
  notifyNewReleases: false,
  listenGuestsControl: true,
  listenRelay: "",
  listenInviteHours: 24,
  listenName: "",
  listenHideUnreleased: false,
  collabPlaylists: true,
  blends: true,
  shareUnreleased: false,
  shareOfferServer: true,
  unreleasedSharing: "ask",
  shareDefaultLibraries: [],
  shareHideUnreleased: true,
  shareAllowDownloads: false,
  shareAcceptInvites: true,
  discordListenAlong: true,
  syncEnabled: false,
  syncFolder: "",
  syncSettings: true,
  syncLibrary: true,
  syncFollows: true,
  syncLyrics: true,
  bookmarksInLibrary: true,
  showDlnaDevices: true,
  allowAutomation: false,
  translateTo: "en",
  translationService: "",
  showPronunciation: false,
  pinsInLibrary: true,
  spineWidthByTracks: true,
  includePlexHistory: true,
  includeTautulliHistory: true,
  includeLastfmHistory: true,
  tautulliUrl: "",
  showSetupGuides: true,
  showHelpButtons: true,
  statsPage: true,
  statsStrip: true,
  monthlyStats: true,
  statsCompare: false,
  wrappedTopSongs: true,
  topSongsShowPlays: false,
  showLineage: true,
  showDescribePlaylist: true,
  showSoundMap: true,
  compareLevelMatch: true,
  scrobbleLastfm: false,
  scrobbleListenbrainz: false,
  scrobbleUnreleased: false,
  aiProvider: "none",
  aiModel: "claude-opus-5-5",
  lidarrUrl: "",
  lidarrAwayUrl: "",
  showMissingReleases: true,
  lidarrStagingFolder: "",
  upgradesAsk: true,
  lidarrScanAfterImport: true,
  lidarrFreeStaging: true,
  lidarrPracticeUntil: -1,
  upgradeOfficialPrefix: "Official",
  miniplayerOnTop: true,
  miniplayerTint: true,
  miniplayerQueue: false,
  miniplayerSideBySide: true,
  miniplayerBounds: null,
  powerKeys: false,
  hintKey: "f",
  nowPlayingFeed: false,
  nowPlayingFeedPort: 32540,
  allowStreamDeck: false,
  importSources: [],
  showPlayedALot: true,
  playedMinPlays: 5,
  playedPeriod: "all",
  bringIn: false,
  bringInWaves: [100, 50, 20],
  bringInPerDay: 30,
  bringInMinFreeGb: 50,
  bringInMaxQueue: 20,
  archiveFolderKeywords: ["Unreleased", "Leaks", "Demos", "Sessions", "Snippets"],
  releaseStatusTag: "",
  releaseStatusValues: [],
  showEraCards: true,
  showVersionCounts: true,
  libraryDoctor: true,
  doctorChecks: {
    duplicates: true,
    covers: true,
    albumTags: true,
    splitAlbums: true,
    albumArtist: true,
    lowQuality: true,
    unmatched: true,
    soundsLike: true,
  },
  doctorCoverPx: 600,
  releaseRadar: false,
  radarNotify: true,
  autoRequest: false,
  autoRequestCap: 3,
  identifyBySound: false,
  soundIdScope: "untagged",
  subsonicCheckMinutes: 5,
  saveQueueOnServer: false,
  catalogueSearch: true,
  fansAlsoLikeCatalogue: false,
  homeNewToYou: false,
  dismissedArtists: [],
  catalogueSource: "lidarr",
  askBeforeGet: false,
  getAlbumsKinds: ["studio"],
  discographyChip: "popular",
  discographySort: "date",
  discographyReverse: false,
  discographyView: "list",
  showPhysicalCollection: true,
  shelfMissingList: false,
  showSleeveNotes: true,
  sleeveLookups: true,
  showVinylSides: true,
  flipPause: false,
  showCrates: true,
  crateShelves: true,
  keepDiary: true,
  diaryAuto: true,
  syncDiary: false,
  theme: "studio",
  accent: "brass",
  followSystem: true,
  newLookOffered: true,
  recordMotion: "full",
  tonearmSeek: true,
  themeFile: null,
  // The mock starts where an existing install is (Classic), so the shell's
  // tests cover the layout people have used all along; the first-run step
  // and the core start new installs on Index.
  layout: CLASSIC_LAYOUT,
  layoutOffered: true,
  showLookBuilder: true,
  lookStepDone: false,
};

// ---------------------------------------------------------------- sync file, folders and keys

/** The sync file's state (docs/IPC.md "Sync, folders and keys"). */
export type SyncFileState = "off" | "signedOut" | "ok" | "folderMissing" | "unreadable" | "newer" | "error";

export interface SyncFileStatus {
  state: SyncFileState;
  folder: string | null;
  /** The file's full path. */
  file: string | null;
  /** ms since the epoch. */
  lastSyncedAt: number | null;
  /** Devices that write the file, this one first. */
  devices: { name: string; lastSeen: number; thisDevice: boolean }[];
  /** Conflicted copies merged in by the last pass. */
  mergedCopies: number;
  /** Plain words for a problem. */
  message: string | null;
}

/** A cloud folder found on this computer. */
export interface CloudFolder {
  provider: string;
  path: string;
}

/** A pin as the core keeps it. */
export interface PinRecord {
  kind: "album" | "artist" | "playlist" | "folder";
  id: string;
  title: string;
  subtitle: string;
  thumb: string | null;
  /** ms; newest first. */
  pinnedAt?: number;
}

export interface LibraryFolder {
  id: string;
  name: string;
  /** The folder it's in; null at the top of the library. */
  parent: string | null;
  createdAt: number;
}

export interface LibraryFolders {
  /** Newest first. */
  folders: LibraryFolder[];
  /** Playlist id → folder id. */
  items: Record<string, string>;
}

/** Keys that can be copied between devices. */
export type KeyKind = "listenbrainz" | "lastfm" | "fanart" | "audiodb" | "imgur" | "catbox";

export type ReceivePhase = "idle" | "waiting" | "receiving" | "received" | "expired" | "locked";

export interface KeyReceiveState {
  phase: ReceivePhase;
  code: string | null;
  /** The code as a QR code: rows of "0" and "1" (dark). */
  qr: { size: number; rows: string[] } | null;
  expiresAt: number | null;
  /** The sending device's name. */
  from: string | null;
  received: KeyKind[];
}

export interface KeySendOutcome {
  device: string;
  sent: KeyKind[];
}

/** What changed in the sync store (namespaces: settings, pins, folders, folderItems, follows, hidden, bookmarks, lyrics). */
export interface SyncStoreChanged {
  namespaces: string[];
  /** From the sync file (another device). */
  remote: boolean;
}

// ---------------------------------------------------------------- downloads

/** What can be downloaded as a whole (docs/IPC.md "Downloads"). */
export type DownloadKind = "album" | "playlist" | "liked" | "mix";
export type DownloadItemState = "queued" | "downloading" | "downloaded" | "error";
/** Why the download queue is waiting. */
export type DownloadPause = "offline" | "notHome" | "full" | "noSpace" | "folderMissing";

export interface DownloadItem {
  kind: DownloadKind;
  /** Empty for Liked Songs. */
  id: string;
  title: string;
  state: DownloadItemState;
  total: number;
  done: number;
  /** Songs that failed after every retry. */
  failed: number;
  /** Finished songs whose file has gone from disk. */
  missing: number;
  /** 0 to 1, counting the song in flight. */
  progress: number;
  /** Bytes removing it would free. */
  bytes: number;
  error?: string | null;
}

export interface DownloadStatus {
  items: DownloadItem[];
  usedBytes: number;
  limitBytes: number;
  freeBytes?: number | null;
  folder: string;
  paused?: DownloadPause | null;
  /** Songs waiting to be fetched. */
  queued: number;
  current?: { trackId: string; received: number; total?: number | null } | null;
}

// ---------------------------------------------------------------- devices

/** What songs become on a device (docs/IPC.md "Devices"). */
export type DeviceFormat = "original" | "flac" | "mp3-320" | "mp3-v0" | "opus";

export type DeviceItemKind = "playlist" | "album" | "artist" | "liked";

export interface DeviceItem {
  kind: DeviceItemKind;
  /** Empty for Liked Songs. */
  id: string;
  title: string;
}

/** A device remembered on this computer. */
export interface Device {
  id: string;
  name: string;
  volumeId: string | null;
  volumeLabel: string;
  /** The music folder below the drive's top, "/" separated ("" is the top). */
  folder: string;
  /** null follows the Default format setting. */
  format: DeviceFormat | null;
  budgetGb: number | null;
  selection: DeviceItem[];
  lastSync: number | null;
  lastRoot: string | null;
}

export interface DeviceView extends Device {
  /** Where it's plugged in now, or null. */
  root: string | null;
  free: number | null;
  total: number | null;
}

/** A mounted drive. */
export interface DriveVolume {
  root: string;
  label: string;
  id: string | null;
  removable: boolean;
  free: number | null;
  total: number | null;
}

export type DeviceSyncPhase = "planning" | "copying" | "waiting" | "paused" | "done" | "cancelled" | "failed";

export interface DeviceSyncStatus {
  deviceId: string;
  deviceName: string;
  phase: DeviceSyncPhase;
  total: number;
  done: number;
  kept: number;
  removed: number;
  current: string | null;
  bytes: number;
  failed: { title: string; error: string }[];
  leftOut: { title: string; songs: number }[];
  leftOutSongs: number;
  error: { kind: "full" | "writeProtected" | "accessDenied" | "removed" | "notConnected" | "other"; message: string } | null;
  /** Something to know that didn't stop the sync. */
  notice: string | null;
  safeToUnplug: boolean;
}

export interface DevicesView {
  devices: DeviceView[];
  /** Removable drives that aren't a device yet. */
  drives: DriveVolume[];
  sync: DeviceSyncStatus | null;
}

export interface DevicePatch {
  name?: string;
  folder?: string;
  /** A format, or "default" to follow the Default format setting. */
  format?: DeviceFormat | "default";
  /** 0 removes the limit. */
  budgetGb?: number;
}

// ---------------------------------------------------------------- mixes

export type MixKind = "daily" | "discover" | "newInLibrary" | "onRepeat" | "rediscover" | "genre" | "mood" | "daylist" | "topSongs" | "radio";

/** A made-for-you mix or a radio, as cards and page headers show it. */
export interface MixSummary {
  id: string;
  kind: MixKind;
  title: string;
  /** "A, B, C and more", "with A, B and more", or what the mix is. */
  description: string;
  /** The card's name strip, #rrggbb. */
  color: string;
  /** The leading artist's photo (radio: the seed's picture). */
  art: string | null;
  /** Covers for a collage; on a radio the seed's picture then two artists' photos. */
  covers: string[];
  artistIds: string[];
  tag: string | null;
  trackCount: number;
  durationMs: number;
  /** When it's made again (the daylist), unix ms. */
  nextUpdateAt: number | null;
}

export interface MixShelves {
  madeForYou: MixSummary[];
  moods: MixSummary[];
  genres: MixSummary[];
}

/** A steering chip: lean towards favourites or towards songs played less. */
export type MixSteer = "familiar" | "new";

export interface MixPage {
  mix: MixSummary;
  tracks: Track[];
  /** Per song: a favourite (true) or something less played. */
  familiar: boolean[];
  steer: MixSteer | null;
}

// ---------------------------------------------------------------- transcription

/** Speech models for lyrics transcription (docs/IPC.md "Lyrics transcription"). */
export type TranscribeModel = "small" | "medium" | "turbo";

/** "Use the graphics card" for transcription and the lyrics check. */
export type TranscribeGpu = "auto" | "off";

/** "Processor use": threads for transcription and the lyrics check. */
export type TranscribeProcessor = "light" | "balanced" | "fast";

export interface TranscribeModelInfo {
  kind: TranscribeModel;
  sizeBytes: number;
  installed: boolean;
  downloading: boolean;
  doneBytes: number;
  /** About how long a song took here lately, on the device used now. */
  secsPerSong?: number;
}

export type TranscribeStage = "waitingForModel" | "reading" | "decoding" | "loading" | "transcribing";

export interface TranscribeStatus {
  enabled: boolean;
  /** This build and this computer can transcribe. */
  available: boolean;
  unavailableReason?: string;
  /** The model used: the one picked, else the default for this computer. */
  model: TranscribeModel;
  /** A model was picked in Settings. */
  modelPicked: boolean;
  /** The graphics card transcription uses (absent: the processor). */
  gpu?: string;
  /** A usable graphics card on this computer, used or not. */
  gpuFound?: string;
  models: TranscribeModelInfo[];
  current?: {
    trackId: string;
    stage: TranscribeStage;
    percent: number;
    background: boolean;
    /** Seconds since the stage or percent last moved, once that has been a while. */
    quietSecs?: number;
    /** The song's title, when the library has it. */
    title?: string;
    /** Reading the song waits while music is playing. */
    waitingForPlayback?: boolean;
    /** The model this song uses (an installed one while the chosen one downloads). */
    model?: TranscribeModel;
  };
  queued: number;
  /** The song that goes next. */
  upNext?: string;
  batch: { done: number; total: number };
  /** About how long each song of a long run takes, breather included. */
  secsPerSong?: number;
  paused?: "battery" | "offline" | "busy";
  error?: { kind: "download" | "checksum" | "disk" | "transcribe"; message: string; trackId?: string };
  /** The user paused the batch (songs asked for one at a time still run). */
  userPaused?: boolean;
  /** Songs done with another model while the chosen one downloaded. */
  redo?: { model: TranscribeModel; count: number }[];
}

// ---------------------------------------------------------------- sharing lyrics

/** What LRCLIB gets when a transcription is shared (docs/IPC.md "Sharing lyrics"). */
export interface LyricsSharePayload {
  trackName: string;
  artistName: string;
  albumName: string;
  /** Seconds. */
  duration: number;
  plainLyrics: string;
  syncedLyrics: string;
}

export type LyricsShareWhyNot = "off" | "notTranscribed" | "notReviewed" | "missingInfo" | "unreleased" | "shared";

/** `lyrics_share_info`: whether to offer sharing, and exactly what would go. */
export interface LyricsShareInfo {
  offer: boolean;
  whyNot?: LyricsShareWhyNot;
  /** The song counts as unreleased (sharing publishes its title). */
  unreleased: boolean;
  payload?: LyricsSharePayload;
}

// ---------------------------------------------------------------- file tags

/** One file's release status from whichever tags it carries (docs/TAGGING.md "Release status"). */
export type FileRelease = "released" | "unreleased" | "leaked" | "alternate";

/** A custom release status value and what it means (Settings). */
export interface ReleaseValueMap {
  value: string;
  status: FileRelease;
}

/** What a song's file tags say about where it came from. */
export interface TrackRelease {
  status?: FileRelease;
  /** VERSION as tagged (free text), else ARCHIVE_SOURCE. */
  provenance?: string;
}

/** Badges for one song from its file's tags (docs/IPC.md "File tags"). */
export interface TrackBadges {
  explicit?: boolean;
  /** ARCHIVE_VERSION, lower case ("leak", "demo", "released"...). */
  version?: string;
  /** ARCHIVE_QUALITY, lower case ("lossless", "cdq", "lq", "snippet"). */
  quality?: string;
  /** A fan or AI edit. */
  edit?: boolean;
  /** The file's release status from whichever tags it carries. */
  release?: FileRelease;
  /** Where the file came from, as tagged (VERSION, else ARCHIVE_SOURCE). */
  provenance?: string;
}

/** What the file tags say about one album. */
export interface AlbumBadges {
  explicit?: boolean;
  version?: string;
  /** ARCHIVE_GROUP: albums sharing it are editions of one album. */
  group?: string;
  /** ARCHIVE_EDITION label ("Standard", "Deluxe", "Build v2"). */
  edition?: string;
  /** The album's status by its files' release status tags (disc 1 decides). */
  release?: FileRelease;
  /** Some files are released and some aren't. */
  mixed?: boolean;
}

/** One edition in a group (the album page's picker). */
export interface Edition {
  albumId: string;
  title: string;
  label: string | null;
  year: number | null;
}

/** `tags_index`: badges, album tags and edition groups (face first). */
export interface TagIndex {
  tracks: Record<string, TrackBadges>;
  albums: Record<string, AlbumBadges>;
  groups: Record<string, Edition[]>;
}

/** `tag_scan_status` and `tags://progress`. */
export interface TagScanStatus {
  enabled: boolean;
  running: boolean;
  done: number;
  total: number;
  failed: number;
  /** Waiting after network or server trouble. */
  paused: boolean;
}

// ---------------------------------------------------------------- discord

/** `discord_status` and the `discord://status` event. */
export interface DiscordStatus {
  /** This build has a Discord app id (without one the feature is hidden). */
  available: boolean;
  /** Talking to the Discord app right now. */
  connected: boolean;
  /** The Imgur client ID: saved in the keychain, refused by Imgur. */
  imgurKey: InfoKeyState;
  /** The last cover upload failed, so Discord shows the Platter icon. */
  uploadFailed: boolean;
}

// ---------------------------------------------------------------- sharing

/** What a share or a platter:// link points at. */
export type ShareKind = "track" | "album" | "artist" | "playlist";

/**
 * Whether an item can get a public link: "available" (released music),
 * "unreleased" (never), "off" (turned off in Settings) or "none" (artists
 * and playlists only get Platter links).
 */
export type PublicLinkState = "available" | "unreleased" | "off" | "none";

/** `share_info`: the Platter link and whether a public link can be had. */
export interface ShareInfo {
  /** platter:// link, with ?t= when a start time was asked for. */
  appLink: string;
  public: PublicLinkState;
}

/** A platter:// link that was opened (`deep_link_take`). */
export interface DeepLinkOpen {
  status: "open" | "notInLibrary" | "invalid";
  kind: ShareKind | null;
  id: string | null;
  /** For a song: its album, the page that opens. */
  albumId: string | null;
  /** For a song: where to start when the user presses play. */
  atSeconds: number | null;
  /** A listening session invite (platter://join/...) to join. */
  join?: string | null;
  /** A shared playlist or Blend invite (platter://collab/..., platter://blend/...) to join. */
  circle?: string | null;
  /** A share link (platter://share/...): ask that person for their music. */
  shareAsk?: string | null;
  /** For an invite or share link: the relay it names (absent: the usual one). */
  relay?: string | null;
}

// ---------------------------------------------------------------- social

/** One person's latest listen on the Plex server. */
export interface FriendActivity {
  accountId: number;
  name: string;
  /** Art path for the round avatar. */
  thumb: string | null;
  /** The song in this library; null when it isn't here. */
  trackId: string | null;
  title: string;
  artist: string;
  /** The artist in this library, if found. */
  artistId: string | null;
  /** The album in this library, if found. */
  albumId: string | null;
  album: string;
  cover: string | null;
  /** Unix seconds. */
  at: number;
  /** Listening right now. */
  now: boolean;
  /** The song is in this library (by id from this server, or by name from another). */
  inLibrary: boolean;
}

/** "ok", or why there's nothing: not the server owner, turned off, or offline. */
export type ActivityState = "ok" | "notAdmin" | "off" | "offline";

export interface ActivityPanel {
  state: ActivityState;
  friends: FriendActivity[];
}

export interface FollowedArtist {
  /** A library artist id, or a credited artist's credit: id. */
  id: string;
  title: string;
  thumb: string | null;
  /** Unix seconds. */
  followedAt: number;
}

export interface NewRelease {
  album: Album;
  /** Added since What's New was last seen. */
  new: boolean;
}

// ---------------------------------------------------------------- hide, bookmarks, smart playlists, scrobbling

export type HiddenKind = "track" | "artist";

/** A song or artist left out of mixes, radio, autoplay and shuffle. */
export interface HiddenItem {
  kind: HiddenKind;
  id: string;
  title: string;
  /** A song's artist. */
  subtitle?: string | null;
  thumb?: string | null;
  /** Unix seconds. */
  hiddenAt: number;
}

export interface HideRequest {
  kind: HiddenKind;
  id: string;
  title: string;
  subtitle?: string | null;
  thumb?: string | null;
}

export type BookmarkKind = "track" | "page";
/** Pages a bookmark can open (route names). */
export type BookmarkPage = "album" | "artist" | "playlist" | "mix" | "radio" | "liked" | "smart" | "discography";

export interface Bookmark {
  id: string;
  kind: BookmarkKind;
  /** The song's id, or the page's id. */
  target: string;
  page?: BookmarkPage | null;
  title: string;
  subtitle?: string | null;
  thumb?: string | null;
  /** Where in the song (songs only). */
  positionMs?: number | null;
  /** Unix seconds. */
  createdAt: number;
}

export type NewBookmark = Omit<Bookmark, "id" | "createdAt">;

export type SmartField =
  | "genre"
  | "style"
  | "mood"
  | "artist"
  | "album"
  | "year"
  | "decade"
  | "addedAt"
  | "playCount"
  | "lastPlayed"
  | "playedHere"
  | "rating"
  | "liked"
  | "duration"
  | "releaseType"
  | "explicit"
  | "version"
  | "era"
  | "unreleased"
  | "lossless"
  | "skips"
  | "finished"
  | "played";
/** `between` takes two local days: "2026-06-01..2026-08-31". */
export type SmartOp = "is" | "isNot" | "contains" | "notContains" | "gt" | "lt" | "inLast" | "notInLast" | "between";

export interface SmartRule {
  field: SmartField;
  op: SmartOp;
  value: string;
  /** The Plex field it was read from (kept when saved back). */
  plexField?: string | null;
}

export type SmartSort = "none" | "random" | "mostPlayed" | "leastPlayed" | "recentlyAdded" | "recentlyPlayed" | "rating";
export type SmartUpdate = "live" | "daily" | "weekly";

export interface SmartRules {
  matchAll: boolean;
  rules: SmartRule[];
  limit?: { unit: "songs" | "hours"; value: number } | null;
  sort: SmartSort;
  update: SmartUpdate;
  /** Plex options the editor doesn't show, kept as they are. */
  extra?: [string, string][];
}

export interface SmartSummary {
  id: string;
  title: string;
  summary: string | null;
  trackCount: number;
  durationMs: number;
  update: SmartUpdate;
  /** Unix seconds. */
  nextUpdateAt: number | null;
  updatedAt: number;
  prompted: boolean;
}

export interface SmartPage {
  playlist: SmartSummary;
  rules: SmartRules;
  picks: string[];
  prompt: string | null;
  tracks: Track[];
}

export interface SmartPreview {
  count: number;
  durationMs: number;
  tracks: Track[];
  /** Why these rules make a Platter smart playlist (empty: Plex can hold them). */
  plexGaps: string[];
}

export interface SmartSaveRequest {
  /** Editing: a Platter (`smart:`) or Plex playlist id. */
  id?: string | null;
  title: string;
  summary?: string | null;
  rules: SmartRules;
  picks?: string[];
  prompt?: string | null;
  /** Keep it on this computer even if Plex could hold it. */
  platter?: boolean;
}

export interface SmartSaved {
  id: string;
  platter: boolean;
  note: string | null;
}

export interface PromptResult {
  title: string;
  rules: SmartRules;
  picks: Track[];
}

/** Describe a playlist: rules read from a description on this computer. */
export interface Described {
  title: string;
  rules: SmartRules;
  /** Words that matched nothing, as typed. */
  unknown: string[];
  /** Other readings for the rule at `index`. */
  choices: { text: string; index: number; others: SmartRule[] }[];
  notes: string[];
}

export type AiProvider = "none" | "anthropic";
export const DEFAULT_AI_MODEL = "claude-opus-5-5";

export interface AiStatus {
  provider: AiProvider;
  model: string;
  hasKey: boolean;
  /** Prompted playlists show. */
  ready: boolean;
}

/** `lidarr_status`: what's set up, before any check. */
export interface LidarrStatus {
  urlSet: boolean;
  awaySet: boolean;
  hasKey: boolean;
  /** Names of the saved proxy headers (values never leave the keychain). */
  headerNames: string[];
  /** Addresses that would send the key unencrypted over the internet. */
  unencrypted: string[];
  /** Set up, with a key, and the last setup check passed: Request shows. */
  ready: boolean;
}

export type LidarrLevel = "red" | "amber" | "ok";

export interface LidarrFinding {
  id: string;
  level: LidarrLevel;
  title: string;
  detail: string;
  /** What to change in Lidarr. */
  fix: string | null;
  /** Set on an empty root folder the user can vouch for. */
  emptyRoot: { path: string; confirmed: boolean } | null;
  /** A red finding that only stops work for protected artists. */
  protectedOnly: boolean;
  /** A fix Platter can make; it shows the exact change and asks first. */
  action: LidarrFixAction | null;
  /** A Lidarr root folder no Plex library scans, offered as the staging folder (a Platter setting). */
  suggestedStaging?: string;
}

/** A one-click fix (a Lidarr write that asks first). */
export type LidarrFixAction =
  | { kind: "addExclusions"; artists: { name: string; mbid: string }[] }
  | { kind: "setRecycleBin" }
  | { kind: "writeTagsOff" }
  | { kind: "renameOff" }
  | { kind: "addStagingRoot"; path: string }
  | { kind: "rematch"; folders: string[] }
  | { kind: "releaseSwitchOff"; albums: { id: number; title: string }[] };

/** `lidarr_check`: the read-only setup check. */
export interface LidarrReport {
  version: string;
  /** Red first, then amber, then OK. */
  findings: LidarrFinding[];
  /** No red findings: work for protected artists may go ahead. */
  passed: boolean;
  /** No red finding that stops Managed artists. */
  managedOk: boolean;
  /** Protected artists the check was run for. */
  protectedCount: number;
  checkedAt: number;
  /** Which address answered. */
  route: "home" | "away" | "bridge" | null;
}

/** Managed: Lidarr owns the artist's folder. Protected: curated files Platter changes itself. */
export type LidarrMode = "managed" | "protected";
export type LidarrModeSetting = "auto" | "managed" | "protected";
/** What better copies aim for. */
export type LidarrTarget = "lossless" | "hiRes";

/** `lidarr_modes`: one artist's mode, with why. */
export interface LidarrArtistMode {
  /** The key choices are kept under (MusicBrainz id, else name:<folded>). */
  key: string;
  name: string;
  artistId: string | null;
  mbid: string | null;
  lidarrId: number | null;
  lidarrPath: string | null;
  folder: string | null;
  mode: LidarrMode;
  setting: LidarrModeSetting;
  /** What Auto would pick. */
  auto: LidarrMode;
  /** "Protected: 403 files tagged unreleased or leaked". */
  line: string;
  reasons: string[];
  files: number;
  curatedFiles: number;
  excluded: boolean;
}

/** An artist's choices (shared across devices). */
export interface LidarrArtistPrefs {
  mode: LidarrModeSetting;
  /** Add new official albums on their own; null asks the first time. */
  autoAdd: boolean | null;
  hunt: boolean;
  target: LidarrTarget;
  name: string;
}

/** A change to Lidarr waiting for a yes. */
export interface LidarrProposal {
  id: string;
  what: string;
  before: string;
  after: string;
  canPutBack: boolean;
}

/** One entry in the changes log. */
export interface LidarrChange {
  id: string;
  at: number;
  device: string;
  what: string;
  before: string;
  after: string;
  /** The user said yes to it (false: part of a request they made). */
  asked: boolean;
  /** Can be put back. */
  undo: unknown | null;
  putBackAt: number | null;
  /** Practice mode: what Platter would have done. */
  practice: boolean;
  /** Lidarr made this change on its own; Platter only noticed it. */
  byLidarr?: boolean;
}

export interface LidarrRematchPlan {
  folder: string;
  albumId: number | null;
  album: string;
  release: string;
  /** The edition shown; the re-match stops if Lidarr has moved the album since. */
  releaseId: number | null;
  files: string[];
  kept: { path: string; why: string }[];
  otherEdition: number;
  linkedNow: number;
  editionNote: string | null;
}

export interface LidarrPlannedFile {
  src: string;
  name: string;
  /** Tag name and the value set (null: removed). */
  edits: [string, string | null][];
}

export type LidarrStagedState = "waiting" | "ready" | "practice" | "added" | "undone";

/** A protected album that landed in the staging folder. */
export interface LidarrStaged {
  id: string;
  albumId: number;
  lidarrArtistId: number;
  artist: string;
  title: string;
  releaseGroup: string;
  partial: boolean;
  state: LidarrStagedState;
  reason: string | null;
  plan: { folder: string; files: LidarrPlannedFile[] } | null;
  folder: string | null;
  confident: boolean;
  doubts: string[];
  auto: boolean;
  at: number;
  updatedAt: number;
}

export interface LidarrCatchUpStatus {
  /** The device that last did the Lidarr work, and when (unix seconds). */
  lastDevice: string | null;
  lastAt: number | null;
  gap: string | null;
  paused: string | null;
}

/** `lidarr_work`: catching up, practice mode and albums in staging. */
export interface LidarrWork {
  catchUp: LidarrCatchUpStatus;
  practiceUntil: number | null;
  practice: boolean;
  staged: LidarrStaged[];
}

export interface LidarrCatchUpReport {
  handled: number;
  elsewhere: number;
  settling: number;
  acted: number;
  gap: string | null;
  scanned: string[];
  announced: string[];
  staged: number[];
  healed: string[];
  paused: string | null;
}

export type LidarrHuntState = "atTarget" | "requested" | "waiting" | "partlyUpgraded" | "outOfScope" | "needsProfile" | "failed";

export interface LidarrHuntItem {
  albumId: string;
  title: string;
  artist: string;
  now: string;
  state: LidarrHuntState;
  note: string | null;
}

/** Where a Lidarr request stands. */
/** An album "Bring in my listening" couldn't ask for on its own. */
export interface BringInNeedsYou {
  key: string;
  artist: string;
  album: string;
  plays: number;
  reason: string;
  /** Unix seconds. */
  at: number;
}

/** What practice mode would have asked for. */
export interface BringInPracticed {
  key: string;
  artist: string;
  album: string;
  releaseGroupId: string;
  at: number;
}

/** `bring_in_status`: the progress card. */
export interface BringInStatus {
  on: boolean;
  /** Practice mode: albums are only listed. */
  practice: boolean;
  /** Lidarr is set up and its last check passed. */
  ready: boolean;
  waves: number[];
  wave: number;
  /** The current wave's fewest plays; null once every wave is done. */
  threshold: number | null;
  done: number;
  /** Albums left in the current wave. */
  remaining: number;
  today: number;
  perDay: number;
  /** Why a guard paused it. */
  paused: string | null;
  userPaused: boolean;
  finished: boolean;
  requestedTotal: number;
  needsYou: BringInNeedsYou[];
  practiced: BringInPracticed[];
  /** Albums "Get albums" queued and not asked for yet, in order. */
  queued?: QueuedGet[];
  /** The paired Bridge's name while it runs the waves. */
  bridge?: string | null;
}

/** An album "Get albums" queued. */
export interface QueuedGet {
  releaseGroupId: string;
  title: string;
  artist: string;
  artistMbid: string | null;
  artistId: string | null;
}

/** Which plays "Played a lot" counts. */
export type PlayedPeriod = "all" | "year" | "days90";

/** How one album of a "Request these" went. */
export interface LidarrBatchItem {
  releaseGroupId: string;
  title: string;
  record: LidarrRequest | null;
  /** Why it wasn't sent. */
  error: string | null;
}

export type LidarrRequestState = "queued" | "requested" | "searching" | "downloading" | "importing" | "imported" | "failed" | "notFound";

/** `lidarr_get`: the request one-click Get made, or why it needs the user. */
export interface LidarrGetOutcome {
  record: LidarrRequest | null;
  /** `firstTime`: the first request ever is confirmed by typing the album's name; `refused`: a check said no. */
  needs: { kind: "firstTime" | "refused"; reason: string } | null;
}

/** What a request needs: the release group and what it's called. */
export interface LidarrRequestInput {
  releaseGroupId: string;
  title: string;
  artist: string;
  artistId?: string | null;
  /** The artist's MusicBrainz id, for an artist that isn't in the library. */
  artistMbid?: string | null;
  /** The album's name typed by the user (their first request ever). */
  typed?: string | null;
}

/** `lidarr_request_preview`: what a request will do. Reads only. */
export interface LidarrPreview {
  releaseGroupId: string;
  title: string;
  artist: string;
  /** Where the files will land, as Lidarr sees it. */
  destination: string | null;
  /** From the last check, since Lidarr can't be reached now. */
  destinationIsGuess: boolean;
  /** The first request ever: the album's name must be typed. */
  needsTyped: boolean;
  /** Lidarr has the album already; nothing will be sent. */
  existing: LidarrRequestState | null;
  /** False: the request will wait until Platter can reach Lidarr. */
  reachable: boolean;
  route: "home" | "away" | "bridge" | null;
  /** The artist is new to Lidarr and will be added with only this album. */
  addsArtist: boolean;
  /** A protected artist: it downloads into the staging folder and waits there. */
  protected: boolean;
}

/** One request, kept on this device. */
export interface LidarrRequest {
  id: string;
  releaseGroupId: string;
  title: string;
  artist: string;
  artistId: string | null;
  state: LidarrRequestState;
  destination: string | null;
  lidarrAlbumId: number | null;
  addedArtist: boolean;
  alreadyInLidarr: boolean;
  createdAt: number;
  updatedAt: number;
  /** Why it's waiting or failed. */
  message: string | null;
  /** What Platter did along the way. */
  notes: string[];
  /** Made by auto-request for a new release. */
  auto: boolean;
  /** For a protected artist: lands in the staging folder and waits there. */
  protected: boolean;
  /** How far the download has got (0 to 100) while it downloads. */
  progress?: number | null;
  /** When the first search went out (unix seconds). */
  searchedAt?: number | null;
  /** Searches sent so far. */
  searches?: number;
  /** When Lidarr is asked to search again while nothing was grabbed. */
  nextSearchAt?: number | null;
  /** Once imported: what Lidarr got ("FLAC, 13 tracks"), and whether it keeps looking for better. */
  grabbed?: { line: string; upgradeNote: string | null } | null;
}

/** Why a swap or its undo stopped. Every one leaves the old files in place. */
export type UpgradeFailure =
  | { kind: "noMapping" }
  | { kind: "shareUnreachable"; path: string }
  | { kind: "changed"; name: string }
  | { kind: "stagingMissing"; name: string }
  | { kind: "fileLocked"; name: string }
  | { kind: "nameTaken"; name: string }
  | { kind: "tagCopyFailed"; name: string; why: string }
  | { kind: "backupNotWritable"; path: string }
  | { kind: "moveFailed"; name: string }
  | { kind: "rescanFailed" }
  | { kind: "plexUnreachable" }
  | { kind: "backupMissing"; name: string }
  | { kind: "backupInLibrary"; path: string };

/** One file a swap replaced. */
export interface SwappedFile {
  trackId: string;
  plexPath: string;
  original: string;
  placed: string;
  backup: string;
  oldHash: string;
  newHash: string;
}

/** One swap that happened. */
export interface SwapRun {
  id: string;
  at: number;
  /** Where the old files are now. */
  backupDir: string;
  files: SwappedFile[];
  undone: boolean;
  /** "Upgraded 12 songs (MP3 320 to FLAC)". */
  summary: string;
}

/** A better copy of a library album: its Lidarr request and any swaps. */
export interface UpgradeRecord extends LidarrRequest {
  albumId: string;
  /** What the album was when the upgrade was asked for ("MP3 320"). */
  fromQuality: string;
  swaps: SwapRun[];
  failure: UpgradeFailure | null;
}

/** `lidarr_upgrade_album`: whether the album menu offers "Find a better copy". */
export interface AlbumUpgrade {
  available: boolean;
  record: UpgradeRecord | null;
  /** The album now, "MP3 320". */
  quality: string | null;
  /** The MusicBrainz release the user confirmed this album is. */
  confirmed: UpgradeConfirmed | null;
}

/** The user's word that a library album is this release group. */
export interface UpgradeConfirmed {
  releaseGroupId: string;
  title: string;
  at: number;
}

/** One track of a release's official tracklist. */
export interface ListedTrack {
  disc: number;
  position: number;
  title: string;
  lengthMs: number | null;
}

/** `lidarr_upgrade_candidates`: the album's songs and the releases it could be. */
export interface UpgradeCandidates {
  title: string;
  artist: string;
  songs: { disc: number; track: number; title: string; durationMs: number }[];
  confirmed: UpgradeConfirmed | null;
  candidates: { group: LidarrGroup; tracks: ListedTrack[] }[];
}

export type UpgradeKeep =
  | "unreleased"
  | "notInAlbum"
  | "notOnRelease"
  | "notInNewCopy"
  | "noReleasedTag"
  | "noOfficialNote"
  | "lengthDiffers"
  | "differentVersion"
  | "soundDiffers"
  | "ambiguous"
  | "notBetter";

export interface OfferSong {
  /** Empty for a file Plex doesn't list on the album. */
  trackId: string;
  title: string;
  disc: number;
  track: number;
  from: string;
  to: string | null;
  /** Why it's kept as it is. */
  reason: string | null;
  keep: UpgradeKeep | null;
  /** How it was matched: the same ISRC, or released + official note + length + sound. */
  by: "isrc" | "sound" | null;
}

export type OfferStatus = "waiting" | "needsMapping" | "unreachable" | "ready" | "nothing" | "swapped";

/** `lidarr_upgrade_offer`: what a swap would do now. */
export interface UpgradeOffer {
  upgradeId: string;
  albumId: string;
  title: string;
  artist: string;
  status: OfferStatus;
  message: string | null;
  /** "Upgrade 12 of 14 songs (MP3 320 to FLAC)". */
  summary: string | null;
  from: string;
  to: string;
  upgrade: OfferSong[];
  kept: OfferSong[];
  /** Ask about each song, whatever the setting says. */
  askEach: boolean;
  askReason: string | null;
  /** "Upgrades: ask before each swap". */
  ask: boolean;
  /** Where old files go. */
  backupDir: string;
  lastSwap: SwapRun | null;
  failure: UpgradeFailure | null;
  failureMessage: string | null;
}

export interface SwapResult {
  record: UpgradeRecord;
  swapped: number;
  backupDir: string;
  summary: string;
}

/** Which albums "look for better copies" covers. */
export type UpgradeUnder = "lossy" | "belowCd";

export interface UpgradeBulkResult {
  requested: UpgradeRecord[];
  skipped: { title: string; reason: string }[];
  /** Albums left for another run. */
  more: number;
}

/** Where this computer reaches the music share (this device only). */
export interface ShareMap {
  music: { server: string; local: string }[];
  /** The staging folder on this computer. */
  staging: string;
  /** Where replaced files go (empty: Platter's own folder). */
  backup: string;
}

/** One release on the release radar. */
export interface RadarItem {
  releaseGroupId: string;
  title: string;
  artist: string;
  artistId: string | null;
  /** "album", "ep" or "single". */
  kind: string;
  /** First release date: "YYYY-MM-DD", "YYYY-MM" or "YYYY". */
  date: string;
  /** The front cover from the Cover Art Archive, as an art path. */
  thumb: string | null;
  /** The library album it matches ("In your library"). */
  libraryAlbumId: string | null;
  request: LidarrRequestState | null;
  /** The request was made by auto-request. */
  auto: boolean;
  /** Why it can't be requested. */
  refusal: string | null;
  /** Found by the latest look. */
  new: boolean;
}

/** `release_radar`. */
export interface RadarPage {
  /** The setting is on. */
  shown: boolean;
  items: RadarItem[];
  checkedAt: number | null;
  checking: boolean;
  lidarrReady: boolean;
  /** Artists looked at. */
  artists: number;
}

/** An artist with auto-request on (this device). */
export interface AutoArtist {
  id: string;
  name: string;
  enabledAt: number;
}

/** One official release the library lacks. */
export interface LidarrMissingItem {
  releaseGroupId: string;
  title: string;
  /** "album", "ep" or "single". */
  kind: string;
  /** Secondary types ("live", "compilation"...), lower case. */
  secondary?: string[];
  /** The discography section it shows in (older cores leave it out: albums and EPs only then). */
  section?: ReleaseSection;
  year: number | null;
  request: LidarrRequestState | null;
  /** The small front cover from the Cover Art Archive, as an art path. */
  thumb: string | null;
  /** Why Get would be refused (a live album or compilation isn't asked for on its own). */
  refusal?: string | null;
}

/** A MusicBrainz release group Lidarr could be asked for. */
export interface LidarrGroup {
  id: string;
  title: string;
  primary: string | null;
  secondary: string[];
  firstRelease: string;
}

/** `lidarr_pick`: the release group to request for an album or song, or why there's none. */
export interface LidarrPicked {
  group: LidarrGroup | null;
  artist: string;
  refusal: { kind: string } | null;
  reason: string | null;
}

export interface LidarrPickInput {
  artist: string;
  artistId?: string | null;
  title: string;
  /** A song (its album is picked) rather than an album. */
  song: boolean;
  isrc?: string | null;
}

/** `lidarr_missing_releases`: for the artist page. */
export interface LidarrMissing {
  /** Setting on, Lidarr set up, last check green. */
  shown: boolean;
  items: LidarrMissingItem[];
}

export interface ScrobbleStatus {
  lastfm: {
    hasKey: boolean;
    hasSecret: boolean;
    signedIn: boolean;
    user: string | null;
    rejected: boolean;
    pending: boolean;
  };
  listenbrainz: { hasToken: boolean; user: string | null; rejected: boolean };
  queued: number;
}

export interface WhatsNew {
  /** Some artist is followed (otherwise everything recently added). */
  following: boolean;
  items: NewRelease[];
  unseen: number;
}

export type ListenRole = "none" | "host" | "guest";
export type ListenStatus = "connecting" | "live" | "hostGone" | "ended" | "removed";
export type RelayHealth = "connecting" | "ok" | "unreachable" | "limited";

/** Someone in a listening session: a display name, never an address. */
export interface ListenPerson {
  id: string;
  name: string;
  /** No pictures are sent yet; the name's letter shows instead. */
  avatar: string | null;
  host: boolean;
  me: boolean;
  /** The host's view: this guest has no copy of the song playing. */
  cantHear: boolean;
  /** The host's view: where "Let them listen from my server" is for them. */
  offer?: ListenOffer | null;
}

/** "Let them listen from my server", for one guest (the host's view). */
export type ListenOffer =
  | { state: "asking" | "inviting" | "invited" | "alreadyShared" | "noInvites" }
  | { state: "failed"; message: string };

/** A guest's view: the host's song has no copy in this library. */
export interface ListenMissing {
  /** Null when the host hides unreleased titles. */
  title: string | null;
  artist: string | null;
  unreleased: boolean;
}

/** An invite: a single-use link that runs out. */
export interface ListenInvite {
  link: string;
  /** The link as a QR code: rows of "0" and "1" (dark). */
  qr: { size: number; rows: string[] } | null;
  /** Unix ms. */
  expiresAt: number;
}

/** A listening session as this computer sees it. */
export interface ListenView {
  role: ListenRole;
  sessionId: string | null;
  hostName: string | null;
  /** The invite to send (host only); null once used or run out. */
  invite: ListenInvite | null;
  /** Everyone in the session, host first. */
  people: ListenPerson[];
  /** Everyone else's names. */
  listeners: string[];
  /** Guests may play, pause, skip, seek and reorder (else only add). */
  guestsCanControl: boolean;
  /** Who added the song playing (a person id). */
  nowBy: string | null;
  /** The shared up next, and who added each song. */
  queue: { trackId: string; by: string | null }[];
  status: ListenStatus | null;
  relay: RelayHealth;
  /** Host: the names of guests who can't hear the song playing. */
  cantHear: string[];
  /** Guest: the song playing isn't in this library; the player waits for the next one. */
  missing: ListenMissing | null;
  /** Guest: playing a copy of another length. */
  anotherVersion: boolean;
  /** Host: "Let them listen from my server" can be offered. */
  offerServer?: boolean;
}

// ---------------------------------------------------------------- sharing

/** A music library on your server, for choosing what to share. */
export interface SharingLibrary {
  id: string;
  title: string;
  /** Unreleased albums in it; null when Platter hasn't read the library. */
  unreleased: number | null;
  /** In "Libraries I share by default". */
  default: boolean;
}

/** `sharing_overview`: what you can share. */
export interface SharingOverview {
  serverName: string | null;
  /** Only the server's owner shares it. */
  owner: boolean;
  /** Null when plex.tv couldn't say. */
  plexPass: boolean | null;
  /** Music libraries only. */
  libraries: SharingLibrary[];
}

export interface ShareOptions {
  libraries: string[];
  hideUnreleased: boolean;
  allowDownloads: boolean;
}

/** How a share went. */
export type ShareStatus =
  | { status: "invited"; hidden: number }
  | { status: "alreadyShared" }
  /** Hiding unreleased music needs Plex Pass; nothing was shared. `others`: chosen libraries without unreleased music. */
  | { status: "needsPlexPass"; withUnreleased: string[]; others: string[] }
  | { status: "unknownUser"; message: string }
  | { status: "refused"; message: string }
  | { status: "plexDown"; message: string };

/** Someone with access to your server. */
export interface PersonWithAccess {
  id: string;
  name: string;
  libraries: string[];
  allowDownloads: boolean;
  unreleasedHidden: boolean;
  /** Invited, not accepted yet. */
  pending: boolean;
  /** Unix seconds. */
  lastSeenAt: number | null;
}

/** A server invite you received. */
export interface ReceivedInvite {
  id: string;
  from: string;
  servers: string[];
  friend: boolean;
  home: boolean;
  server: boolean;
  createdAt: number | null;
}

/** `sharing_accept`. */
export interface InviteAccepted {
  servers: string[];
  libraries: number;
}

/** A share link and its QR code. */
export interface ShareLinkView {
  link: string;
  qr: { size: number; rows: string[] } | null;
  expiresAt: number;
}

/** What opening someone's share link did. */
export type ShareLinkResult = { status: "asked" | "invited" | "alreadyShared"; host: string } | { status: "failed"; message: string };

/** Someone who opened your share link and waits for your yes. */
export interface ShareLinkAsk {
  id: string;
  /** Their Plex username. */
  user: string;
  /** The name they go by in Platter. */
  name: string;
  /** Unix ms. */
  at: number;
}

/** What a guest asks the host to do (`listen_control`). */
export type ListenOp =
  | { op: "play" }
  | { op: "pause" }
  | { op: "next" }
  | { op: "previous" }
  | { op: "seek"; ms: number }
  | { op: "add"; ids: string[] }
  | { op: "move"; from: number; to: number };

// ---------------------------------------------------------------- artist info

/** An outside artist info source. */
export type InfoSource = "wikipedia" | "lastfm" | "fanart" | "audiodb" | "listenbrainz";
/** A source that takes the user's API key (ListenBrainz: an optional token). */
export type KeyedInfoSource = Exclude<InfoSource, "wikipedia">;

export interface InfoMember {
  name: string;
  /** Set when the member is an artist in this library. */
  artistId: string | null;
  /** "1991–present", "1991–2004" (MusicBrainz), when known. */
  years?: string;
}

/** Another artist this one is related to (MusicBrainz). */
export interface InfoRelated {
  name: string;
  artistId: string | null;
  /** "Member of", "Subgroup", "Collaboration", "Formerly", and so on. */
  relation: string;
}

/** A "where this came from" line in the About dialog. */
export interface InfoCredit {
  source: "wikipedia" | "wikidata" | "lastfm" | "fanart" | "audiodb" | "musicbrainz" | "listenbrainz";
  text: string;
  url: string | null;
  licence: string | null;
  licenceUrl: string | null;
}

/** Outside info for an artist page (Plex first, sources filling gaps). */
export interface ArtistInfo {
  id: string;
  bio: string | null;
  bioSource: "plex" | "wikipedia" | "lastfm" | "audiodb" | null;
  origin: string | null;
  yearsActive: string | null;
  members: InfoMember[];
  tags: string[];
  /** Last.fm listeners and plays, else ListenBrainz's. */
  listeners: number | null;
  scrobbles: number | null;
  /** Where the listener numbers come from. */
  listenersFrom: "lastfm" | "listenbrainz" | null;
  /** Other artists this one is related to (MusicBrainz). */
  related: InfoRelated[];
  /** Art path for a photo, only when Plex has none. */
  photo: string | null;
  /** Art path for the header banner, only when Plex has no background art. */
  banner: string | null;
  logo: string | null;
  credits: InfoCredit[];
  /** Art paths for the About dialog's gallery: Plex's images first, then the sources'. */
  gallery: string[];
  /** Official website and profiles (Wikidata), then Wikipedia and Last.fm. */
  links: InfoLink[];
  /** Library artists the bio names, as written there. */
  bioLinks: BioLink[];
}

/** One person credited on an album or a song (MusicBrainz). */
export interface AlbumCreditRow {
  name: string;
  /** A library artist with this name. */
  artistId: string | null;
  /** "producer", "writer", "composer", "lyricist", "featured", "mixer", "engineer", "mastering". */
  roles: string[];
}

/**
 * Whether an album counts as released. taggedUnreleased: a demo release type
 * or an unreleased version tag. probablyUnreleased: Plex couldn't match it and
 * nothing is tagged. plexMissed: Plex couldn't match it but MusicBrainz has it.
 */
export type ReleaseStatus = "taggedUnreleased" | "confirmedUnreleased" | "probablyUnreleased" | "plexMissed" | "released";

/** The user's answer for an album or a song. */
export type ReleaseMark = "unreleased" | "released";

export type EvidenceKind =
  | "tag"
  | "mark"
  | "markOverridden"
  | "unmatched"
  | "untyped"
  | "musicbrainz"
  | "notChecked"
  | "folder"
  | "duration";

/** One piece of evidence: weight leans unreleased (positive) or released (negative). */
export interface Evidence {
  kind: EvidenceKind;
  text: string;
  weight: number;
  /** This piece settled the status. */
  decides: boolean;
}

// ---------------------------------------------------------------- library doctor

/** What a library doctor finding is about. */
export type DoctorKind = "duplicate" | "cover" | "albumTags" | "splitAlbum" | "albumArtist" | "lowQuality" | "unmatched" | "soundsLike";

/** The app flow a finding's fix opens. */
export type DoctorFix = "cover" | "match" | "mergeAlbums" | "openAlbum" | "tags" | "suggestedTags";

/** Which library doctor checks run (Settings). */
export interface DoctorChecks {
  duplicates: boolean;
  covers: boolean;
  albumTags: boolean;
  splitAlbums: boolean;
  albumArtist: boolean;
  lowQuality: boolean;
  /** Songs identified by sound as another recording than their tags say. */
  soundsLike?: boolean;
  unmatched: boolean;
}

/** One thing worth fixing. */
export interface DoctorFinding {
  /** Stable: the kind and the items it's about. */
  id: string;
  kind: DoctorKind;
  artistId: string;
  artist: string;
  albumId: string;
  album: string;
  thumb: string | null;
  title: string;
  /** What's wrong and what the fix does. */
  text: string;
  /** What it's based on, for "Why?". */
  evidence: string[];
  fix: DoctorFix;
  tracks: string[];
  /** The other half of a split album, or the album with the better copy. */
  others: { id: string; title: string; thumb: string | null }[];
  /** Each song's own tags (album tag and album artist findings). */
  table: { trackId: string; title: string; album: string | null; albumArtist: string | null; year: number | null }[];
  /** The tags its sound suggests next to its own (sounds-like findings). Nothing is written. */
  suggest?: DoctorSuggestedTags | null;
}

/** What a song's sound suggests, next to what its file says. */
export interface DoctorSuggestedTags {
  trackId: string;
  title: string;
  artist: string;
  album: string;
  newTitle: string;
  newArtist: string;
  newAlbum: string | null;
  recordingMbid: string;
  /** 0 to 100. */
  score: number;
}

// ---------------------------------------------------------------- identify by sound

/** Which songs identify by sound covers. */
export type SoundIdScope = "untagged" | "all";

/** `sound_id_status`; `soundid://changed` fires when it moves. */
export interface SoundIdStatus {
  /** Identify songs by sound is on (and not offline). */
  on: boolean;
  /** This build came with an AcoustID key. */
  appKey: boolean;
  /** The user saved their own key on this device. */
  userKey: boolean;
  /** AcoustID refused the key in use. */
  keyRefused: boolean;
  /** There's a key to use. */
  configured: boolean;
  running: boolean;
  /** Waiting after network or service trouble. */
  paused: boolean;
  /** Songs the setting covers, done, and identified. */
  total: number;
  done: number;
  identified: number;
}

/** `track_sounds_like`: the recording a song sounds like. A hint only. */
export interface SoundsLike {
  trackId: string;
  recordingMbid: string;
  title: string;
  artist: string;
  album: string | null;
  /** 0 to 100. */
  score: number;
  /** The song is unreleased: never treated as that recording. */
  unreleased: boolean;
  /** A released copy of that recording in the library. */
  inLibrary: { trackId: string; albumId: string } | null;
}

/** `doctor_report`. */
export interface DoctorReport {
  on: boolean;
  findings: DoctorFinding[];
  ignored: DoctorFinding[];
  count: number;
  running: boolean;
  /** Unix seconds. */
  checkedAt: number | null;
  /** Files whose album tags are read, of all files. */
  tagsRead: number;
  tagsTotal: number;
}

/** `album_release_status`; `releasestatus://changed` fires when it changes. */
export interface AlbumReleaseStatus {
  id: string;
  status: ReleaseStatus;
  /** Plain sentences for "Why?". */
  reasons: string[];
  artistId?: string;
  /** Every piece of evidence with its weight. */
  evidence?: Evidence[];
  /** The user's mark on the album. */
  mark?: ReleaseMark | null;
  /** The weights summed. */
  score?: number;
  /** Its files mix released songs with unreleased, leaked or alternate ones. */
  mixed?: boolean;
}

// ---------------------------------------------------------------- sleeve notes

/** Where a sleeve notes fact came from, in the order they win. */
export type SleeveSource = "tags" | "plex" | "musicbrainz" | "discogs";

/** One fact and the sources that agree on it, the winning source first. */
export interface SleeveFact {
  value: string;
  sources: SleeveSource[];
}

/** One person in a role group; `key` opens their credit page. */
export interface SleevePerson {
  name: string;
  key: string;
  artistId: string | null;
  /** Instruments, for performers. */
  details: string[];
  sources: SleeveSource[];
}

/** Everyone credited with one role ("producer", "writer", "performer"...). */
export interface SleeveRoleGroup {
  role: string;
  people: SleevePerson[];
}

export interface SleeveStudio {
  name: string;
  /** "recorded", "mixed" or "mastered". */
  role: string;
  sources: SleeveSource[];
}

export interface SleeveTrack {
  trackId: string;
  title: string;
  disc: number;
  number: number;
  groups: SleeveRoleGroup[];
  recorded: SleeveFact | null;
}

/** Which lookups are on, so an empty tab can say how to get more. */
export interface SleeveLookups {
  online: boolean;
  musicbrainz: boolean;
  discogs: boolean;
  /** Unreleased albums are never looked up online. */
  unreleased: boolean;
}

/** `sleeve_notes`: an album's liner notes from every source. */
export interface SleeveNotes {
  albumId: string;
  /** Lookups are running; `sleeve://changed` follows. */
  looking: boolean;
  lookups: SleeveLookups;
  released: SleeveFact | null;
  editionDate: SleeveFact | null;
  /** "1996-11" or a span "1996-11/1997-02". */
  recorded: SleeveFact | null;
  label: SleeveFact | null;
  catalogue: SleeveFact | null;
  country: SleeveFact | null;
  barcode: SleeveFact | null;
  format: SleeveFact | null;
  edition: SleeveFact | null;
  /** "released", "unreleased", "leaked" or "alternate" (from the files' tags). */
  status: SleeveFact | null;
  /** Where an archive file came from. */
  origin: SleeveFact | null;
  /** The release's own notes. */
  notes: SleeveFact | null;
  studios: SleeveStudio[];
  /** How this edition differs from the library's other editions. */
  editionNotes: SleeveFact[];
  groups: SleeveRoleGroup[];
  tracks: SleeveTrack[];
  links: { source: SleeveSource; url: string }[];
}

/** `song_credits`: one song's part of its album's notes. */
export interface SongCredits {
  trackId: string;
  albumId: string;
  looking: boolean;
  lookups: SleeveLookups;
  song: SleeveTrack | null;
  links: { source: SleeveSource; url: string }[];
}

/** One side of a record: "A", "B"... and its songs in album order. */
export interface AlbumSide {
  label: string;
  trackIds: string[];
}

/** `album_sides`: an album's sides, from a vinyl edition or split by hand. */
export interface AlbumSides {
  albumId: string;
  /** Empty when the album has no sides. */
  sides: AlbumSide[];
  source: "musicbrainz" | "discogs" | "manual" | null;
  /** A vinyl edition's sides are known (used unless split by hand). */
  detected: boolean;
}

/** One listening diary entry (`diary_entries`). */
export interface DiaryItem {
  id: string;
  albumId: string;
  /** The album as it was when logged. */
  title: string;
  artist: string;
  year: number | null;
  /** When it finished, or was logged (unix ms). */
  at: number;
  startedAt: number | null;
  songsPlayed: number;
  songsTotal: number;
  minutes: number;
  /** The computer it was played on. */
  device: string | null;
  /** The output device's name. */
  output: string | null;
  /** 0.5 to 5 in half steps. */
  rating: number | null;
  note: string;
  /** Written on its own (false: logged by hand). */
  auto: boolean;
  /** The album's cover now. */
  thumb: string | null;
  inLibrary: boolean;
  listenedOn: "headphones" | "speakers" | null;
}

export interface DiaryQuery {
  from?: number;
  until?: number;
  rated?: boolean;
  withNotes?: boolean;
  text?: string;
  albumId?: string;
}

export interface DiaryList {
  /** Newest first. */
  entries: DiaryItem[];
  /** Entries in the whole diary. */
  total: number;
  firstAt: number | null;
  lastAt: number | null;
}

export interface DiaryEdit {
  rating: number | null;
  note: string;
  at?: number;
}

/** `credit_page`: everything in the library a person is credited on, by role. */
/** What a crate groups albums by. Years are reached from their era. */
export type CrateKind = "label" | "producer" | "studio" | "era" | "year" | "genre";

/** One crate on the Crates page: its first sleeves and how many it holds. */
export interface Crate {
  kind: CrateKind;
  /** The folded name (a decade's first year for an era: "1990"). */
  key: string;
  name: string;
  count: number;
  artists: number;
  fromYear: number | null;
  toYear: number | null;
  /** The first albums, by year. */
  albums: Album[];
}

export interface CrateList {
  kind: CrateKind;
  crates: Crate[];
}

export interface CratePage {
  kind: CrateKind;
  key: string;
  name: string;
  artists: number;
  fromYear: number | null;
  toYear: number | null;
  albums: { album: Album; onMap: boolean }[];
  /** For an era: its years that are crates themselves. */
  years: number[];
  /** Every song, album by album, for Play all and Shuffle (5,000 at most). */
  trackIds: string[];
}

/** "More from <label>" and the like on an album page. */
export interface AlbumShelf {
  kind: CrateKind;
  key: string;
  name: string;
  /** Albums in the whole crate, this one included. */
  count: number;
  /** The crate's other albums. */
  albums: Album[];
}

export interface CreditPage {
  key: string;
  name: string;
  artistId: string | null;
  groups: {
    role: string;
    albums: {
      album: Album;
      /** Credited on the album itself, or on every song. */
      whole: boolean;
      songs: { id: string; title: string }[];
      details: string[];
      sources: SleeveSource[];
    }[];
  }[];
}

/** An album's original release date, label and credits from MusicBrainz. */
export interface AlbumCredits {
  albumId: string;
  /** off: the setting is off. pending: being looked up (`albumcredits://changed` follows). none: no sure match. */
  state: "off" | "pending" | "none" | "found";
  /** The original release date ("1997-05-21", "1997-05" or "1997"). */
  released: string | null;
  /** This edition's date, when it differs from the original. */
  editionDate: string | null;
  label: string | null;
  /** Everyone credited on the album (featured artists stay with their songs). */
  credits: AlbumCreditRow[];
  /** Per-song credits for the songs that matched the library. */
  tracks: { trackId: string; credits: AlbumCreditRow[] }[];
  /** The release on musicbrainz.org. */
  url: string | null;
}

export interface InfoLink {
  kind: "website" | "wikipedia" | "lastfm" | "instagram" | "x" | "facebook" | "youtube" | "bandcamp" | "soundcloud" | string;
  label: string;
  url: string;
}

export interface BioLink {
  text: string;
  artistId: string;
}

export interface InfoKeyState {
  saved: boolean;
  /** The service refused the key this session. */
  rejected: boolean;
}

export type InfoKeysStatus = Record<KeyedInfoSource, InfoKeyState>;

// ---------------------------------------------------------------- output devices

/** An audio output device on this computer. */
export interface OutputDevice {
  id: string;
  name: string;
  /** The system default output right now. */
  isDefault: boolean;
}

export interface OutputState {
  devices: OutputDevice[];
  /** The chosen device id; null follows the system default. */
  selected: string | null;
  /** The device playing, or the one the next play would open. */
  current: OutputDevice | null;
  /** How the open stream holds its device (bit-perfect output). */
  exclusive?: Exclusive;
}

/** Where the playing audio is read from. */
export type SignalSourceFrom = "server" | "cache" | "download";

/** What happens to the sound on its way to the output (`signal_path`, `audio://signal-path`). */
export interface SignalPath {
  trackId: string | null;
  source: {
    /** Upper case: "FLAC", "MP3". */
    codec: string | null;
    lossless: boolean;
    bitDepth: number | null;
    sampleRate: number | null;
    channels: number | null;
    /** kbps of the file itself (unknown for a transcode). */
    bitrate: number | null;
    transcoded: boolean;
    from: SignalSourceFrom | null;
  } | null;
  /** What the decoder hands on ("32-bit float"). */
  decode: string | null;
  loudness: { on: boolean; mode: "smart" | "track" | "album" | "off"; gainDb: number | null };
  eq: { eq: boolean; correction: boolean; correctionPreampDb: number | null; preampDb: number; limiter: boolean };
  resample: { fromRate: number; toRate: number; fromChannels: number; toChannels: number; quality: string } | null;
  blend: { automix: boolean; crossfade: "smart" | "fixed" | "off"; crossfadeMs: number | null };
  output: {
    device: string | null;
    mode: "shared" | "exclusive" | "fallback";
    reason: string | null;
    rate: number | null;
    channels: number | null;
    bits: number | null;
    network: boolean;
    /** Linear slider 0..1 (0 when muted). */
    volume: number;
    volumeBypassed: boolean;
  };
  verdict: { kind: "idle" | "bitPerfect" | "lossless" | "lossy"; text: string };
  /** "Why isn't this bit-perfect?": a line and the Settings row that changes it. */
  why: { text: string; setting: string | null } | null;
}

/** Bit-perfect state of the open output: shared, exclusive at the music's rate, or shared after an exclusive open failed. */
export type Exclusive = { state: "off" } | { state: "on"; rate: number; bits: number } | { state: "fallback"; reason: string };

/** One parametric filter of a headphone correction profile. */
export interface PeqFilter {
  kind: "peaking" | "lowShelf" | "highShelf";
  freq: number;
  gainDb: number;
  q: number;
}

/** A headphone in the bundled correction index (headphoneSearch). */
export interface HeadphoneEntry {
  name: string;
  /** Who measured it, and on what rig when they used several. */
  source: string;
  /** earbud, in-ear or over-ear. */
  form: string;
  /** Its place in the index; pass to headphoneCorrectionSet. */
  path: string;
}

/** A correction profile used whenever an output device plays. */
export interface DeviceCorrection extends HeadphoneEntry {
  deviceId: string;
  deviceName: string;
  preampDb: number;
  filters: PeqFilter[];
}

/** An output device that plays bit-perfect. */
export interface BitPerfectOutput {
  deviceId: string;
  deviceName: string;
}

// ---------------------------------------------------------------- listen on

/** What a player is, for its icon. "group" is a Cast speaker group. */
export type PlayerKind = "computer" | "phone" | "speaker" | "browser" | "tv" | "group";

/** How Platter reaches a device: another Platter computer, a Plex player, a Cast device, or an AirPlay receiver. */
export type PlayerVia = "platter" | "plex" | "cast" | "airplay" | "dlna";

/** A Plex player in the device picker. */
export interface RemotePlayer {
  /** Plex client identifier. */
  id: string;
  name: string;
  /** "Plexamp", "Plex for Windows", "Sonos"... */
  product: string;
  kind: PlayerKind;
  /** What it's playing now ("Song • Artist", "Paused: ..."). */
  playing: string | null;
  /** Seen on the network now. Others are listed but may not answer. */
  online: boolean;
  via: PlayerVia;
  /** An AirPlay receiver with a password Platter doesn't have yet. */
  needsPassword: boolean;
  /** Listed but can't be played on, and why. */
  unsupported: string | null;
  /** A Platter computer Plex says is on that doesn't answer directly
   * (another network, or a firewall on that computer). */
  unreachable: boolean;
}

export type RemoteStatus = "connecting" | "playing" | "paused" | "buffering";

/** Playback on another device. */
export interface RemoteSession {
  playerId: string;
  playerName: string;
  kind: PlayerKind;
  status: RemoteStatus;
  track: Track | null;
  positionMs: number;
  durationMs: number;
  /** 0..1 when the player reports it. */
  volume: number | null;
  shuffle: boolean;
  repeat: RepeatMode;
}

export interface RemoteState {
  session: RemoteSession | null;
  /** The device stopped by itself (someone stopped it there); controls are local again. */
  stoppedOn: string | null;
}

/** A player bar control while playing on another device. */
export type RemoteCommand =
  | { kind: "play" }
  | { kind: "pause" }
  | { kind: "next" }
  | { kind: "previous" }
  | { kind: "seek"; positionMs: number }
  | { kind: "volume"; volume: number }
  | { kind: "shuffle"; on: boolean }
  | { kind: "repeat"; mode: RepeatMode };

export interface ArtCacheInfo {
  usedBytes: number;
  capBytes: number;
}

export interface AppInfo {
  version: string;
  licence: string;
  repoUrl: string;
}

// ---------------------------------------------------------------- updates

export interface UpdateInfo {
  available: boolean;
  version?: string | null;
  notes?: string | null;
}

// ---------------------------------------------------------------- servers (docs/IPC.md "Servers")

/** What a music server can do. Features hide by these, never by the kind of server. */
export type CapName =
  | "liveUpdates"
  | "incrementalSync"
  | "likeDates"
  | "starRatings"
  | "playlistMove"
  | "playlistPoster"
  | "playlistSummary"
  | "serverSmartPlaylists"
  | "transcode"
  | "lyrics"
  | "syncedLyrics"
  | "serverRadio"
  | "sonic"
  | "playQueueHandoff"
  | "resumeQueue"
  | "remotePlayers"
  | "friendActivity"
  | "friendHistory"
  | "metadataEdit"
  | "homeUsers"
  | "realFilePaths"
  | "checkFiles"
  | "loudness"
  | "releaseTypes"
  | "serverHistory"
  | "backdatedScrobble"
  | "trackDetails";

export type Caps = Record<CapName, boolean>;

export type BackendKind = "plex" | "jellyfin" | "subsonic";

/** One server in use (backend_caps). */
export interface ServerCaps {
  /** The prefix its ids carry; null while not connected. */
  tag: string | null;
  kind: BackendKind;
  /** The name its owner gave it. */
  name: string | null;
  /** The server with the main library. */
  primary: boolean;
  caps: Caps;
}

/** `backend_caps`: what any server in use can do, and each server. */
export interface BackendCaps {
  caps: Caps;
  servers: ServerCaps[];
}

/** How a server added by address is doing. */
export type ServerState = "unknown" | "online" | "offline" | "signInAgain";

/** A server in Settings (`servers_saved`). */
export interface SavedServer {
  tag: string;
  kind: BackendKind;
  name: string;
  url: string | null;
  user: string | null;
  primary: boolean;
  /** Null for Plex, whose state the Account section shows. */
  state: ServerState | null;
  canTranscode: boolean;
  canDownload: boolean;
}

/** What an address turned out to be (`jellyfin_check`). */
export interface ServerCheck {
  /** The address, tidied. */
  url: string;
  name: string;
  version: string;
  quickConnect: boolean;
}

/** What an address turned out to be (`subsonic_check`). */
export interface SubsonicCheck {
  /** The address, tidied. */
  url: string;
  name: string;
  version: string | null;
  /** More than the original Subsonic API. */
  openSubsonic: boolean;
  /** It takes API keys. */
  apiKeys: boolean;
}

/** A queue another app saved on a server (`resume_queues`). */
export interface ResumeOffer {
  tag: string;
  serverName: string;
  trackCount: number;
  currentId: string;
  positionMs: number;
  /** The app that saved it. */
  changedBy: string | null;
  /** Unix seconds. */
  changedAt: number | null;
}

/** A Quick Connect code to show (`jellyfin_quick_start`). */
export interface QuickCode {
  code: string;
  serverName: string;
}

export type QuickPoll = { state: "waiting" } | { state: "done"; server: SavedServer };

/** A music library on a server and whether it's included. */
export interface ServerLibrary {
  id: string;
  title: string;
  included: boolean;
}

// ---------------------------------------------------------------- events

/** Every event in docs/IPC.md → its payload. */
export interface EventMap {
  "auth://changed": AuthStatus;
  /** The servers in use or what they can do changed: ask backend_caps again. */
  "backends://changed": Record<string, never>;
  /** A Home user switch went through; the app restarts as them. */
  "home://changed": { restarting: boolean };
  "sync://progress": SyncStatus;
  /** How far preparing Compare is: one version's file, 0 to 1. */
  "compare://progress": { side: CompareSide; fraction: number };
  "library://changed": null;
  /** Listening diary entries changed on this device. */
  "diary://changed": null;
  /** One artist's page data changed outside a sync (similar artists fetched). */
  "artist://changed": { id: string };
  /** New outside info arrived for an artist (artistInfo can be asked again). */
  "artistinfo://changed": { id: string };
  /** A fetch of the global Popular list finished, found or not. */
  "artistinfo://popular": { id: string };
  /** An album's MusicBrainz credits were looked up (found or not). */
  "albumcredits://changed": { id: string };
  /** Lookups an album's sleeve notes started have finished (ask sleeveNotes again). */
  "sleeve://changed": { id: string };
  "releasestatus://changed": { id: string };
  /** Release marks or the user's sections changed on another device. */
  "archive://changed": Record<string, never>;
  /** Library doctor findings or what's ignored changed: the badge count and whether a check is going. */
  "doctor://changed": { count: number; running: boolean };
  /** Owned albums or the Discogs import changed. */
  "physical://changed": Record<string, never>;
  /** Identify by sound found something or moved on (read sound_id_status). */
  "soundid://changed": null;
  /** Songs analysed for Automix (their ids). */
  "analysis://changed": string[];
  /** Points on the sound map were added, moved or removed. */
  "soundmap://changed": null;
  "player://state": PlayerState;
  "player://position": PlayerPosition;
  "player://error": PlayerError;
  /** Output devices or the device in use changed. */
  "output://changed": OutputState;
  /** What happens to the sound changed (track, device or sound settings). */
  "audio://signal-path": SignalPath;
  /** The chosen output device went away; playback paused and moved to the system default. */
  "output://lost": { device: string };
  /** Playback on another device started, changed, stopped or came back. */
  "remote://state": RemoteState;
  /** The Plex player list was refreshed. */
  "remote://players": RemotePlayer[];
  "health://changed": Health;
  toast: ToastEvent;
  "update://ready": { version: string };
  /** The core changed settings itself (the EQ followed an output device). */
  "settings://changed": Settings;
  /** These songs were liked or unliked (here, rolled back, or from Plex). */
  "likes://changed": { ids: string[] };
  /** A platter:// link was opened; take it with deepLinkTake. */
  "deeplink://open": null;
  /** Discord connected or went away, or a cover upload failed. */
  "discord://status": DiscordStatus;
  /** The now-playing feed started, stopped or couldn't open its port. */
  "feed://status": FeedStatus;
  /** The Stream Deck control API started, stopped, was paired or revoked. */
  "deck://status": DeckStatus;
  /** A Bridge paired, unpaired or sent news: read bridgeOverview again. */
  "bridge://changed": null;
  /** A Stream Deck key asked for Request album or Listen along. */
  "deck://action": DeckAction;
  /** The Stream Deck asked to pair (an id), or the prompt closed (null). */
  "deck://pair-request": { id: string | null };
  /** The file tag scan moved on, started, paused or finished. */
  "tags://progress": TagScanStatus;
  /** File tags changed: badges, editions and filters should be read again. */
  "tags://changed": null;
  /** Transcription progress, model downloads and failures. */
  "transcribe://status": TranscribeStatus;
  /** A song was transcribed (`found`: words came out). */
  "transcribe://done": { trackId: string; found: boolean };
  /** A song's lyrics were checked against the song: ask for them again. */
  "lyrics://checked": { trackId: string };
  /** A song's lyrics check moved on. */
  "lyrics://check-status": LyricsCheckStatus;
  /** Lyrics choices arrived from another device. */
  "lyrics://choices": null;
  /** Download progress, the queue's state and storage. */
  "downloads://status": DownloadStatus;
  /** The set of downloaded songs changed; ask downloadedTracks again. */
  "downloads://changed": null;
  /** A sync to a device moved on. */
  "devices://progress": DeviceSyncStatus;
  /** The device list changed. */
  "devices://changed": null;
  /** Mixes were made again (a new day, the next daylist, new settings). */
  "mixes://changed": null;
  /** The follow list changed. */
  "follows://changed": FollowedArtist[];
  /** How many What's New items are unseen. */
  "whatsnew://changed": { unseen: number };
  /** The release radar found something, a look started or ended, or auto-request changed. */
  "radar://changed": null;
  /** A listening session started, changed or ended. */
  "listen://state": ListenView;
  /** Something to tell the user about the session (a request the host turned down). */
  "listen://notice": { message: string };
  /** Shared playlists, Blends, invites or the relay changed. */
  "circle://changed": CircleView;
  /** Worth a toast: an edit the owner couldn't make, a deleted playlist, an invite. */
  "circle://notice": { message: string };
  /** The server invites you received changed. */
  "sharing://invites": ReceivedInvite[];
  /** Someone used your share link. */
  "sharing://notice": { message: string };
  /** The share link requests waiting for your yes changed. */
  "sharing://link-asks": ShareLinkAsk[];
  /** The sync file's state changed. */
  "sync-file://status": SyncFileStatus;
  /** Pins, folders or other app-only state changed (here or from the sync file). */
  "syncstore://changed": SyncStoreChanged;
  /** The Copy keys dialog's state (a code, keys arriving, expiry). */
  "keys://receive": KeyReceiveState;
  /** The hidden songs and artists changed. */
  "hidden://changed": HiddenItem[];
  /** Bookmarks changed (newest first). */
  "bookmarks://changed": Bookmark[];
  /** Platter smart playlists changed. */
  "smart://changed": SmartSummary[];
  /** Scrobbles waiting to be sent, or a service's sign-in changed. */
  "scrobble://status": { queued: number };
  /** Play history from Plex, Tautulli or Last.fm: a run started, moved on or ended. */
  "playhistory://status": PlayHistoryStatus;
  /** The miniplayer window opened or closed. */
  "miniplayer://changed": MiniplayerState;
  /** The miniplayer asked the main window to open a page. */
  "miniplayer://go": MiniplayerGo;
  /** A menu bar item was picked (macOS): the UI action to run. */
  "menu://action": { action: string };
}

export type EventName = keyof EventMap;

// ---------------------------------------------------------------- Edit in Plex (docs/IPC.md "Edit in Plex")

export type EditKind = "album" | "artist";
/** An album cover or artist photo ("poster"), or the background behind an artist ("art"). */
export type ImageKind = "poster" | "art";

/** What the UI may offer: the Settings switch and the servers this account can edit. */
export interface EditAccess {
  show: boolean;
  /** Tags of servers that can be edited. */
  servers: string[];
  /** Ids without a tag (the connected server's) can be edited. */
  connected: boolean;
  lockFields: boolean;
  matchUnreleased: boolean;
}

/** The fields Platter edits. `released` is YYYY or YYYY-MM-DD. */
export interface EditFields {
  title: string;
  sortTitle: string;
  released: string;
  studio: string;
  summary: string;
  genres: string[];
  styles: string[];
  moods: string[];
}

export interface EditInfo {
  id: string;
  kind: EditKind | null;
  fields: EditFields;
  /** Plex field names that are locked. */
  locked: string[];
  guid: string | null;
  thumb: string | null;
  art: string | null;
  artistTitle: string | null;
  sectionId: string | null;
  /** Matched to an online entry. */
  matched: boolean;
  /** Albums: counts as unreleased. */
  unreleased: boolean;
}

export interface ImageChoice {
  key: string;
  /** Art path for the preview. */
  preview: string | null;
  provider: string | null;
  selected: boolean;
}

export interface MatchCandidate {
  guid: string;
  name: string;
  /** The album's artist, or where an artist is from. */
  artist: string | null;
  year: number | null;
  score: number | null;
}

export type EditAction = "image" | "match" | "unmatch" | "details" | "merge" | "split" | "refresh";

export interface EditChange {
  id: string;
  itemId: string;
  kind: EditKind;
  action: EditAction;
  label: string;
  /** Unix ms. */
  at: number;
  undoable: boolean;
  undone: boolean;
}

// ---------------------------------------------------------------- discover (music that isn't in the library)

/** An artist marked "Not interested" (synced). */
export interface DismissedArtist {
  mbid: string;
  name: string;
}

/** An artist from MusicBrainz. */
export interface CatArtist {
  mbid: string;
  name: string;
  disambiguation: string | null;
  /** "Person", "Group"... */
  kind: string | null;
  photo: string | null;
  /** The library artist they are, if any. */
  libraryId: string | null;
  /** Where the photo is from, when it needs a credit. */
  photoCredit?: string | null;
}

/** `catalogue_artist_photo`. */
export interface CataloguePhoto {
  photo: string | null;
  credit: string | null;
}

export type CataloguePartKind = "artists" | "albums" | "songs";

/** `catalogue_search_part`: one kind of results, as it arrives. */
export interface CataloguePart {
  shown: boolean;
  part: CataloguePartKind;
  source: "lidarr" | "musicBrainz" | "";
  artists: CatArtist[];
  albums: CatAlbum[];
  songs: CatSong[];
  /** Library artists and albums a catalogue result folded into. */
  inLibrary: string[];
}

/** `catalogue_get_albums_plan`: what "Get albums" would queue, most popular first. */
export interface GetAlbumsPlan {
  artist: string;
  artistMbid: string;
  kinds: string[];
  albums: CatAlbum[];
}

/** A release group from MusicBrainz. */
export interface CatAlbum {
  releaseGroupId: string;
  title: string;
  artist: string;
  artistMbid: string | null;
  /** "album", "ep" or "single". */
  kind: string;
  secondary: string[];
  year: number | null;
  thumb: string | null;
  libraryAlbumId: string | null;
  request: LidarrRequestState | null;
  /** Why it can't be requested. */
  refusal: string | null;
  /** On the Missing list. */
  inMissing: boolean;
}

export interface CatSong {
  recordingId: string;
  title: string;
  artist: string;
  artistMbid: string | null;
  album: string | null;
  releaseGroupId: string | null;
  durationMs: number | null;
  thumb: string | null;
}

/** `catalogue_search`. */
export interface CatalogueSearch {
  /** The setting is on and Platter is online. */
  shown: boolean;
  artists: CatArtist[];
  albums: CatAlbum[];
  songs: CatSong[];
}

export interface CatSection {
  /** The section, with the library's section ids ("albums", "singlesEps", "compilations", "live"...). */
  kind: string;
  albums: CatAlbum[];
}

/** `featured_artist`: what the catalogue knows about a featured guest (a `credit:` page). */
export interface FeaturedArtist {
  /** Lookups ran (online, and MusicBrainz knows the name). */
  shown: boolean;
  mbid: string | null;
  photo: string | null;
  photoCredit: string | null;
  /** Their releases by section, newest first. */
  sections: CatSection[];
}

/** `catalogue_artist`. */
export interface CatalogueArtistPage {
  artist: CatArtist;
  area: string | null;
  lifeSpan: string | null;
  bio: string | null;
  bioCredit: string | null;
  bioUrl: string | null;
  banner: string | null;
  sections: CatSection[];
  similar: CatArtist[];
  dismissed: boolean;
  /** Their most popular albums and EPs. */
  popular?: CatAlbum[];
}

export interface CatTrack {
  disc: number | null;
  number: number;
  title: string;
  artist: string | null;
  durationMs: number | null;
}

export interface StoreLinkSet {
  bandcamp: string;
  qobuz: string;
  itunes: string;
}

/** `catalogue_album`. */
export interface CatalogueAlbumPage {
  album: CatAlbum;
  tracks: CatTrack[];
  released: string | null;
  label: string | null;
  links: StoreLinkSet;
  artistLibraryId: string | null;
}

export interface NewArtist {
  mbid: string;
  name: string;
  /** The artists played most that led here. */
  because: string[];
}

/** `new_to_you`. */
export interface NewToYou {
  shown: boolean;
  artists: NewArtist[];
}

/** An album added to the Missing list by hand. */
export interface WantedAlbum {
  releaseGroupId: string;
  title: string;
  artist: string;
  artistMbid?: string | null;
  addedAt?: number;
}

/** `import_missing_albums`. */
export interface MissingAlbum extends WantedAlbum {
  thumb: string | null;
  links: StoreLinkSet;
  libraryAlbumId: string | null;
}

// ---------------------------------------------------------------- physical collection

/** A format an album can be owned on. */
export type PhysicalFormat = "vinyl" | "cd" | "cassette" | "other";

/** Which Discogs collection item a copy came from. */
export interface DiscogsRef {
  releaseId: number;
  instanceId: number;
}

/** One copy the user owns. */
export interface OwnedCopy {
  format: PhysicalFormat;
  pressing: string;
  colour: string;
  condition: string;
  notes: string;
  discogs?: DiscogsRef;
}

/** `physical_list` item: an album the user owns, with its copies. */
export interface OwnedAlbum {
  albumId: string;
  title: string;
  artist: string;
  thumb: string | null;
  copies: OwnedCopy[];
  /** Each format once: vinyl, cd, cassette, other. */
  formats: PhysicalFormat[];
  /** When it was first marked (unix seconds). */
  at: number;
}

/** `physical_set` payload. No copies takes the mark off. */
export interface SetOwned {
  copies: OwnedCopy[];
  title: string;
  artist: string;
  thumb: string | null;
}

export interface ShelfAlbum {
  albumId: string;
  title: string;
  artist: string;
  thumb: string | null;
  plays: number;
  formats: PhysicalFormat[];
}

/** `physical_stats`: the shelf in numbers, on the shelf page. */
export interface ShelfStats {
  albums: number;
  copies: number;
  byFormat: { format: PhysicalFormat; albums: number; copies: number }[];
  mostPlayed: ShelfAlbum[];
  /** Albums played most that aren't owned; null while its setting is off. */
  missing: ShelfAlbum[] | null;
}

/** A record in the Discogs collection the library doesn't have. */
export interface ShelfOnly {
  releaseId: number;
  artist: string;
  title: string;
  year: number | null;
  format: PhysicalFormat;
  thumb: string | null;
  links: StoreLinkSet;
}

/** What the last Discogs import found. */
export interface DiscogsReport {
  username: string;
  importedAt: number;
  total: number;
  matched: number;
  added: number;
  unmatched: ShelfOnly[];
}

/** Where play history from before Platter and other apps comes from. */
export type PlayHistorySource = "plexHistory" | "tautulli" | "lastfm";

/** One source of imported play history (`play_history_status`). */
export interface PlayHistorySourceStatus {
  source: PlayHistorySource;
  /** Signed in to a Plex server; Tautulli's address and key saved; Last.fm connected. */
  setUp: boolean;
  /** Its Include setting. */
  included: boolean;
  /** Plays brought in (kept while the setting is off, but not counted). */
  plays: number;
  /** Of those, plays of songs that aren't in your library. */
  notInLibrary: number;
  /** Plays left out for being too short to count (Tautulli). */
  shortPlays: number;
  /** Unix ms of the last finished update. */
  updatedAt: number | null;
  /** The whole history has been read once. */
  firstReadDone: boolean;
  importing: boolean;
  /** Plays read so far in this update, and the total when known. */
  done: number;
  total: number | null;
  /** Why the last update stopped, in words. Tried again later. */
  error: string | null;
  /** Its plays were removed; nothing comes in until it's turned back on. */
  removed: boolean;
  /** You stopped a read; it goes on when you ask for an update. */
  paused: boolean;
}

export interface PlayHistoryStatus {
  sources: PlayHistorySourceStatus[];
}

/** `tautulli_status`. The key itself never comes back. */
export interface TautulliStatus {
  urlSet: boolean;
  hasKey: boolean;
}

/** `discogs_status`. The token itself never comes back. */
export interface DiscogsStatus {
  saved: boolean;
  username: string | null;
  importing: boolean;
  report: DiscogsReport | null;
}

// ---------------------------------------------------------------- Tag editor (docs/IPC.md "Tag editor")

/** A field the tag editor reads and writes (docs/TAGGING.md). */
export type TagField =
  | "albumArtist"
  | "album"
  | "year"
  | "genre"
  | "releaseType"
  | "edition"
  | "group"
  | "era"
  | "quality"
  | "explicit"
  | "trackNumber"
  | "trackTotal"
  | "discNumber"
  | "discTotal"
  | "title"
  | "artist"
  | "archiveVersion"
  | "status"
  | "picardStatus"
  | "customStatus"
  | "versionNote"
  | "archiveDate"
  | "source"
  | "flags"
  | "isrc";

/** A file's fields that have a value. */
export type TagValues = Partial<Record<TagField, string>>;

/** Where an album keeps its release status. */
export type TagScheme = "archive" | "plain" | "picard" | "custom";

export interface TagEditFile {
  trackId: string;
  albumId: string;
  serverPath: string | null;
  localPath: string | null;
  fileName: string;
  format: "flac" | "oggVorbis" | "opus" | "mp3" | "mp4" | null;
  id3Version: "2.3" | "2.4" | null;
  values: TagValues;
  keys: string[];
  /** Present when the file was read for editing. */
  stamp: string | null;
  /** Why it couldn't be read. */
  error: string | null;
  durationMs: number;
}

export interface TagEditorData {
  title: string;
  files: TagEditFile[];
  scheme: TagScheme;
  statusField: TagField;
  statusChoices: string[];
  customKey: string | null;
  writable: boolean;
  reason: string | null;
  /** No music folder: Save becomes "Set up music folder". */
  needsMusicFolder: boolean;
}

export interface TagSaveFile {
  trackId: string;
  stamp: string;
  /** New values; "" removes the tag. */
  edits: TagValues;
}

export interface TagSaveRequest {
  label: string;
  files: TagSaveFile[];
}

export type TagRefusal = "changed" | "busy" | "readOnly" | "locked" | "gone" | "failed";

export interface TagFileProblem {
  trackId: string;
  fileName: string;
  why: TagRefusal;
  message: string;
}

export interface TagSaveResult {
  saved: number;
  unchanged: number;
  problems: TagFileProblem[];
  /** "Saved 12 files", or why nothing was saved. */
  message: string;
  undoId: string | null;
  rescanned: string[];
  rescanError: string | null;
}

export interface TagUndoInfo {
  id: string;
  at: number;
  label: string;
  files: number;
}

export interface MusicFolderMapping {
  server: string;
  local: string;
}

export interface MusicFolderCheck {
  ok: boolean;
  found: number;
  checked: number;
  /** "Connected, 5 of 5 files found" or what's wrong. */
  message: string;
  mapping: MusicFolderMapping | null;
  probes: { serverPath: string; localPath: string | null; found: boolean; sizeDiffers: boolean }[];
}

export type TagCheckScope = { kind: "album"; id: string } | { kind: "artist"; id: string } | { kind: "library" };

export type TagRule =
  | "missingAlbumArtist"
  | "albumTagsDiffer"
  | "releaseTypeHalf"
  | "lonelyGroup"
  | "splitAlbum"
  | "unknownArchiveValue"
  | "discTags"
  | "statusMissing"
  | "noteMissing"
  | "artistStyle"
  | "featPlacement"
  | "numberGaps"
  | "duplicateTitle"
  | "filenamePattern"
  | "numberVsFilename"
  | "titleVsFilename"
  | "strayTags"
  | "shortTrack"
  | "lowBitrate"
  | "missingIsrc";

export interface TagFinding {
  id: string;
  rule: TagRule;
  severity: "error" | "warning" | "info";
  albumId: string;
  albumTitle: string;
  trackIds: string[];
  what: string;
  why: string;
  /** Opens the editor on these songs; null without a music folder. */
  fix: { albumId: string; trackIds: string[]; field: TagField | null } | null;
}

export interface TagCheckReport {
  findings: TagFinding[];
  styles: { albumId: string; text: string }[];
  albums: number;
  files: number;
  ignored: number;
  fromFiles: boolean;
}

export type TagFixKind = "discTags" | "releaseStatus" | "isrc" | "explicit";

export interface TagFixChange {
  trackId: string;
  fileName: string;
  stamp: string;
  field: TagField;
  before: string | null;
  after: string;
  note: string | null;
}

export interface TagFixPreview {
  kind: TagFixKind;
  label: string;
  changes: TagFixChange[];
  kept: { trackId: string; fileName: string; reason: string }[];
  /** Changes start unticked: the user picks each one. */
  pickEach: boolean;
  message: string;
}

export interface IsrcRelease {
  source: "musicbrainz" | "deezer";
  id: string;
  title: string;
  artist: string;
  date: string | null;
  country: string | null;
  tracks: number;
}

export interface IsrcReleases {
  candidates: IsrcRelease[];
  wanted: number;
  message: string;
}

export interface PlexLibraryCheck {
  id: string;
  title: string;
  agent: string;
  items: { label: string; ok: boolean | null; help: string }[];
  ok: boolean;
}

// ---------------------------------------------------------------- Platter Bridge

/** How a Bridge reaches the music folder. */
export type BridgeMusicAccess = "none" | "readOnly" | "readWrite";

/** A Bridge's last catch-up with Lidarr, in short. */
export interface BridgeCatchUp {
  /** Unix seconds. */
  at: number;
  handled: number;
  acted: number;
  scanned: number;
  announced: string[];
  healed: number;
  paused: string | null;
  error: string | null;
}

/** What a Bridge last said about itself. */
export interface BridgeStatus {
  name: string;
  version: string;
  /** Unix seconds. */
  at: number;
  /** One plain line: what it's doing now. */
  doing: string;
  lastCatchUp: BridgeCatchUp | null;
  music: BridgeMusicAccess;
  plex: boolean;
  lidarr: boolean;
  problem: string | null;
}

/** A paired Bridge. */
export interface BridgeInfo {
  id: string;
  name: string;
  version: string;
  pairedAt: number;
  lastSeen: number | null;
  /** Heard from in the last hour or so. */
  online: boolean;
  status: BridgeStatus | null;
  music: BridgeMusicAccess;
}

/** A pair code waiting for its Bridge. */
export interface BridgePairStart {
  code: string;
  /** Unix seconds. */
  expiresAt: number;
}

/** A Lidarr settings change a Bridge didn't make and queued for a yes. */
export interface BridgeQueuedWrite {
  id: string;
  what: string;
  method: string;
  path: string;
  body: string;
  at: number;
  device: string;
}

/** Settings > Bridge. */
export interface BridgeOverview {
  bridges: BridgeInfo[];
  pairing: BridgePairStart | null;
  queued: BridgeQueuedWrite[];
}

// ---------------------------------------------------------------- shared playlists and Blends

/** A role on a shared playlist. A follower left a public one and still sees it. */
export type CollabRole = "owner" | "collaborator" | "listener" | "follower";

export interface CirclePerson {
  id: string;
  name: string;
  /** Art path for the round picture (null: the name's letter). */
  avatar: string | null;
  role: CollabRole | null;
  /** This is you. */
  you: boolean;
  /** Songs they added. */
  added: number;
}

/** A shared playlist: yours (id is the Plex playlist's) or someone else's (`shared:<group>`). */
export interface SharedSummary {
  id: string;
  group: string;
  title: string;
  owner: CirclePerson;
  /** Your role. */
  role: CollabRole;
  /** The owner first, then everyone it's shared with. */
  people: CirclePerson[];
  public: boolean;
  /** Deleted by its owner, or you were taken off. */
  gone: boolean;
  inLibrary: boolean;
  /** Your edits the owner hasn't confirmed yet. */
  waiting: number;
  /** Your edits were sent a few times without an answer. */
  ownerUnreachable: boolean;
  trackCount: number;
}

export interface BlendSummary {
  /** `blend:<group>`. */
  id: string;
  /** Members' names joined with " + ". */
  title: string;
  description: string;
  /** You first. */
  people: CirclePerson[];
  trackCount: number;
}

/** An invite that came straight to you (someone on your server or Home). */
export interface CircleIncoming {
  id: string;
  from: string;
  kind: "playlist" | "blend";
  title: string;
  at: number;
}

/** `circle_state` and `circle://changed`. */
export interface CircleView {
  relay: "connecting" | "ok" | "unreachable" | "limited";
  playlists: SharedSummary[];
  blends: BlendSummary[];
  incoming: CircleIncoming[];
  /** Invites sent and waiting for their owner's answer. */
  joining: number;
}

/** Someone on your server or Plex Home you can invite without a link. */
export interface CircleAccount {
  key: string;
  name: string;
  avatar: string | null;
  /** In your Plex Home. */
  home: boolean;
}

export interface InviteLink {
  link: string;
  /** Unix ms. */
  expiresAt: number;
}

export type JoinResult = { state: "joined"; id: string; kind: "playlist" | "blend" } | { state: "waiting"; kind: "playlist" | "blend" };

/** The owner's view of who added what on their shared playlist. */
export interface SharedInfo {
  summary: SharedSummary;
  /** Row (item) id → who added it. Rows the owner added aren't listed. */
  addedBy: Record<string, CirclePerson>;
}

/** One row of a playlist shared with you. */
export interface SharedRow {
  key: string;
  /** The copy you can play; null: Not in your library. */
  track: Track | null;
  title: string;
  artist: string;
  album: string | null;
  durationMs: number;
  releaseGroupId: string | null;
  by: CirclePerson | null;
  addedAt: number;
  /** Your change, not confirmed by the owner yet. */
  pending: boolean;
  /** You added it, so you may remove it. */
  removable: boolean;
}

export interface SharedPage {
  summary: SharedSummary;
  description: string | null;
  durationMs: number;
  rows: SharedRow[];
}

export interface BlendRow {
  track: Track;
  by: CirclePerson;
}

export interface BlendMissing {
  title: string;
  artist: string;
  album: string | null;
  releaseGroupId: string | null;
  by: CirclePerson;
}

export interface BlendPage {
  summary: BlendSummary;
  rows: BlendRow[];
  /** Members' songs that aren't in your library. */
  missing: BlendMissing[];
  durationMs: number;
}

export interface BlendCreated {
  id: string;
  invite: InviteLink;
}
