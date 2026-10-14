/**
 * In-memory mock of the Rust core, used outside Tauri (Vite dev server,
 * Playwright, Vitest). Implements every command and event in docs/IPC.md with
 * invented fixture data, simulated playback (position ticks 4×/s, the same
 * queue rules as the core: Previous by context position, resume points,
 * per-context shuffle, context end rewinds, repeat-one drops), sync progress
 * and health changes.
 *
 * URL switches (plain browser only):
 *   ?latency=ms     delay every command (default 120; 0 in tests)
 *   ?signedOut=1    start on the sign-in screen
 *   ?albums=5000    pad the album list to N items (virtual grid perf check)
 *   ?httpArt=1      art URLs in the Windows app's form (http://art.localhost/…?w=&h=)
 *                   instead of generated SVGs, for memory checks that serve
 *                   real JPEGs at the requested size (scripts/measure-memory.mjs)
 *   ?update=1       "Check for updates" finds a new version (ready to install)
 *   ?link=<url>     open a platter:// link a moment after start, as if the
 *                   OS had handed it over (url-encoded)
 *   ?health=offline start in a given app state (see HEALTH_DEMOS in ./health:
 *                   offline, noInternet, starting, updating, startTimeout,
 *                   relay, remote, signIn, accessRemoved, filesUnavailable,
 *                   noMusicLibrary, diskFull, scanning)
 */
import type { Backend, Unlisten } from "../backend";
import type {
  AbLoop,
  AddPlace,
  Album,
  AlbumSort,
  ArtistSort,
  AuthStatus,
  EventMap,
  EventName,
  Health,
  HomeShelfId,
  KeyedInfoSource,
  LibraryItem,
  OutputDevice,
  OutputState,
  PinInfo,
  PlayContextArgs,
  PlayerState,
  QueueContextInfo,
  QueueItem,
  RemoteCommand,
  RepeatMode,
  ReleaseSection,
  SearchArgs,
  SearchHit,
  SearchKind,
  SearchResults,
  ServerInfo,
  Settings,
  ShareKind,
  SyncStatus,
  Track,
} from "../types";
import { DEFAULT_SETTINGS } from "../types";
import { buildArtistLookup } from "../credits";
import { mockArtUrl } from "./art";
import { mockNowPlayingDetails } from "./details";
import { popularTracks } from "./popular";
import { BROKEN_TRACK_TITLE_SUFFIX, buildFixtures, mulberry32, type Fixtures } from "./fixtures";
import { mockBio } from "./bio";
import { mockArtistInfo, mockKeysStatus, mockSetKey, type MockInfoKeys } from "./artistInfo";
import { demoHealth, isHealthDemo, type HealthDemo } from "./health";
import * as eq from "./eq";
import { MockHome, type ContextPlay } from "./home";
import { MockLikes } from "./likes";
import { MockPlaylists } from "./playlists";
import { MockRemote } from "./remote";
import { MockShare } from "./share";

export interface MockOptions {
  latencyMs: number;
  signedOut: boolean;
  albumCount?: number;
  /** Art URLs in the app's http://art.localhost form (see the header). */
  httpArt?: boolean;
  /** Run startup timelines (health "starting" → "local", incremental sync). */
  timelines: boolean;
  health?: HealthDemo;
  /** Auto-approve the sign-in PIN after this many ms (null = never). */
  autoApproveMs: number | null;
  /** "Check for updates" finds a new version. */
  updateAvailable?: boolean;
  /** A platter:// link to open after start. */
  link?: string;
}

const isTestEnv = (() => {
  try {
    return import.meta.env?.MODE === "test";
  } catch {
    return false;
  }
})();

/** Same cap as the core's CONTEXT_MEMORY_CAP. */
const CONTEXT_MEMORY_CAP = 200;
/** Previous restarts the track at or past this point. */
export const PREVIOUS_RESTART_MS = 3000;

interface ResumePoint {
  index: number;
  trackId: string;
  positionMs: number;
}

export function optionsFromUrl(search: string): MockOptions {
  const p = new URLSearchParams(search);
  return {
    latencyMs: p.has("latency") ? Number(p.get("latency")) : isTestEnv ? 0 : 120,
    signedOut: p.get("signedOut") === "1",
    albumCount: p.has("albums") ? Number(p.get("albums")) : undefined,
    httpArt: p.get("httpArt") === "1",
    timelines: !isTestEnv,
    health: isHealthDemo(p.get("health")) ? (p.get("health") as HealthDemo) : undefined,
    autoApproveMs: isTestEnv ? null : 6000,
    updateAvailable: p.get("update") === "1",
    link: p.get("link") ?? undefined,
  };
}

const SERVER: ServerInfo = {
  machineId: "mock-machine-01",
  name: "Basement Server",
  owned: true,
  online: true,
  version: "1.42.2",
  connection: "local",
};

/** Invented audio outputs; the first is the system default. */
const MOCK_OUTPUTS: OutputDevice[] = [
  { id: "mock:speakers", name: "Speakers (Built-in Audio)", isDefault: true },
  { id: "mock:headphones", name: "Headphones (USB Audio DAC)", isDefault: false },
  { id: "mock:hdmi", name: "Living Room Receiver (HDMI)", isDefault: false },
];

const clone = <T,>(v: T): T => (typeof structuredClone === "function" ? structuredClone(v) : JSON.parse(JSON.stringify(v)));

export class MockBackend implements Backend {
  readonly kind = "mock" as const;
  readonly fx: Fixtures;
  /** Playlist editing (test hooks: refuseNextWrite, deleteElsewhere). */
  readonly pl: MockPlaylists;
  /** Likes (test hooks: refuseNextWrite, likeElsewhere, clearAll). */
  readonly likes: MockLikes;
  /** Listen on (test hooks: stopElsewhere, loseRemote, setPlayers). */
  readonly remote: MockRemote;
  /** Sharing (test hooks: openLink, failNext). */
  readonly share: MockShare;
  private opts: MockOptions;
  private listeners = new Map<EventName, Set<(p: unknown) => void>>();
  private timers = new Set<ReturnType<typeof setTimeout>>();

  // auth
  private auth: AuthStatus;
  // health / sync
  private healthState: Health;
  private sync: SyncStatus = { phase: "idle", done: 0, total: 0, lastSyncAt: Math.floor(Date.now() / 1000) - 3600 };
  // search
  /** Items opened from search, newest first. */
  private recent: { kind: SearchKind; id: string }[] = [];
  private recentSeeded = false;
  private settings: Settings = { ...DEFAULT_SETTINGS };
  // output devices (invented)
  private outputs: OutputDevice[] = MOCK_OUTPUTS.map((d) => ({ ...d }));
  private outputSelected: string | null = null;
  /** Bytes in the (pretend) artwork cache. */
  private artUsed = 412 * 1024 * 1024;
  /** Outside artist info: keys entered in Settings, and artists already "fetched". */
  private infoKeys: MockInfoKeys = { keys: {}, rejected: new Set() };
  private infoFetched = new Set<string>();

  // player
  private status: PlayerState["status"] = "idle";
  private positionMs = 0;
  private volume = 0.7;
  private muted = false;
  private shuffle = false;
  private repeat: RepeatMode = "off";
  /** A-B loop on the current track. */
  private abLoop: AbLoop | null = null;
  private current: QueueItem | null = null;
  private userQueue: QueueItem[] = [];
  private ctxOriginal: QueueItem[] = [];
  private ctxOrder: QueueItem[] = [];
  private ctxPos = -1;
  private history: QueueItem[] = [];
  /** Plays started from an album, artist or playlist (feeds Home). */
  private contextPlays: ContextPlay[] = [];
  private ctxInfo: QueueContextInfo | null = null;
  /** Context ids with shuffle on, least recently used first. */
  private shuffledIds: string[] = [];
  /** Resume point per context id; Map order is least recently used first. */
  private resumePoints = new Map<string, ResumePoint>();
  private itemSeq = 1;
  private tick: ReturnType<typeof setInterval> | null = null;
  private rng = mulberry32(42);

