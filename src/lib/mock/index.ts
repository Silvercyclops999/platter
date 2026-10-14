/**
 * In-memory mock of the Rust core, used outside Tauri (Vite dev server,
 * Playwright, Vitest). Implements every command and event in docs/IPC.md with
 * invented fixture data, simulated playback (position ticks 4×/s, queue rules
 * roughly like the real core), sync progress and health changes.
 *
 * URL switches (plain browser only):
 *   ?latency=ms     delay every command (default 120; 0 in tests)
 *   ?signedOut=1    start on the sign-in screen
 *   ?albums=5000    pad the album list to N items (virtual grid perf check)
 *   ?httpArt=1      art URLs in the Windows app's form (http://art.localhost/…?w=&h=)
 *                   instead of generated SVGs, for memory checks that serve
 *                   real JPEGs at the requested size (scripts/measure-memory.mjs)
 *   ?health=offline start in a given app state (see HEALTH_DEMOS in ./health:
 *                   offline, noInternet, starting, updating, startTimeout,
 *                   relay, remote, signIn, accessRemoved, filesUnavailable,
 *                   noMusicLibrary, diskFull, scanning)
 */
import type { Backend, Unlisten } from "../backend";
import type {
  Album,
  AlbumSort,
  ArtistSort,
  AuthStatus,
  EventMap,
  EventName,
  Health,
  PinInfo,
  PlayContextArgs,
  PlayerState,
  QueueContextInfo,
  QueueItem,
  RepeatMode,
  ReleaseSection,
  SearchArgs,
  SearchResults,
  ServerInfo,
  Settings,
  SyncStatus,
  Track,
} from "../types";
import { mockArtUrl } from "./art";
import { BROKEN_TRACK_TITLE_SUFFIX, buildFixtures, mulberry32, type Fixtures } from "./fixtures";
import { demoHealth, isHealthDemo, type HealthDemo } from "./health";

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
}

const isTestEnv = (() => {
  try {
    return import.meta.env?.MODE === "test";
  } catch {
    return false;
  }
})();

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

const clone = <T,>(v: T): T => (typeof structuredClone === "function" ? structuredClone(v) : JSON.parse(JSON.stringify(v)));

export class MockBackend implements Backend {
  readonly kind = "mock" as const;
  readonly fx: Fixtures;
  private opts: MockOptions;
  private listeners = new Map<EventName, Set<(p: unknown) => void>>();
  private timers = new Set<ReturnType<typeof setTimeout>>();

  // auth
  private auth: AuthStatus;
  // health / sync
  private healthState: Health;
  private sync: SyncStatus = { phase: "idle", done: 0, total: 0, lastSyncAt: Math.floor(Date.now() / 1000) - 3600 };
  // search
  private recent: string[] = ["lantern", "Odessa Wren", "saltglass"];
  private settings: Settings = { privateSession: false, reportToPlex: true, volume: 0.7 };

  // player
  private status: PlayerState["status"] = "idle";
  private positionMs = 0;
  private volume = 0.7;
  private muted = false;
  private shuffle = false;
  private repeat: RepeatMode = "off";
  private current: QueueItem | null = null;
  private userQueue: QueueItem[] = [];
  private ctxOriginal: QueueItem[] = [];
  private ctxOrder: QueueItem[] = [];
  private ctxPos = -1;
  private history: QueueItem[] = [];
  private ctxInfo: QueueContextInfo | null = null;
  private itemSeq = 1;
  private tick: ReturnType<typeof setInterval> | null = null;
  private rng = mulberry32(42);

