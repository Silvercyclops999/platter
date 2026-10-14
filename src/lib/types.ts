/**
 * TypeScript mirror of `src-tauri/src/model.rs` (canonical) and every IPC
 * type in `docs/IPC.md`. Wire format is camelCase JSON. Rust `Option<T>`
 * serializes as `null`, so model fields are `T | null`; IPC types documented
 * with `?` are optional and may also be null.
 */

// ---------------------------------------------------------------- model.rs

export type ReleaseSection =
  | "albums"
  | "singlesEps"
  | "live"
  | "compilations"
  | "soundtracks"
  | "remixes"
  | "demos"
  | "mixtapes";

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
  /** Global listener count from Plex; absent when it has none. */
  ratingCount?: number;
  /** When the user last rated it, unix seconds (for a liked song: when it was liked). */
  ratedAt?: number | null;
  /** Plex genre and mood tags on the track itself. */
  genres?: string[];
}

/** A library artist's name and id (`artist_names`). */
export interface ArtistName {
  id: string;
  title: string;
}

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

export interface AuthStatus {
  state: "signedOut" | "pending" | "signedIn";
  username?: string | null;
  server?: ServerInfo | null;
  needsServerPick: boolean;
  /** Plex rejected the saved sign-in: keep the app open with a sign-in banner. */
  reauthRequired?: boolean;
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

export interface HomeData {
  /** Up to 8 shortcut tiles: recent first, then most played, de-duplicated. */
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
}

export interface AlbumPage {
  album: Album;
  tracks: Track[];
  totalDurationMs: number;
  moreByArtist: Album[];
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
  /** The source had timestamps (shown as plain text for now). */
  synced: boolean;
  /** Lines without timestamps; "" marks a break between verses. */
  lines: string[];
  /** Writer and provider credit to show under the lyrics. */
  credit?: string;
}

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
}

// ---------------------------------------------------------------- playback & queue

/** "liked" is Liked Songs (id "liked"). */
export type PlayContextKind = "album" | "playlist" | "artist" | "tracks" | "search" | "liked";

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
  source: "user" | "context";
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
  | "diskFull";

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
/** Which Plex star ratings count as liked ("five": rating 10; "fourUp": 8 and up). */
export type LikedMeans = "five" | "fourUp";
/** When track rows show the like control. */
export type LikeControlRows = "hover" | "always" | "off";

export interface Settings {
  privateSession: boolean;
  reportToPlex: boolean;
  volume: number;
  shuffleStyle: ShuffleStyle;
  qualityHome: StreamQuality;
  qualityAway: StreamQuality;
  showFormatBadge: boolean;
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
  /** Which star ratings count as liked. Liking always writes five stars. */
  likedMeans: LikedMeans;
  /** When track rows show the like control (liked songs always show it unless "off"). */
  likeControlRows: LikeControlRows;
  [key: string]: unknown;
}

export const DEFAULT_SETTINGS: Settings = {
  privateSession: false,
  reportToPlex: true,
  volume: 0.7,
  shuffleStyle: "spread",
  qualityHome: "original",
  qualityAway: "320",
  showFormatBadge: true,
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
  infoWikipedia: true,
  infoLastfm: false,
  infoFanart: false,
  infoAudiodb: false,
  likedMeans: "five",
  likeControlRows: "hover",
};

// ---------------------------------------------------------------- artist info

/** An outside artist info source. */
export type InfoSource = "wikipedia" | "lastfm" | "fanart" | "audiodb";
/** A source that needs the user's API key. */
export type KeyedInfoSource = Exclude<InfoSource, "wikipedia">;

export interface InfoMember {
  name: string;
  /** Set when the member is an artist in this library. */
  artistId: string | null;
}

/** A "where this came from" line in the About dialog. */
export interface InfoCredit {
  source: "wikipedia" | "wikidata" | "lastfm" | "fanart" | "audiodb";
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
  /** Last.fm listeners and plays. */
  listeners: number | null;
  scrobbles: number | null;
  /** Art path for a photo, only when Plex has none. */
  photo: string | null;
  /** Art path for the header banner, only when Plex has no background art. */
  banner: string | null;
  logo: string | null;
  credits: InfoCredit[];
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
}

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

// ---------------------------------------------------------------- events

/** Every event in docs/IPC.md → its payload. */
export interface EventMap {
  "auth://changed": AuthStatus;
  "sync://progress": SyncStatus;
  "library://changed": null;
  /** One artist's page data changed outside a sync (similar artists fetched). */
  "artist://changed": { id: string };
  /** New outside info arrived for an artist (artistInfo can be asked again). */
  "artistinfo://changed": { id: string };
  "player://state": PlayerState;
  "player://position": PlayerPosition;
  "player://error": PlayerError;
  /** Output devices or the device in use changed. */
  "output://changed": OutputState;
  /** The chosen output device went away; playback paused and moved to the system default. */
  "output://lost": { device: string };
  "health://changed": Health;
  toast: ToastEvent;
  "update://ready": { version: string };
  /** These songs were liked or unliked (here, rolled back, or from Plex). */
  "likes://changed": { ids: string[] };
}

export type EventName = keyof EventMap;