  constructor(opts: Partial<MockOptions> = {}) {
    this.opts = { ...optionsFromUrl(""), ...opts };
    this.fx = buildFixtures();
    this.pl = new MockPlaylists(
      this.fx,
      (id) => this.trackById(id),
      () => this.emit("library://changed", null),
      (message) => this.emit("toast", { kind: "error", message }),
    );
    this.likes = new MockLikes(
      this.fx,
      () => this.settings.likedMeans,
      (ids) => this.emit("likes://changed", { ids }),
      (message) => this.emit("toast", { kind: "error", message }),
    );
    this.contextPlays = this.seedContextPlays();
    this.share = new MockShare(
      this.fx,
      () => this.settings,
      (id) => this.fx.playlists.some((p) => p.id === id),
      () => this.emit("deeplink://open", null),
    );
    const link = this.opts.link;
    if (link) this.later(1500, () => this.share.openLink(link));
    this.remote = new MockRemote(
      {
        local: () => this.snapshot(),
        pauseLocal: () => this.pauseNow(),
        resumeAt: (id, ms, play) => this.resumeAt(id, ms, play),
        emitState: (st) => this.emit("remote://state", st),
        emitPlayers: (p) => this.emit("remote://players", p),
        toast: (message) => this.emit("toast", { kind: "error", message }),
        settings: () => this.settings,
        later: (ms, fn) => this.later(ms, fn),
      },
      this.fx.tracks[7],
    );
    this.auth = this.opts.signedOut
      ? { state: "signedOut", needsServerPick: false }
      : this.opts.health === "signIn"
        ? { state: "signedOut", username: "listener", needsServerPick: false, reauthRequired: true }
        : { state: "signedIn", username: "listener", server: SERVER, needsServerPick: false };
    const startStatus = this.opts.health ?? (this.opts.timelines ? "starting" : "local");
    this.healthState = this.makeHealth(startStatus);

    // Something in the player bar from the start, paused (like a restored session).
    const firstAlbum = this.fx.albums[0];
    this.loadContext({ context: { kind: "album", id: firstAlbum.id } }, false);
    this.positionMs = 62_000;

    if (this.opts.timelines) this.runStartup();
  }

  // ---------------------------------------------------------------- infra

  private delay<T>(fn: () => T): Promise<T> {
    const ms = this.opts.latencyMs;
    return new Promise((resolve, reject) => {
      const run = () => {
        try {
          resolve(clone(fn()));
        } catch (e) {
          reject(e);
        }
      };
      if (ms <= 0) queueMicrotask(run);
      else this.later(ms * (0.6 + this.rng() * 0.8), run);
    });
  }

  private later(ms: number, fn: () => void) {
    const t = setTimeout(() => {
      this.timers.delete(t);
      fn();
    }, ms);
    this.timers.add(t);
  }

  emit<E extends EventName>(event: E, payload: EventMap[E]) {
    this.listeners.get(event)?.forEach((cb) => cb(clone(payload)));
  }

  listen<E extends EventName>(event: E, cb: (payload: EventMap[E]) => void): Promise<Unlisten> {
    let set = this.listeners.get(event);
    if (!set) this.listeners.set(event, (set = new Set()));
    const fn = cb as (p: unknown) => void;
    set.add(fn);
    return Promise.resolve(() => set!.delete(fn));
  }

  artUrl(path: string, w: number, h: number): string {
    if (this.opts.httpArt) return `http://art.localhost/${encodeURIComponent(path)}?w=${Math.round(w)}&h=${Math.round(h)}`;
    return mockArtUrl(path);
  }

  /** Stop all timers (tests). */
  dispose() {
    this.timers.forEach(clearTimeout);
    this.timers.clear();
    this.stopTick();
    this.remote.dispose();
    this.listeners.clear();
  }

  private notFound(what: string): never {
    throw { kind: "notFound", message: `${what} not found` };
  }

  // ---------------------------------------------------------------- startup timeline

  private runStartup() {
    this.later(700, () => this.setHealth(this.opts.health ?? "local"));
    const h = this.opts.health;
    if (h && h !== "local" && h !== "remote" && h !== "relay" && h !== "scanning") return; // nothing to sync from
    // incremental sync
    const total = 340;
    let done = 0;
    this.later(900, () => {
      this.sync = { ...this.sync, phase: "incremental", done: 0, total };
      this.emit("sync://progress", this.sync);
      const step = () => {
        done = Math.min(total, done + 34);
        this.sync = { ...this.sync, done };
        this.emit("sync://progress", this.sync);
        if (done < total) this.later(250, step);
        else
          this.later(200, () => {
            this.sync = { phase: "idle", done: 0, total: 0, lastSyncAt: Math.floor(Date.now() / 1000) };
            this.emit("sync://progress", this.sync);
            this.emit("library://changed", null);
          });
      };
      this.later(250, step);
    });
  }

  private makeHealth(demo: HealthDemo): Health {
    return demoHealth(demo, { name: SERVER.name, version: SERVER.version ?? "" });
  }

  /** Test/demo hook: change the app state (emits health://changed, and a
   *  "back" toast when leaving offline, like the core). */
  setHealth(demo: HealthDemo, patch: Partial<Health> = {}) {
    const wasOffline = this.healthState.banner?.kind === "offline" || this.healthState.banner?.kind === "noInternet";
    this.healthState = { ...this.makeHealth(demo), ...patch };
    this.emit("health://changed", this.healthState);
    const s = this.healthState.status;
    if (wasOffline && (s === "local" || s === "remote" || s === "relay")) this.emit("toast", { kind: "info", message: "Your server is back." });
  }

  /** Test/demo hook: Plex rejected the saved sign-in (expired or revoked). */
  expireSignIn() {
    this.auth = { state: "signedOut", username: "listener", needsServerPick: false, reauthRequired: true };
    this.emit("auth://changed", this.auth);
    this.setHealth("signIn");
  }

  /** Mock: the first two albums are on this computer (pre-cached). */
  cachedTracks() {
    return this.delay(() => {
      const albums = new Set(this.fx.albums.slice(0, 2).map((a) => a.id));
      return this.fx.tracks.filter((t) => albums.has(t.albumId)).map((t) => t.id);
    });
  }

  // ---------------------------------------------------------------- auth

  authStatus() {
    return this.delay(() => this.auth);
  }
  authStart(): Promise<PinInfo> {
    this.auth = { state: "pending", needsServerPick: false };
    this.emit("auth://changed", this.auth);
    if (this.opts.autoApproveMs != null) this.later(this.opts.autoApproveMs, () => this.approvePin());
    return this.delay(() => ({
      code: "K7QX",
      authUrl: "https://app.plex.tv/auth#?code=K7QX",
      expiresAt: Math.floor(Date.now() / 1000) + 600,
    }));
  }
  /** Test/demo hook: the user approved the PIN in their browser. */
  approvePin() {
    if (this.auth.state !== "pending") return;
    this.auth = { state: "signedIn", username: "listener", server: SERVER, needsServerPick: false };
    this.emit("auth://changed", this.auth);
  }
  authCancel() {
    this.auth = { state: "signedOut", needsServerPick: false };
    this.emit("auth://changed", this.auth);
    return this.delay(() => undefined);
  }
  authSignOut() {
    this.auth = { state: "signedOut", needsServerPick: false };
    this.emit("auth://changed", this.auth);
    return this.delay(() => undefined);
  }
  serversList() {
    return this.delay(() => [SERVER, { ...SERVER, machineId: "mock-machine-02", name: "Cabin NAS", owned: false, online: false, connection: null }]);
  }
  serverSelect(machineId: string) {
    return this.delay(() => {
      if (machineId !== SERVER.machineId) throw { kind: "invalid", message: "Server is offline" };
      this.auth = { ...this.auth, server: SERVER, needsServerPick: false };
      this.emit("auth://changed", this.auth);
    });
  }