  constructor(opts: Partial<MockOptions> = {}) {
    this.opts = { ...optionsFromUrl(""), ...opts };
    this.fx = buildFixtures();
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
  syncNow() {
    return this.delay(() => undefined);
  }
  home() {
    return this.delay(() => {
      const al = this.fx.albums;
      const recentlyPlayed = al
        .filter((a) => a.lastViewedAt)
        .sort((a, b) => (b.lastViewedAt ?? 0) - (a.lastViewedAt ?? 0))
        .slice(0, 12);
      const recentlyAdded = [...al].sort((a, b) => b.addedAt - a.addedAt).slice(0, 12);
      const jumpBackIn = [...al].sort((a, b) => b.viewCount - a.viewCount).slice(0, 12);
      return { recentlyPlayed, recentlyAdded, jumpBackIn };
    });
  }
  artists(args: { sort: ArtistSort; offset: number; limit: number }) {
    return this.delay(() => {
      const list = [...this.fx.artists];
      if (args.sort === "name") list.sort((a, b) => a.sortTitle.localeCompare(b.sortTitle));
      if (args.sort === "plays") list.sort((a, b) => b.viewCount - a.viewCount);
      return { items: list.slice(args.offset, args.offset + args.limit), total: list.length };
    });
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
  artistPage(id: string) {
    return this.delay(() => {
      const artist = this.fx.artists.find((a) => a.id === id) ?? this.notFound("Artist");
      const albums = this.fx.albums.filter((a) => a.artistId === id);
      const popular = this.fx.tracks
        .filter((t) => t.artistId === id)
        .sort((a, b) => b.viewCount - a.viewCount)
        .slice(0, 10);
      const order: ReleaseSection[] = ["albums", "singlesEps", "live", "compilations", "soundtracks", "remixes", "demos", "mixtapes"];
      const sections = order
        .map((section) => ({
          section,
          albums: albums.filter((a) => a.section === section).sort((a, b) => (b.year ?? 9999) - (a.year ?? 9999)),
        }))
        .filter((s) => s.albums.length > 0);
      return { artist, popular, sections };
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
      return { playlist, tracks: ids.map((tid) => this.trackById(tid)!).filter(Boolean) };
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
  recentSearches() {
    return this.delay(() => this.recent);
  }
  recentSearchAdd(query: string) {
    return this.delay(() => {
      const q = query.trim();
      if (!q) return;
      this.recent = [q, ...this.recent.filter((r) => r.toLowerCase() !== q.toLowerCase())].slice(0, 10);
    });
  }
  recentSearchesClear() {
    return this.delay(() => {
      this.recent = [];
    });
  }

  // ---------------------------------------------------------------- playback

  private item(track: Track, source: QueueItem["source"]): QueueItem {
    return { itemId: `q${this.itemSeq++}`, track, source };
  }

  private shuffled<T>(arr: T[]): T[] {
    const a = [...arr];
    for (let i = a.length - 1; i > 0; i--) {
      const j = Math.floor(this.rng() * (i + 1));
      [a[i], a[j]] = [a[j], a[i]];
    }
    return a;
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
    if (args.shuffle !== undefined) this.shuffle = args.shuffle;
    if (this.current) this.pushHistory(this.current);
    this.ctxOriginal = tracks.map((t) => this.item(t, "context"));
    const start = args.startTrackId ? this.ctxOriginal.find((i) => i.track.id === args.startTrackId) : undefined;
    if (this.shuffle) {
      const first = start ?? this.ctxOriginal[Math.floor(this.rng() * this.ctxOriginal.length)];
      this.ctxOrder = [first, ...this.shuffled(this.ctxOriginal.filter((i) => i !== first))];
      this.ctxPos = 0;
    } else {
      this.ctxOrder = [...this.ctxOriginal];
      this.ctxPos = start ? this.ctxOrder.indexOf(start) : 0;
    }
    this.ctxInfo = info;
    this.setCurrent(this.ctxOrder[this.ctxPos], play);
  }

  private pushHistory(item: QueueItem) {
    this.history.push(item);
    if (this.history.length > 50) this.history.shift();
  }

  private setCurrent(item: QueueItem | null, play: boolean) {
    this.current = item;
    this.positionMs = 0;
    if (item && item.track.title.endsWith(BROKEN_TRACK_TITLE_SUFFIX) && play) {
      // Spec 07: one file missing/corrupt → toast + skip.
      // The UI turns player://error into the "Skipped" toast.
      this.emit("player://error", { trackId: item.track.id, kind: "io", message: "Skipped: file unavailable" });
      this.advance(false);
      return;
    }
    this.status = item ? (play ? "playing" : "paused") : "idle";
    if (this.status === "playing") this.startTick();
    else this.stopTick();
    this.emitState();
  }

  private advance(natural: boolean) {
    if (natural && this.repeat === "one" && this.current) {
      this.positionMs = 0;
      this.emitState();
      return;
    }
    if (this.current) this.pushHistory(this.current);
    if (this.userQueue.length) {
      this.setCurrent(this.userQueue.shift()!, true);
    } else if (this.ctxPos + 1 < this.ctxOrder.length) {
      this.ctxPos++;
      this.setCurrent(this.ctxOrder[this.ctxPos], true);
    } else if (this.repeat === "all" && this.ctxOrder.length) {
      if (this.shuffle) this.ctxOrder = this.shuffled(this.ctxOrder);
      this.ctxPos = 0;
      this.setCurrent(this.ctxOrder[0], true);
    } else {
      // End of everything: stay on the last track, paused at 0.
      if (this.current) this.history.pop();
      this.status = "paused";
      this.positionMs = 0;
      this.stopTick();
      this.emitState();
    }
  }

  private startTick() {
    if (this.tick) return;
    this.tick = setInterval(() => {
      if (this.status !== "playing" || !this.current) return;
      this.positionMs += 250;
      const d = this.current.track.durationMs;
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
      repeat: this.repeat,
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
  private pauseNow() {
    if (this.status !== "playing") return;
    this.status = "paused";
    this.stopTick();
    this.emitState();
  }
  private resumeNow() {
    if (!this.current) return;
    this.status = "playing";
    this.startTick();
    this.emitState();
  }
  playNext() {
    return this.act(() => this.advance(false));
  }
  playPrevious() {
    return this.act(() => {
      if (this.positionMs > 3000 || !this.history.length) {
        this.positionMs = 0;
        this.emitState();
        return;
      }
      const prev = this.history.pop()!;
      if (this.current?.source === "user") this.userQueue.unshift(this.current);
      if (prev.source === "context") {
        const idx = this.ctxOrder.indexOf(prev);
        if (idx >= 0) this.ctxPos = idx;
      }
      this.setCurrent(prev, this.status === "playing" || this.status === "paused" ? true : false);
    });
  }
  playSeek(positionMs: number) {
    return this.act(() => {
      this.positionMs = Math.max(0, Math.min(positionMs, this.current?.track.durationMs ?? 0));
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
    return this.act(() => {
      if (shuffle === this.shuffle) return;
      this.shuffle = shuffle;
      const played = this.ctxOrder.slice(0, this.ctxPos + 1);
      if (shuffle) {
        this.ctxOrder = [...played, ...this.shuffled(this.ctxOrder.slice(this.ctxPos + 1))];
      } else if (this.current) {
        // Back to original order, continuing after the current track.
        const at = this.ctxOriginal.findIndex((i) => i.itemId === this.current!.itemId);
        if (at >= 0) {
          this.ctxOrder = [...this.ctxOriginal];
          this.ctxPos = at;
        }
      }
      this.emitState();
    });
  }
  setRepeat(repeat: RepeatMode) {
    return this.act(() => {
      this.repeat = repeat;
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
      this.settings = { ...this.settings, ...patch };
      if (typeof patch.volume === "number") this.volume = Math.min(1, Math.max(0, patch.volume));
    });
  }
  updateCheck() {
    return this.delay(() => ({ available: false, version: null, notes: null }));
  }
  updateInstallOnRestart() {
    return this.delay(() => undefined);
  }
}

export function createMockBackend(opts?: Partial<MockOptions>): MockBackend {
  const fromUrl = typeof window !== "undefined" ? optionsFromUrl(window.location.search) : optionsFromUrl("");
  const mock = new MockBackend({ ...fromUrl, ...opts });
  if (typeof window !== "undefined") (window as unknown as { __platterMock?: MockBackend }).__platterMock = mock;
  return mock;
}
