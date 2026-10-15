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
  /** Plex ratingCount; absent when it has none. Not on one scale: only breaks ties in Popular. */
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
}

export interface AlbumPage {
  album: Album;
  tracks: Track[];
  totalDurationMs: number;
  moreByArtist: Album[];
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
export type ThemeSetting = "dark" | "light" | "system" | "midnight" | "forest" | "sand";

/** A theme file that passed the checks (also what the `themeFile` setting holds). */
export interface ThemeFile {
  /** The file's name in the themes folder. */
  file: string;
  name: string;
  base: "dark" | "light";
  tokens: Record<string, string>;
}

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
  /** The speech model transcription uses (this device only). */
  transcribeModel: TranscribeModel;
  /** Show lyrics transcribed on this computer. */
  showAutoLyrics: boolean;
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
  /** Home shows the made-for-you and mood and genre mix shelves. */
  homeMixes: boolean;
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
  /** The miniplayer window stays above other windows (this device only). */
  miniplayerOnTop: boolean;
  /** The miniplayer's card takes a colour from the cover (this device only). */
  miniplayerTint: boolean;
  /** The miniplayer lists what's up next when it's tall enough (this device only). */
  miniplayerQueue: boolean;
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
  /** Library import sources turned on ("streaming", "transfer", "lists"). All off until picked. */
  importSources: string[];
  /** Dark, Light, Match system, or another built-in theme. */
  theme: ThemeSetting;
  /** A preset id ("ivory" is the default) or a custom colour as #RRGGBB. */
  accent: string;
  /** The theme file in use, carried whole so it travels in the sync file. */
  themeFile: ThemeFile | null;
  [key: string]: unknown;
}

/** `feed_status` and the `feed://status` event: the now-playing feed for widgets. */
export type FeedStatus =
  | { state: "off" }
  | { state: "on"; port: number; url: string }
  | { state: "portInUse"; port: number }
  | { state: "failed"; port: number; message: string };

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

export type DownloadQuality = "original" | "320" | "192" | "128";

export type MixVariety = "balanced" | "familiar" | "adventurous";
export type QueueEnd = "keepPlaying" | "stop";

export type ExplicitContent = "allowed" | "filtered";

export type CrossfadeMode = "smart" | "fixed" | "off";
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
  infoListenbrainz: true,
  infoMusicbrainz: true,
  infoListenbrainzStats: true,
  linkFeatured: true,
  likedMeans: "five",
  likeControlRows: "hover",
  showPlexPlayers: true,
  actAsPlexPlayer: false,
  moveStopsOldDevice: true,
  crossfade: "smart",
  crossfadeSeconds: 6,
  crossfadeSkipAlbums: true,
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
  sharePublicLinks: true,
  shareCardQr: true,
  shareCardSongLink: true,
  showCastDevices: true,
  discordStatus: true,
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
  explicitContent: "allowed",
  showFanEdits: false,
  countUnmatchedUnreleased: true,
  checkUnmatchedMusicbrainz: true,
  groupEditions: true,
  plexEditShow: true,
  plexEditLock: true,
  plexEditMatchUnreleased: true,
  dragAndDrop: true,
  transcribeLyrics: false,
  transcribeModel: "small",
  showAutoLyrics: true,
  showToMyDevices: true,
  computerName: "",
  showAirplayLocked: false,
  downloadQuality: "original",
  downloadLimitGb: 20,
  downloadFolder: "",
  downloadHomeOnly: true,
  downloadLiked: false,
  goOffline: false,
  homeMixes: true,
  mixVariety: "balanced",
  mixExcludeArtists: [],
  mixExcludeGenres: [],
  mixExcludeAlbums: [],
  mixUnreleased: true,
  daylistHours: 3,
  queueEnd: "keepPlaying",
  smartShuffle: true,
  showFriendActivity: true,
  shareActivity: true,
  whatsNewAnnouncements: true,
  notifyNewReleases: false,
  listenGuestsControl: true,
  listenRelay: "",
  listenInviteHours: 24,
  listenName: "",
  listenHideUnreleased: false,
  discordListenAlong: true,
  syncEnabled: false,
  syncFolder: "",
  syncSettings: true,
  syncLibrary: true,
  syncFollows: true,
  syncLyrics: true,
  bookmarksInLibrary: true,
  scrobbleLastfm: false,
  scrobbleListenbrainz: false,
  scrobbleUnreleased: false,
  aiProvider: "none",
  aiModel: "claude-opus-5-5",
  lidarrUrl: "",
  lidarrAwayUrl: "",
  miniplayerOnTop: true,
  miniplayerTint: true,
  miniplayerQueue: false,
  miniplayerBounds: null,
  powerKeys: false,
  hintKey: "f",
  nowPlayingFeed: false,
  nowPlayingFeedPort: 32540,
  importSources: [],
  theme: "dark",
  accent: "ivory",
  themeFile: null,
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
  kind: "album" | "artist" | "playlist";
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
export type DownloadPause = "offline" | "notHome" | "full" | "noSpace";

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
export type TranscribeModel = "small" | "medium";

export interface TranscribeModelInfo {
  kind: TranscribeModel;
  sizeBytes: number;
  installed: boolean;
  downloading: boolean;
  doneBytes: number;
}