  // ---------------------------------------------------------------- library

  syncStatus() {
    return this.delay(() => this.sync);
  }
  syncNow(full?: boolean) {
    return this.delay(() => {
      if (full && this.sync.phase === "idle") this.runSync(["artists", "albums", "tracks"]);
    });
  }

  /** Simulated sync: each phase counts up, then idle + library://changed. */
  private runSync(phases: SyncStatus["phase"][]) {
    const [phase, ...rest] = phases;
    if (!phase) {
      this.sync = { phase: "idle", done: 0, total: 0, lastSyncAt: Math.floor(Date.now() / 1000) };
      this.emit("sync://progress", this.sync);
      this.emit("library://changed", null);
      return;
    }
    const total = phase === "artists" ? this.fx.artists.length : phase === "albums" ? this.fx.albums.length : this.fx.tracks.length;
    const step = Math.max(1, Math.ceil(total / 6));
    let done = 0;
    const tick = () => {
      this.sync = { ...this.sync, phase, done, total };
      this.emit("sync://progress", this.sync);
      if (done >= total) this.later(200, () => this.runSync(rest));
      else {
        done = Math.min(total, done + step);
        this.later(250, tick);
      }
    };
    tick();
  }
  home() {
    return this.delay(() => new MockHome(this.fx, this.contextPlays).home());
  }
  homeShelf(shelf: HomeShelfId, limit?: number) {
    return this.delay(() => new MockHome(this.fx, this.contextPlays).shelf(shelf, limit));
  }
  /** Invented history: a few playlist and artist plays around the newest album play. */
  private seedContextPlays(): ContextPlay[] {
    const newest = Math.max(0, ...this.fx.albums.map((a) => a.lastViewedAt ?? 0));
    const pl = this.fx.playlists;
    const ar = this.fx.artists;
    const out: ContextPlay[] = [];
    if (pl[0]) out.push({ kind: "playlist", id: pl[0].id, at: newest + 3600 }, { kind: "playlist", id: pl[0].id, at: newest - 5 * 86400 });
    if (ar[3]) out.push({ kind: "artist", id: ar[3].id, at: newest - 1800 });
    if (pl[2]) out.push({ kind: "playlist", id: pl[2].id, at: newest - 2 * 86400 });
    if (pl[4]) out.push({ kind: "playlist", id: pl[4].id, at: newest - 9 * 86400 });
    return out;
  }
  artists(args: { sort: ArtistSort; offset: number; limit: number }) {
    return this.delay(() => {
      const list = [...this.fx.artists];
      if (args.sort === "name") list.sort((a, b) => a.sortTitle.localeCompare(b.sortTitle));
      if (args.sort === "plays") list.sort((a, b) => b.viewCount - a.viewCount);
      return { items: list.slice(args.offset, args.offset + args.limit), total: list.length };
    });
  }
  artistNames() {
    return this.delay(() => this.fx.artists.map((a) => ({ id: a.id, title: a.title })));
  }
  private allAlbums(): Album[] {
    const base = this.fx.albums;
    const n = this.opts.albumCount;
    if (!n || n <= base.length) return base;
    const out = [...base];
    for (let i = base.length; i < n; i++) {
      const src = base[i % base.length];
      out.push({
        ...src,
        title: `${src.title} (${Math.floor(i / base.length) + 1})`,
        sortTitle: `${src.sortTitle} ${i}`,
        thumb: `${src.thumb}/v${i}`,
        addedAt: src.addedAt - i * 3600,
      });
    }
    return out;
  }
  albums(args: { sort: AlbumSort; offset: number; limit: number; section?: ReleaseSection }) {
    return this.delay(() => {
      let list = this.allAlbums();
      if (args.section) list = list.filter((a) => a.section === args.section);
      list = [...list];
      const by: Record<AlbumSort, (a: Album, b: Album) => number> = {
        name: (a, b) => a.sortTitle.localeCompare(b.sortTitle),
        artist: (a, b) => a.artistTitle.localeCompare(b.artistTitle) || (a.year ?? 0) - (b.year ?? 0),
        year: (a, b) => (b.year ?? 0) - (a.year ?? 0),
        recent: (a, b) => b.addedAt - a.addedAt,
        plays: (a, b) => b.viewCount - a.viewCount,
      };
      list.sort(by[args.sort]);
      return { items: list.slice(args.offset, args.offset + args.limit), total: list.length };
    });
  }
  playlists() {
    return this.delay(() => this.fx.playlists);
  }
  libraryItems() {
    return this.delay(() => {
      const albums = this.allAlbums();
      const byArtist = new Map<string, { added: number | null; played: number | null }>();
      // The large-library demo repeats albums; keep their ids unique here.
      const items: LibraryItem[] = albums.map((a, i) => {
        const d = byArtist.get(a.artistId) ?? { added: null, played: null };
        d.added = d.added === null ? a.addedAt : Math.min(d.added, a.addedAt);
        d.played = Math.max(d.played ?? 0, a.lastViewedAt ?? 0) || null;
        byArtist.set(a.artistId, d);
        return {
          kind: "album",
          id: i < this.fx.albums.length ? a.id : `${a.id}-${i}`,
          title: a.title,
          sortTitle: a.sortTitle,
          creator: a.artistTitle,
          creatorId: a.artistId,
          thumb: a.thumb,
          smart: false,
          format: a.format,
          addedAt: a.addedAt,
          playedAt: a.lastViewedAt,
        };
      });
      for (const a of this.fx.artists) {
        const d = byArtist.get(a.id);
        items.push({ kind: "artist", id: a.id, title: a.title, sortTitle: a.sortTitle, thumb: a.thumb, smart: false, addedAt: d?.added ?? null, playedAt: d?.played ?? null });
      }
      for (const p of this.fx.playlists) {
        items.push({ kind: "playlist", id: p.id, title: p.title, sortTitle: p.title, thumb: p.thumb, smart: p.smart, addedAt: null, playedAt: null });
      }
      return items;
    });
  }
  artistPage(id: string) {
    return this.delay(() => {
      const artist = this.fx.artists.find((a) => a.id === id) ?? this.notFound("Artist");
      const albums = this.fx.albums.filter((a) => a.artistId === id);
      const popular = popularTracks(this.fx.tracks, artist, buildArtistLookup(this.fx.artists), this.settings.popularSource);
      const order: ReleaseSection[] = ["albums", "singlesEps", "live", "compilations", "soundtracks", "remixes", "demos", "mixtapes"];
      const sections = order
        .map((section) => ({
          section,
          albums: albums.filter((a) => a.section === section).sort((a, b) => (b.year ?? 9999) - (a.year ?? 9999)),
        }))
        .filter((s) => s.albums.length > 0);
      const plays = this.fx.tracks.filter((t) => t.artistId === id).reduce((n, t) => n + t.viewCount, 0);
      const idx = this.fx.artists.indexOf(artist);
      // Most artists have a biography; every third has none. Similar artists
      // are the next two in the library; the last few have none.
      const about = this.plexBio(idx, artist.title);
      const similar = idx >= this.fx.artists.length - 3 ? [] : [1, 2].map((d) => this.fx.artists[(idx + d) % this.fx.artists.length]);
      return { artist, popular, sections, plays, about, similar };
    });
  }
  albumPage(id: string) {
    return this.delay(() => {
      const album = this.fx.albums.find((a) => a.id === id) ?? this.notFound("Album");
      const tracks = this.fx.tracksByAlbum.get(id) ?? [];
      return {
        album,
        tracks,
        totalDurationMs: tracks.reduce((s, t) => s + t.durationMs, 0),
        moreByArtist: this.fx.albums.filter((a) => a.artistId === album.artistId && a.id !== id),
      };
    });
  }
  playlistPage(id: string) {
    return this.delay(() => {
      const playlist = this.fx.playlists.find((p) => p.id === id) ?? this.notFound("Playlist");
      const ids = this.fx.playlistTracks.get(id) ?? [];
      const itemIds = this.pl.itemIds(id);
      const rows = ids.map((tid, i) => ({ track: this.trackById(tid), itemId: itemIds[i] ?? `it-${id}-${i}` })).filter((r) => r.track);
      return { playlist, tracks: rows.map((r) => r.track!), itemIds: rows.map((r) => r.itemId) };
    });
  }

  // ---------------------------------------------------------------- playlist editing

  playlistCreate(title?: string, trackIds: string[] = []) {
    return this.delay(() => this.pl.create(this.settings.playlistNewName, title, trackIds));
  }
  playlistAdd(id: string, trackIds: string[], place?: AddPlace) {
    return this.delay(() => this.pl.add(id, trackIds, place));
  }
  playlistDuplicates(id: string, trackIds: string[]) {
    return this.delay(() => this.pl.duplicates(id, trackIds));
  }
  playlistRemove(id: string, itemIds: string[]) {
    return this.delay(() => this.pl.remove(id, itemIds));
  }
  playlistMove(id: string, itemIds: string[], after?: string | null) {
    return this.delay(() => this.pl.move(id, itemIds, after));
  }
  playlistUpdate(id: string, details: { title?: string; summary?: string }) {
    return this.delay(() => this.pl.update(id, details));
  }
  playlistSetCover(id: string, bytes: Uint8Array, contentType: string) {
    return this.delay(() => this.pl.setCover(id, bytes, contentType));
  }
  playlistDelete(id: string) {
    return this.delay(() => this.pl.delete(id));
  }

  // ---------------------------------------------------------------- likes

  likesSet(trackIds: string[], liked: boolean) {
    return this.delay(() => this.likes.set(trackIds, liked));
  }
  likedIds() {
    return this.delay(() => this.likes.ids());
  }
  likedPage() {
    return this.delay(() => this.likes.page());
  }
  playlistsWithTrack(trackId: string) {
    return this.delay(() => this.likes.playlistsWith(trackId));
  }

  nowPlayingDetails(trackId: string) {
    return this.delay(() => {
      const track = this.trackById(trackId) ?? this.notFound("Track");
      return mockNowPlayingDetails(track, this.fx.artists.find((a) => a.id === track.artistId));
    });
  }

  private trackById(id: string): Track | undefined {
    return this.fx.tracks.find((t) => t.id === id);
  }

  // ---------------------------------------------------------------- search

  search(args: SearchArgs) {
    const started = performance.now();
    return this.delay((): SearchResults => {
      const q = args.query.trim().toLowerCase();
      const limit = args.limit ?? 20;
      const empty: SearchResults = { top: null, songs: [], artists: [], albums: [], playlists: [], tookMs: 0, source: "local" };
      if (!q) return empty;
      const score = (s: string) => {
        const l = s.toLowerCase();
        if (l === q) return 3;
        if (l.startsWith(q)) return 2;
        if (l.split(/\s+/).some((w) => w.startsWith(q))) return 1.5;
        return l.includes(q) ? 1 : 0;
      };
      const rank = <T,>(items: T[], key: (t: T) => string, pop: (t: T) => number) =>
        items
          .map((t) => ({ t, s: score(key(t)) }))
          .filter((x) => x.s > 0)
          .sort((a, b) => b.s - a.s || pop(b.t) - pop(a.t))
          .map((x) => x.t);
      const f = args.filter;
      const artists = f === "all" || f === "artists" ? rank(this.fx.artists, (a) => a.title, (a) => a.viewCount) : [];
      const albums = f === "all" || f === "albums" ? rank(this.fx.albums, (a) => a.title, (a) => a.viewCount) : [];
      const songs =
        f === "all" || f === "songs"
          ? rank(this.fx.tracks, (t) => `${t.title}`, (t) => t.viewCount).concat(
              rank(this.fx.tracks, (t) => t.artistTitle, (t) => t.viewCount).filter((t) => score(t.title) === 0),
            )
          : [];
      const playlists = f === "all" || f === "playlists" ? rank(this.fx.playlists, (p) => p.title, () => 0) : [];
      let top: SearchResults["top"] = null;
      const cands: [number, NonNullable<SearchResults["top"]>][] = [];
      if (artists[0]) cands.push([score(artists[0].title) + 0.3, { kind: "artist", artist: artists[0] }]);
      if (albums[0]) cands.push([score(albums[0].title) + 0.2, { kind: "album", album: albums[0] }]);
      if (songs[0] && score(songs[0].title) > 0) cands.push([score(songs[0].title), { kind: "track", track: songs[0] }]);
      if (playlists[0]) cands.push([score(playlists[0].title) + 0.1, { kind: "playlist", playlist: playlists[0] }]);
      if (cands.length) top = cands.sort((a, b) => b[0] - a[0])[0][1];
      return {
        top,
        songs: songs.slice(0, limit),
        artists: artists.slice(0, limit),
        albums: albums.slice(0, limit),
        playlists: playlists.slice(0, limit),
        tookMs: Math.round((performance.now() - started) * 10) / 10,
        source: "local",
      };
    });
  }
  searchCompletions(query: string, limit = 4) {
    return this.delay(() => {
      const fold = (s: string) =>
        s
          .normalize("NFD")
          .replace(/\p{M}/gu, "")
          .toLowerCase()
          .split(/[^\p{L}\p{N}]+/u)
          .filter(Boolean)
          .join(" ");
      const q = fold(query.slice(0, 200));
      if (!q) return [] as string[];
      const cands: [string, number][] = [
        ...this.fx.artists.map((a): [string, number] => [a.title, a.viewCount]),
        ...this.fx.albums.map((a): [string, number] => [a.title, a.viewCount]),
        ...this.fx.tracks.map((t): [string, number] => [t.title, t.viewCount]),
      ];
      cands.sort((a, b) => b[1] - a[1] || [...a[0]].length - [...b[0]].length);
      const seen = new Set<string>();
      const out: string[] = [];
      for (const [title] of cands) {
        const n = fold(title);
        if (!n.startsWith(q) || seen.has(n)) continue;
        seen.add(n);
        out.push(title);
        if (out.length >= Math.min(limit, 10)) break;
      }
      return out;
    });
  }
  /** A few remembered items so the dropdown is demoable (not in tests). */
  private seedRecents() {
    if (this.recentSeeded) return;
    this.recentSeeded = true;
    if (isTestEnv) return;
    const fx = this.fx;
    this.recent = [
      fx.albums[0] && { kind: "album" as const, id: fx.albums[0].id },
      fx.artists[1] && { kind: "artist" as const, id: fx.artists[1].id },
      fx.tracks[3] && { kind: "track" as const, id: fx.tracks[3].id },
      fx.playlists[0] && { kind: "playlist" as const, id: fx.playlists[0].id },
    ].filter((x): x is { kind: SearchKind; id: string } => !!x);
  }
  searchRecents() {
    return this.delay(() => {
      this.seedRecents();
      const out: SearchHit[] = [];
      for (const r of this.recent) {
        if (r.kind === "artist") {
          const artist = this.fx.artists.find((a) => a.id === r.id);
          if (artist) out.push({ kind: "artist", artist });
        } else if (r.kind === "album") {
          const album = this.fx.albums.find((a) => a.id === r.id);
          if (album) out.push({ kind: "album", album });
        } else if (r.kind === "track") {
          const track = this.trackById(r.id);
          if (track) out.push({ kind: "track", track });
        } else {
          const playlist = this.fx.playlists.find((p) => p.id === r.id);
          if (playlist) out.push({ kind: "playlist", playlist });
        }
      }
      return clone(out);
    });
  }
  searchRecentAdd(kind: SearchKind, id: string) {
    return this.delay(() => {
      this.seedRecents();
      if (!id) return;
      this.recent = [{ kind, id }, ...this.recent.filter((r) => !(r.kind === kind && r.id === id))].slice(0, 20);
    });
  }
  searchRecentRemove(kind: SearchKind, id: string) {
    return this.delay(() => {
      this.seedRecents();
      this.recent = this.recent.filter((r) => !(r.kind === kind && r.id === id));
    });
  }
  searchRecentsClear() {
    return this.delay(() => {
      this.recentSeeded = true;
      this.recent = [];
    });
  }