export type TranscribeStage = "waitingForModel" | "reading" | "decoding" | "transcribing";

export interface TranscribeStatus {
  enabled: boolean;
  /** This build and this computer can transcribe. */
  available: boolean;
  unavailableReason?: string;
  model: TranscribeModel;
  models: TranscribeModelInfo[];
  current?: { trackId: string; stage: TranscribeStage; percent: number; background: boolean };
  queued: number;
  batch: { done: number; total: number };
  paused?: "battery" | "slowConnection" | "offline";
  error?: { kind: "download" | "checksum" | "disk" | "transcribe"; message: string; trackId?: string };
}

// ---------------------------------------------------------------- file tags

/** Badges for one song from its file's tags (docs/IPC.md "File tags"). */
export interface TrackBadges {
  explicit?: boolean;
  /** ARCHIVE_VERSION, lower case ("leak", "demo", "released"...). */
  version?: string;
  /** ARCHIVE_QUALITY, lower case ("lossless", "cdq", "lq", "snippet"). */
  quality?: string;
  /** A fan or AI edit. */
  edit?: boolean;
}

/** What the file tags say about one album. */
export interface AlbumBadges {
  explicit?: boolean;
  version?: string;
  /** ARCHIVE_GROUP: albums sharing it are editions of one album. */
  group?: string;
  /** ARCHIVE_EDITION label ("Standard", "Deluxe", "Build v2"). */
  edition?: string;
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
}

// ---------------------------------------------------------------- social

/** One person's latest listen on the Plex server. */
export interface FriendActivity {
  accountId: number;
  name: string;
  /** Art path for the round avatar. */
  thumb: string | null;
  trackId: string;
  title: string;
  artist: string;
  albumId: string | null;
  album: string;
  cover: string | null;
  /** Unix seconds. */
  at: number;
  /** Listening right now. */
  now: boolean;
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
  | "version";
export type SmartOp = "is" | "isNot" | "contains" | "notContains" | "gt" | "lt" | "inLast" | "notInLast";

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
}

/** `lidarr_check`: the read-only setup check. */
export interface LidarrReport {
  version: string;
  /** Red first, then amber, then OK. */
  findings: LidarrFinding[];
  /** No red findings. */
  passed: boolean;
  checkedAt: number;
  /** Which address answered. */
  route: "home" | "away" | null;
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
}

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
export type ReleaseStatus = "taggedUnreleased" | "probablyUnreleased" | "plexMissed" | "released";

/** `album_release_status`; `releasestatus://changed` fires when it changes. */
export interface AlbumReleaseStatus {
  id: string;
  status: ReleaseStatus;
  /** Plain sentences for "Why?". */
  reasons: string[];
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
}

// ---------------------------------------------------------------- listen on

/** What a player is, for its icon. "group" is a Cast speaker group. */
export type PlayerKind = "computer" | "phone" | "speaker" | "browser" | "tv" | "group";

/** How Platter reaches a device: another Platter computer, a Plex player, a Cast device, or an AirPlay receiver. */
export type PlayerVia = "platter" | "plex" | "cast" | "airplay";

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

// ---------------------------------------------------------------- events

/** Every event in docs/IPC.md → its payload. */
export interface EventMap {
  "auth://changed": AuthStatus;
  /** A Home user switch went through; the app restarts as them. */
  "home://changed": { restarting: boolean };
  "sync://progress": SyncStatus;
  "library://changed": null;
  /** One artist's page data changed outside a sync (similar artists fetched). */
  "artist://changed": { id: string };
  /** New outside info arrived for an artist (artistInfo can be asked again). */
  "artistinfo://changed": { id: string };
  /** A fetch of the global Popular list finished, found or not. */
  "artistinfo://popular": { id: string };
  /** An album's MusicBrainz credits were looked up (found or not). */
  "albumcredits://changed": { id: string };
  "releasestatus://changed": { id: string };
  "player://state": PlayerState;
  "player://position": PlayerPosition;
  "player://error": PlayerError;
  /** Output devices or the device in use changed. */
  "output://changed": OutputState;
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
  /** The file tag scan moved on, started, paused or finished. */
  "tags://progress": TagScanStatus;
  /** File tags changed: badges, editions and filters should be read again. */
  "tags://changed": null;
  /** Transcription progress, model downloads and failures. */
  "transcribe://status": TranscribeStatus;
  /** A song was transcribed (`found`: words came out). */
  "transcribe://done": { trackId: string; found: boolean };
  /** Download progress, the queue's state and storage. */
  "downloads://status": DownloadStatus;
  /** The set of downloaded songs changed; ask downloadedTracks again. */
  "downloads://changed": null;
  /** Mixes were made again (a new day, the next daylist, new settings). */
  "mixes://changed": null;
  /** The follow list changed. */
  "follows://changed": FollowedArtist[];
  /** How many What's New items are unseen. */
  "whatsnew://changed": { unseen: number };
  /** A listening session started, changed or ended. */
  "listen://state": ListenView;
  /** Something to tell the user about the session (a request the host turned down). */
  "listen://notice": { message: string };
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
  /** The miniplayer window opened or closed. */
  "miniplayer://changed": MiniplayerState;
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