  // ---------------------------------------------------------------- playback

  private item(track: Track, source: QueueItem["source"]): QueueItem {
    return { itemId: `q${this.itemSeq++}`, track, source };
  }

  /** Plain random order (Fisher-Yates). */
  private shuffled<T>(arr: T[]): T[] {
    const a = [...arr];
    for (let i = a.length - 1; i > 0; i--) {
      const j = Math.floor(this.rng() * (i + 1));
      [a[i], a[j]] = [a[j], a[i]];
    }
    return a;
  }

  /** Spread: each artist's tracks evenly spaced, random offset and jitter
   *  (the core's Spread style, minus its recent-play weighting). */
  private spread(items: QueueItem[]): QueueItem[] {
    const groups = new Map<string, QueueItem[]>();
    for (const it of items) {
      const k = (it.track.trackArtist ?? it.track.artistTitle).toLowerCase();
      let g = groups.get(k);
      if (!g) groups.set(k, (g = []));
      g.push(it);
    }
    const placed: [number, QueueItem][] = [];
    for (const g of groups.values()) {
      const n = g.length;
      const offset = this.rng() / n;
      this.shuffled(g).forEach((it, j) => placed.push([offset + j / n + ((this.rng() - 0.5) * 0.2) / n, it]));
    }
    return placed.sort((a, b) => a[0] - b[0]).map((p) => p[1]);
  }

  private shuffleOrder(items: QueueItem[]): QueueItem[] {
    return this.settings.shuffleStyle === "standard" ? this.shuffled(items) : this.spread(items);
  }

  private shufflePref(id: string): boolean {
    return this.shuffledIds.includes(id);
  }

  private storeShufflePref(id: string, on: boolean) {
    this.shuffledIds = this.shuffledIds.filter((s) => s !== id);
    if (on) this.shuffledIds = [...this.shuffledIds, id].slice(-CONTEXT_MEMORY_CAP);
  }

  /** Remember where the current context is (track change, pause, ticks). */
  private notePosition() {
    const id = this.ctxInfo?.id;
    const cur = this.current;
    if (!id || !cur) return;
    let rp: ResumePoint | null = null;
    if (cur.source === "context") {
      const at = this.ctxOrder[this.ctxPos];
      if (at?.itemId === cur.itemId) rp = { index: this.ctxOriginal.indexOf(at), trackId: at.track.id, positionMs: this.positionMs };
    } else {
      // A queued song is playing: the context picks up at its next track.
      const nxt = this.ctxOrder[this.ctxPos + 1];
      if (nxt) rp = { index: this.ctxOriginal.indexOf(nxt), trackId: nxt.track.id, positionMs: 0 };
    }
    if (!rp) return;
    this.resumePoints.delete(id);
    this.resumePoints.set(id, rp);
    while (this.resumePoints.size > CONTEXT_MEMORY_CAP) this.resumePoints.delete(this.resumePoints.keys().next().value!);
  }

  private resumeStart(id: string): { index: number; positionMs: number } | null {
    const rp = this.resumePoints.get(id);
    if (!rp) return null;
    if (this.ctxOriginal[rp.index]?.track.id === rp.trackId) return { index: rp.index, positionMs: rp.positionMs };
    const i = this.ctxOriginal.findIndex((it) => it.track.id === rp.trackId);
    return i >= 0 ? { index: i, positionMs: rp.positionMs } : null;
  }

  private contextTracks(args: PlayContextArgs): { tracks: Track[]; info: QueueContextInfo } {
    const c = args.context;
    switch (c.kind) {
      case "album": {
        const album = this.fx.albums.find((a) => a.id === c.id) ?? this.notFound("Album");
        return { tracks: this.fx.tracksByAlbum.get(album.id) ?? [], info: { kind: "album", id: album.id, title: album.title } };
      }
      case "playlist": {
        const pl = this.fx.playlists.find((p) => p.id === c.id) ?? this.notFound("Playlist");
        const ids = this.fx.playlistTracks.get(pl.id) ?? [];
        return { tracks: ids.map((i) => this.trackById(i)!).filter(Boolean), info: { kind: "playlist", id: pl.id, title: pl.title } };
      }
      case "liked":
        return { tracks: c.trackIds ? c.trackIds.map((i) => this.trackById(i)!).filter(Boolean) : this.likes.likedTracks(), info: { kind: "liked", id: "liked", title: "Liked Songs" } };
      case "artist": {
        const ar = this.fx.artists.find((a) => a.id === c.id) ?? this.notFound("Artist");
        const tracks = this.fx.tracks.filter((t) => t.artistId === ar.id).sort((a, b) => b.viewCount - a.viewCount);
        return { tracks, info: { kind: "artist", id: ar.id, title: ar.title } };
      }
      default: {
        const tracks = (c.trackIds ?? []).map((i) => this.trackById(i)!).filter(Boolean);
        return { tracks, info: { kind: c.kind, id: c.id ?? null, title: c.kind === "search" ? "Search results" : "Tracks" } };
      }
    }
  }

  private loadContext(args: PlayContextArgs, play: boolean) {
    const { tracks, info } = this.contextTracks(args);
    if (!tracks.length) return;
    this.notePosition(); // where the outgoing context was left
    const id = info.id ?? null;
    const kind = args.context.kind;
    if (play && id && (kind === "album" || kind === "artist" || kind === "playlist"))
      this.contextPlays.push({ kind, id, at: Math.floor(Date.now() / 1000) });
    if (args.shuffle !== undefined) {
      this.shuffle = args.shuffle;
      if (id) this.storeShufflePref(id, args.shuffle);
    } else {
      this.shuffle = id ? this.shufflePref(id) : false;
    }
    if (this.current) this.pushHistory(this.current);
    this.ctxOriginal = tracks.map((t) => this.item(t, "context"));
    this.ctxInfo = info;
    let first = args.startTrackId ? this.ctxOriginal.find((i) => i.track.id === args.startTrackId) : undefined;
    let offset = 0;
    if (!first && this.shuffle) first = this.ctxOriginal[Math.floor(this.rng() * this.ctxOriginal.length)];
    if (!first) {
      const r = id && this.settings.resumeContexts ? this.resumeStart(id) : null;
      first = this.ctxOriginal[r?.index ?? 0];
      offset = r?.positionMs ?? 0;
    }
    if (this.shuffle) {
      this.ctxOrder = [first, ...this.shuffleOrder(this.ctxOriginal.filter((i) => i !== first))];
      this.ctxPos = 0;
    } else {
      this.ctxOrder = [...this.ctxOriginal];
      this.ctxPos = this.ctxOrder.indexOf(first);
    }
    this.setCurrent(this.ctxOrder[this.ctxPos], play, offset);
  }

  private pushHistory(item: QueueItem) {
    this.history.push(item);
    if (this.history.length > 50) this.history.shift();
  }

  private setCurrent(item: QueueItem | null, play: boolean, startMs = 0) {
    // Like the core: a loop belongs to one track.
    if (item !== this.current) this.abLoop = null;
    this.current = item;
    this.positionMs = startMs;
    if (item && item.track.title.endsWith(BROKEN_TRACK_TITLE_SUFFIX) && play) {
      // Spec 07: one file missing/corrupt → toast + skip.
      // The UI turns player://error into the "Skipped" toast.
      this.emit("player://error", { trackId: item.track.id, kind: "io", message: "Skipped: file unavailable" });
      this.advance(false);
      return;
    }
    this.status = item ? (play ? "playing" : "paused") : "idle";
    this.notePosition();
    if (this.status === "playing") this.startTick();
    else this.stopTick();
    this.emitState();
  }

  private advance(natural: boolean, manual = false) {
    if (natural && this.repeat === "one" && this.current) {
      this.positionMs = 0;
      this.emitState();
      return;
    }
    // Next in repeat-one skips on and drops to repeat-all.
    if (manual && this.repeat === "one") this.repeat = "all";
    if (this.userQueue.length) {
      if (this.current) this.pushHistory(this.current);
      this.setCurrent(this.userQueue.shift()!, true);
    } else if (this.ctxPos + 1 < this.ctxOrder.length) {
      if (this.current) this.pushHistory(this.current);
      this.ctxPos++;
      this.setCurrent(this.ctxOrder[this.ctxPos], true);
    } else if (this.repeat === "all" && this.ctxOrder.length) {
      // Repeat-all replays the same (shuffled) order.
      if (this.current) this.pushHistory(this.current);
      this.ctxPos = 0;
      this.setCurrent(this.ctxOrder[0], true);
    } else {
      // The context ended (repeat off): back to its first track, paused at 0.
      if (this.ctxOrder.length) {
        if (this.current) this.pushHistory(this.current);
        this.ctxPos = 0;
        this.current = this.ctxOrder[0];
      }
      this.status = "paused";
      this.positionMs = 0;
      this.stopTick();
      this.notePosition();
      this.emitState();
    }
  }

  private startTick() {
    if (this.tick) return;
    this.tick = setInterval(() => {
      if (this.status !== "playing" || !this.current) return;
      this.positionMs += 250;
      if (this.positionMs % 5000 === 0) this.notePosition();
      const d = this.current.track.durationMs;
      // A-B loop: at B (or the end), back to A.
      if (this.abLoop && this.positionMs >= Math.min(this.abLoop.bMs, d)) this.positionMs = this.abLoop.aMs;
      if (this.positionMs >= d) {
        this.advance(true);
        return;
      }
      this.emit("player://position", { positionMs: this.positionMs, durationMs: d, bufferedMs: Math.min(d, this.positionMs + 30_000) });
    }, 250);
  }

  private stopTick() {
    if (this.tick) clearInterval(this.tick);
    this.tick = null;
  }

  private snapshot(): PlayerState {
    return {
      status: this.status,
      track: this.current?.track ?? null,
      positionMs: this.positionMs,
      durationMs: this.current?.track.durationMs ?? 0,
      volume: this.volume,
      muted: this.muted,
      shuffle: this.shuffle,
      shuffledContexts: this.shuffledIds,
      repeat: this.repeat,
      abLoop: this.abLoop,
      queue: {
        current: this.current,
        userQueue: this.userQueue,
        nextUp: this.ctxOrder.slice(this.ctxPos + 1),
        history: this.history,
        context: this.ctxInfo,
      },
    };
  }

  private emitState() {
    this.emit("player://state", this.snapshot());
  }

  private act(fn: () => void) {
    return this.delay(() => {
      fn();
    });
  }

  playContext(args: PlayContextArgs) {
    return this.act(() => this.loadContext(args, true));
  }
  playToggle() {
    return this.act(() => (this.status === "playing" ? this.pauseNow() : this.resumeNow()));
  }
  playPause() {
    return this.act(() => this.pauseNow());
  }
  /** Test/demo hook: headphones unplugged / Bluetooth dropped. The core
   *  pauses; Play resumes on whatever output is there now. */
  loseDevice() {
    if (this.status === "buffering") this.status = "playing";
    this.pauseNow();
  }
  playResume() {
    return this.act(() => this.resumeNow());
  }

  // ---------------------------------------------------------------- output devices

  private outputState(): OutputState {
    const sel = this.outputs.find((d) => d.id === this.outputSelected);
    return {
      devices: this.outputs,
      selected: this.outputSelected,
      current: sel ?? this.outputs.find((d) => d.isDefault) ?? null,
    };
  }
  outputDevices() {
    return this.delay(() => this.outputState());
  }
  outputSelect(deviceId: string | null) {
    return this.delay(() => {
      if (deviceId !== null && !this.outputs.some((d) => d.id === deviceId)) this.notFound("Output device");
      // Same as the core: position and play state carry over.
      this.outputSelected = deviceId;
      this.settings.outputDevice = deviceId;
      const s = this.outputState();
      this.emitOutput();
      return s;
    });
  }
  // ---------------------------------------------------------------- listen on

  remotePlayers() {
    return this.delay(() => this.remote.players());
  }
  remoteState() {
    return this.delay(() => this.remote.state());
  }
  remoteMove(playerId: string) {
    return this.delay(() => this.remote.move(playerId));
  }
  remoteReturn() {
    return this.delay(() => this.remote.returnHere());
  }
  remotePlayContext(args: PlayContextArgs) {
    return this.delay(() => {
      this.loadContext(args, false);
      return this.remote.resend();
    });
  }
  remoteQueueJump(itemId: string) {
    return this.delay(() => {
      const it = [...this.userQueue, ...this.ctxOrder].find((i) => i.itemId === itemId) ?? this.notFound("Queue item");
      this.resumeAt(it.track.id, 0, false);
      return this.remote.resend();
    });
  }
  remoteMoveWithPassword(playerId: string, password: string) {
    return this.delay(() => this.remote.moveWithPassword(playerId, password));
  }
  remoteCommand(command: RemoteCommand) {
    return this.delay(() => this.remote.command(command));
  }
  /** Local playback onto this song at this spot (back from another device). */
  private resumeAt(trackId: string, ms: number, play: boolean) {
    if (this.current?.track.id === trackId) {
      this.positionMs = ms;
      if (play) this.resumeNow();
      else this.pauseNow();
      this.emitState();
      return;
    }
    const u = this.userQueue.findIndex((i) => i.track.id === trackId);
    if (u >= 0) {
      if (this.current) this.pushHistory(this.current);
      const it = this.userQueue[u];
      this.userQueue = this.userQueue.slice(u + 1);
      this.setCurrent(it, play, ms);
      return;
    }
    const c = this.ctxOrder.findIndex((i, k) => k > this.ctxPos && i.track.id === trackId);
    if (c >= 0) {
      if (this.current) this.pushHistory(this.current);
      this.ctxPos = c;
      this.setCurrent(this.ctxOrder[c], play, ms);
      return;
    }
    this.loadContext({ context: { kind: "tracks", trackIds: [trackId] }, startTrackId: trackId }, play);
    this.positionMs = ms;
    this.emitState();
  }

  /** `output://changed`, and the EQ follows the device like in the core. */
  private emitOutput() {
    const s = this.outputState();
    this.emit("output://changed", s);
    if (eq.outputChanged(this.settings, s.current)) this.emit("settings://changed", { ...this.settings, volume: this.volume });
  }
  /** Test/demo hook: an output device is unplugged. If it was the chosen
   *  one, the core pauses, falls back to the system default and says so. */
  unplugOutput(id: string) {
    const gone = this.outputs.find((d) => d.id === id);
    if (!gone) return;
    this.outputs = this.outputs.filter((d) => d.id !== id);
    if (gone.isDefault && this.outputs[0]) this.outputs = this.outputs.map((d, i) => ({ ...d, isDefault: i === 0 }));
    const chosen = this.outputSelected === id;
    const followedDefault = this.outputSelected === null && gone.isDefault;
    if (chosen) this.outputSelected = null;
    if (chosen || followedDefault) this.loseDevice();
    this.emitOutput();
    if (chosen) this.emit("output://lost", { device: gone.name });
  }
  /** Test/demo hook: a device is plugged in. */
  plugOutput(device: OutputDevice) {
    this.outputs = [...this.outputs.filter((d) => d.id !== device.id), device];
    this.emitOutput();
  }
  /** Test hook: audio settings back to their defaults (profiles and bindings too). */
  resetAudio() {
    const d = DEFAULT_SETTINGS;
    Object.assign(this.settings, {
      crossfade: d.crossfade,
      crossfadeSeconds: d.crossfadeSeconds,
      crossfadeSkipAlbums: d.crossfadeSkipAlbums,
      leveling: d.leveling,
      preampDb: d.preampDb,
      limiter: d.limiter,
      eqEnabled: d.eqEnabled,
      eqBands: [...d.eqBands],
      eqPreset: d.eqPreset,
      eqProfiles: [],
      eqOutputs: [],
      eqFallback: null,
      eqLastOutput: null,
    });
  }
  /** Test hook: the starting devices, following the system default. */
  resetOutputs() {
    this.outputs = MOCK_OUTPUTS.map((d) => ({ ...d }));
    this.outputSelected = null;
    this.settings.outputDevice = null;
  }
  private pauseNow() {
    if (this.status !== "playing") return;
    this.status = "paused";
    this.stopTick();
    this.notePosition();
    this.emitState();
  }
  private resumeNow() {
    if (!this.current) return;
    this.status = "playing";
    this.startTick();
    this.emitState();
  }
  playNext() {
    return this.act(() => this.advance(false, true));
  }
  playPrevious() {
    return this.act(() => {
      const restart = () => {
        this.positionMs = 0;
        this.emitState();
      };
      // At or past 3 s: restart, and repeat-one is kept.
      if (this.positionMs >= PREVIOUS_RESTART_MS || !this.current) return restart();
      // Earlier: repeat-one drops to repeat-all, and Previous goes to the
      // previous context position (never a queued song). On a playing
      // queued song that is the context track before it; the song is dropped.
      if (this.repeat === "one") this.repeat = "all";
      const target = this.current.source === "user" ? this.ctxPos : this.ctxPos - 1;
      if (target < 0 || target >= this.ctxOrder.length) return restart();
      this.pushHistory(this.current);
      this.ctxPos = target;
      this.setCurrent(this.ctxOrder[target], true);
    });
  }
  playSeek(positionMs: number) {
    return this.act(() => {
      this.positionMs = Math.max(0, Math.min(positionMs, this.current?.track.durationMs ?? 0));
      // Seeking outside the loop's span clears it.
      if (this.abLoop && (this.positionMs < this.abLoop.aMs || this.positionMs > this.abLoop.bMs)) this.abLoop = null;
      this.emitState();
    });
  }
  setVolume(volume: number) {
    return this.act(() => {
      // Like the core: the level and mute are separate; setting one leaves the other.
      this.volume = Math.max(0, Math.min(1, volume));
      this.emitState();
    });
  }
  setMuted(muted: boolean) {
    return this.act(() => {
      this.muted = muted;
      this.emitState();
    });
  }
  setShuffle(shuffle: boolean) {
    return this.act(() => this.applyShuffle(shuffle));
  }
  /** Shuffle for the playing context; remembered for its id. */
  private applyShuffle(shuffle: boolean) {
    if (this.ctxInfo?.id) this.storeShufflePref(this.ctxInfo.id, shuffle);
    if (shuffle !== this.shuffle) {
      this.shuffle = shuffle;
      const at = this.ctxPos >= 0 ? this.ctxOrder[this.ctxPos] : undefined;
      if (shuffle) {
        // Every other track (earlier ones too) dealt into a new order.
        const live = new Set(this.ctxOrder);
        const others = this.ctxOriginal.filter((i) => i !== at && live.has(i));
        this.ctxOrder = at ? [at, ...this.shuffleOrder(others)] : this.shuffleOrder(others);
        this.ctxPos = at ? 0 : -1;
      } else {
        // Back to original order, continuing after the current position.
        const live = new Set(this.ctxOrder);
        this.ctxOrder = this.ctxOriginal.filter((i) => live.has(i));
        this.ctxPos = at ? this.ctxOrder.indexOf(at) : -1;
      }
    }
    this.emitState();
  }
  /** Shuffle preference for a context: applies now if it's playing,
   *  otherwise the next time it plays. */
  setContextShuffle(contextId: string, shuffle: boolean) {
    return this.act(() => {
      if (this.ctxInfo?.id === contextId) return this.applyShuffle(shuffle);
      this.storeShufflePref(contextId, shuffle);
      this.emitState();
    });
  }
  setRepeat(repeat: RepeatMode) {
    return this.act(() => {
      this.repeat = repeat;
      this.emitState();
    });
  }
  setAbLoop(abLoop: AbLoop | null) {
    return this.act(() => {
      if (abLoop) {
        if (!this.current) throw { kind: "invalid", message: "nothing is playing" };
        const d = this.current.track.durationMs;
        const a = Math.min(abLoop.aMs, abLoop.bMs, d);
        const b = Math.min(Math.max(abLoop.aMs, abLoop.bMs), d);
        if (b - a < 1000) throw { kind: "invalid", message: "B must be at least 1 s after A" };
        this.abLoop = { aMs: Math.round(a), bMs: Math.round(b) };
      } else {
        this.abLoop = null;
      }
      this.emitState();
    });
  }
  queueAdd(trackIds: string[], at: "next" | "end") {
    return this.act(() => {
      const items = trackIds.map((id) => this.trackById(id)).filter((t): t is Track => !!t).map((t) => this.item(t, "user"));
      this.userQueue = at === "next" ? [...items, ...this.userQueue] : [...this.userQueue, ...items];
      if (!this.current && this.userQueue.length) this.setCurrent(this.userQueue.shift()!, false);
      this.emitState();
      this.emit("toast", { kind: "info", message: items.length === 1 ? "Added to queue" : `Added ${items.length} songs to queue` });
    });
  }
  queueRemove(itemId: string) {
    return this.act(() => {
      this.userQueue = this.userQueue.filter((i) => i.itemId !== itemId);
      const idx = this.ctxOrder.findIndex((i) => i.itemId === itemId);
      if (idx > this.ctxPos) this.ctxOrder.splice(idx, 1);
      this.emitState();
    });
  }
  queueMove(itemId: string, toIndex: number) {
    return this.act(() => {
      const from = this.userQueue.findIndex((i) => i.itemId === itemId);
      if (from < 0) return;
      const [it] = this.userQueue.splice(from, 1);
      this.userQueue.splice(Math.max(0, Math.min(toIndex, this.userQueue.length)), 0, it);
      this.emitState();
    });
  }
  queueClear() {
    return this.act(() => {
      this.userQueue = [];
      this.emitState();
    });
  }
  queueJump(itemId: string) {
    return this.act(() => {
      const u = this.userQueue.findIndex((i) => i.itemId === itemId);
      if (u >= 0) {
        if (this.current) this.pushHistory(this.current);
        const it = this.userQueue[u];
        this.userQueue = this.userQueue.slice(u + 1);
        this.current = null;
        this.setCurrent(it, true);
        return;
      }
      const c = this.ctxOrder.findIndex((i) => i.itemId === itemId);
      if (c > this.ctxPos) {
        if (this.current) this.pushHistory(this.current);
        this.ctxPos = c;
        this.current = null;
        this.setCurrent(this.ctxOrder[c], true);
        return;
      }
      const h = this.history.findIndex((i) => i.itemId === itemId);
      if (h >= 0) {
        const it = this.history[h];
        if (this.current) this.pushHistory(this.current);
        this.current = null;
        this.setCurrent({ ...it, itemId: `q${this.itemSeq++}` }, true);
      }
    });
  }
  playerState() {
    return this.delay(() => this.snapshot());
  }

  // ---------------------------------------------------------------- health, settings, updates

  health() {
    return this.delay(() => this.healthState);
  }
  settingsGet() {
    return this.delay(() => ({ ...this.settings, volume: this.volume }));
  }
  settingsSet(patch: Partial<Settings>) {
    return this.delay(() => {
      const next = { ...this.settings, ...patch };
      // Profiles and bindings change only through their own calls.
      next.eqProfiles = this.settings.eqProfiles;
      next.eqOutputs = this.settings.eqOutputs;
      next.eqFallback = this.settings.eqFallback;
      next.eqLastOutput = this.settings.eqLastOutput;
      next.eqPreset = this.settings.eqPreset;
      next.eqBands = this.settings.eqBands;
      eq.applyAudioPatch(next, patch);
      // Same ranges as the core.
      next.precacheCount = Math.min(50, Math.max(1, Math.round(next.precacheCount)));
      next.artCacheMb = Math.min(51_200, Math.max(128, Math.round(next.artCacheMb)));
      // Only outputSelect changes the output device.
      next.outputDevice = this.settings.outputDevice;
      this.settings = next;
      this.artUsed = Math.min(this.artUsed, next.artCacheMb * 1024 * 1024);
      if (typeof patch.volume === "number") this.volume = Math.min(1, Math.max(0, patch.volume));
      if (patch.abLoopHandles === false && this.abLoop) {
        this.abLoop = null;
        this.emitState();
      }
    });
  }
  private changeEq(f: (s: Settings) => void) {
    return this.delay(() => {
      const next: Settings = { ...this.settings, eqProfiles: this.settings.eqProfiles.map((p) => ({ ...p })) };
      f(next);
      this.settings = next;
      return { ...next, volume: this.volume };
    });
  }
  eqProfileSave(name: string) {
    return this.changeEq((s) => eq.saveAs(s, name));
  }
  eqProfileRename(from: string, to: string) {
    return this.changeEq((s) => eq.rename(s, from, to));
  }
  eqProfileDelete(name: string) {
    return this.changeEq((s) => eq.remove(s, name));
  }
  eqUseForOutput(on: boolean) {
    return this.changeEq((s) => {
      const d = this.outputState().current;
      if (!d) throw { kind: "notFound", message: "No output device is in use." };
      eq.useForOutput(s, d, on);
    });
  }
  artCacheInfo() {
    return this.delay(() => ({ usedBytes: this.artUsed, capBytes: this.settings.artCacheMb * 1024 * 1024 }));
  }
  artCacheClear() {
    return this.delay(() => {
      const freed = this.artUsed;
      this.artUsed = 0;
      return freed;
    });
  }
  appInfo() {
    return this.delay(() => ({ version: "0.1.0", licence: "GPL-3.0-or-later", repoUrl: "https://github.com/Silvercyclops999/platter" }));
  }
  openProjectPage() {
    return this.delay(() => undefined);
  }
  updateCheck() {
    return this.delay(() =>
      this.opts.updateAvailable
        ? { available: true, version: "0.2.0", notes: null }
        : { available: false, version: null, notes: null },
    );
  }
  updateInstallOnRestart() {
    return this.delay(() => undefined);
  }

  // ---------------------------------------------------------------- artist info

  /** Plex bio as artistPage gives it (every third artist has none). */
  private plexBio(idx: number, title: string): string | null {
    return idx % 3 === 2 ? null : mockBio(title, idx);
  }

  artistInfo(id: string) {
    return this.delay(() => {
      const artist = this.fx.artists.find((a) => a.id === id) ?? this.notFound("Artist");
      const idx = this.fx.artists.indexOf(artist);
      // In the browser the first open answers with Plex only, then the
      // "background fetch" lands, like the core.
      const fetched = !this.opts.timelines || this.infoFetched.has(id);
      if (!fetched) {
        this.infoFetched.add(id);
        this.later(600, () => this.emit("artistinfo://changed", { id }));
      }
      return mockArtistInfo(artist, idx, this.plexBio(idx, artist.title), this.fx.artists, this.settings, this.infoKeys, fetched);
    });
  }
  artistInfoKeys() {
    return this.delay(() => mockKeysStatus(this.infoKeys));
  }
  artistInfoKeySet(source: KeyedInfoSource, key: string | null) {
    return this.delay(() => mockSetKey(this.infoKeys, source, key));
  }
  artistInfoCacheClear() {
    return this.delay(() => {
      const n = this.infoFetched.size;
      this.infoFetched.clear();
      return n;
    });
  }
  openInfoLink(url: string) {
    return this.delay(() => {
      if (typeof window !== "undefined" && !isTestEnv) window.open(url, "_blank", "noopener");
    });
  }

  // ---------------------------------------------------------------- sharing

  shareInfo(kind: ShareKind, id: string, atSeconds?: number) {
    return this.delay(() => this.share.info(kind, id, atSeconds));
  }
  sharePublicLink(kind: ShareKind, id: string) {
    return this.delay(() => this.share.publicLink(kind, id));
  }
  deepLinkTake() {
    return this.delay(() => this.share.take());
  }
}

export function createMockBackend(opts?: Partial<MockOptions>): MockBackend {
  const fromUrl = typeof window !== "undefined" ? optionsFromUrl(window.location.search) : optionsFromUrl("");
  const mock = new MockBackend({ ...fromUrl, ...opts });
  if (typeof window !== "undefined") (window as unknown as { __platterMock?: MockBackend }).__platterMock = mock;
  return mock;
}
