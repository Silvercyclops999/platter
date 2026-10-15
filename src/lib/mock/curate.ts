/**
 * Mock hidden songs and artists, bookmarks, smart playlists (with a pretend
 * Plex and AI provider) and scrobbling, over the fixtures.
 *
 * Test hooks: `smart.refusePlex` (Plex turns the next rules away),
 * `smart.offline`, `scrobble.reject("lastfm" | "listenbrainz")`.
 */
import type {
  AiStatus,
  Album,
  Bookmark,
  HiddenItem,
  HideRequest,
  NewBookmark,
  Playlist,
  PromptResult,
  ScrobbleStatus,
  Settings,
  SmartPage,
  SmartPreview,
  SmartRule,
  SmartRules,
  SmartSaveRequest,
  SmartSaved,
  SmartSummary,
  Track,
} from "../types";
import type { Fixtures } from "./fixtures";

const DAY = 86400;
const now = () => Math.floor(Date.now() / 1000);

function invalid(message: string): never {
  throw { kind: "invalid", message };
}

// ---------------------------------------------------------------- hidden

export class MockHidden {
  private items: HiddenItem[] = [];
  constructor(private emit: (all: HiddenItem[]) => void) {}

  reset() {
    this.items = [];
  }

  list(): HiddenItem[] {
    return [...this.items].sort((a, b) => b.hiddenAt - a.hiddenAt);
  }

  set(req: HideRequest, on: boolean): HiddenItem[] {
    if (!req.id.trim()) invalid("Nothing to hide.");
    const had = this.items.some((h) => h.kind === req.kind && h.id === req.id);
    if (on && !had) this.items.push({ kind: req.kind, id: req.id, title: req.title, subtitle: req.subtitle ?? null, thumb: req.thumb ?? null, hiddenAt: now() + this.items.length });
    else if (!on && had) this.items = this.items.filter((h) => !(h.kind === req.kind && h.id === req.id));
    else return this.list();
    const all = this.list();
    this.emit(all);
    return all;
  }

  /** Whether mixes, radio, autoplay and shuffle skip this song. */
  hides(t: Pick<Track, "id" | "artistId">): boolean {
    return this.items.some((h) => (h.kind === "track" ? h.id === t.id : h.id === t.artistId));
  }
}

// ---------------------------------------------------------------- bookmarks

export class MockBookmarks {
  private items: Bookmark[] = [];
  private seq = 1;
  constructor(
    private hooks: { emit: (all: Bookmark[]) => void; playAt: (trackId: string, ms: number) => void },
    fx: Fixtures,
  ) {
    this.seed(fx);
  }

  private seed(fx: Fixtures) {
    const t = fx.tracks[3];
    const a = fx.albums[1];
    const at = now();
    this.items = [
      { id: "bm-1", kind: "track", target: t.id, title: t.title, subtitle: t.artistTitle, thumb: t.thumb, positionMs: 83_000, createdAt: at - 2 * DAY },
      { id: "bm-2", kind: "page", target: a.id, page: "album", title: a.title, subtitle: a.artistTitle, thumb: a.thumb, createdAt: at - 5 * DAY },
    ];
  }

  reset(fx: Fixtures) {
    this.seed(fx);
  }

  list(): Bookmark[] {
    return [...this.items].sort((a, b) => b.createdAt - a.createdAt || b.id.localeCompare(a.id));
  }

  add(b: NewBookmark): Bookmark {
    if (!b.target.trim() && b.kind === "track") invalid("Nothing to bookmark.");
    const pos = b.kind === "track" ? (b.positionMs ?? 0) : null;
    const same = this.items.find(
      (x) => x.kind === b.kind && x.target === b.target && (x.page ?? null) === (b.page ?? null) && (pos === null || Math.floor((x.positionMs ?? 0) / 1000) === Math.floor(pos / 1000)),
    );
    if (same) return same;
    const bm: Bookmark = { ...b, id: `bm-${++this.seq + 100}`, positionMs: pos, page: b.kind === "page" ? b.page : null, createdAt: now() + this.seq };
    this.items.push(bm);
    this.hooks.emit(this.list());
    return bm;
  }

  remove(id: string): Bookmark[] {
    this.items = this.items.filter((b) => b.id !== id);
    const all = this.list();
    this.hooks.emit(all);
    return all;
  }

  open(id: string): Bookmark {
    const b = this.items.find((x) => x.id === id);
    if (!b) throw { kind: "notFound", message: "bookmark" };
    if (b.kind === "track") this.hooks.playAt(b.target, b.positionMs ?? 0);
    return b;
  }
}

// ---------------------------------------------------------------- smart playlists

interface MockSmartList {
  id: string;
  title: string;
  summary: string | null;
  rules: SmartRules;
  picks: string[];
  prompt: string | null;
  createdAt: number;
  updatedAt: number;
}

const LABEL: Record<string, string> = {
  playedHere: "Played on this computer",
  duration: "Length",
  releaseType: "Release type",
  explicit: "Explicit",
  version: "Version",
};

/** Why Plex can't hold these rules (same rules as the core). */
export function plexGaps(r: SmartRules): string[] {
  const out: string[] = [];
  for (const rule of r.rules) {
    const l = LABEL[rule.field];
    if (l && !out.includes(`${l} isn't something Plex can filter on`)) out.push(`${l} isn't something Plex can filter on`);
  }
  if (r.limit?.unit === "hours") out.push("Plex limits by number of songs, not hours");
  if (r.update !== "live") out.push("Plex smart playlists are always up to date");
  return out;
}

function span(v: string): number | null {
  const m = /^(\d+)(d|w|m|y)$/.exec(v.trim().toLowerCase());
  if (!m) return null;
  return Number(m[1]) * { d: DAY, w: 7 * DAY, m: 30 * DAY, y: 365 * DAY }[m[2] as "d"];
}

function checkRule(r: SmartRule): boolean {
  const v = r.value.trim();
  switch (r.field) {
    case "genre":
    case "style":
    case "mood":
    case "artist":
    case "album":
    case "version":
      return v.length > 0;
    case "year":
    case "playCount":
    case "duration":
      return v !== "" && Number(v) >= 0;
    case "rating":
      return v !== "" && Number(v) >= 0 && Number(v) <= 5;
    case "decade":
      return /^\d{4}$/.test(v) && Number(v) % 10 === 0;
    case "addedAt":
    case "lastPlayed":
    case "playedHere":
      return span(v) !== null;
    case "liked":
    case "explicit":
      return v === "true" || v === "false";
    case "releaseType":
      return v.length > 0;
  }
}

function text(op: SmartRule["op"], have: string[], want: string): boolean {
  const w = want.trim().toLowerCase();
  const h = have.map((x) => x.toLowerCase());
  switch (op) {
    case "is":
      return h.includes(w);
    case "isNot":
      return !h.includes(w);
    case "contains":
      return h.some((x) => x.includes(w));
    case "notContains":
      return !h.some((x) => x.includes(w));
    default:
      return false;
  }
}

function num(op: SmartRule["op"], have: number, want: number): boolean {
  return op === "is" ? have === want : op === "isNot" ? have !== want : op === "gt" ? have > want : op === "lt" ? have < want : false;
}

export class MockSmart {
  private lists: MockSmartList[] = [];
  private keys = new Set<string>();
  /** Plex turns the next rule set away. */
  refusePlex = false;
  offline = false;
  constructor(
    private fx: Fixtures,
    private hooks: {
      settings: () => Settings;
      emit: (all: SmartSummary[]) => void;
      libraryChanged: () => void;
      track: (id: string) => Track | undefined;
      album: (id: string) => Album | undefined;
    },
  ) {}

  reset() {
    this.lists = [];
    this.keys.clear();
    this.refusePlex = false;
    this.offline = false;
  }

  private matches(r: SmartRule, t: Track, a: Album | undefined): boolean {
    const v = r.value;
    const t0 = now();
    switch (r.field) {
      case "genre":
        return text(r.op, [...(a?.genres ?? []), ...(t.genres ?? [])], v);
      case "style":
        return text(r.op, [], v);
      case "mood":
        return text(r.op, t.genres ?? [], v);
      case "artist":
        return text(r.op, [t.artistTitle, ...(t.trackArtist ? [t.trackArtist] : [])], v);
      case "album":
        return text(r.op, [t.albumTitle], v);
      case "version":
        return text(r.op, [], v);
      case "year":
        return num(r.op, a?.year ?? 0, Number(v));
      case "decade": {
        const is = a?.year != null && a.year - (a.year % 10) === Number(v);
        return r.op === "is" ? is : !is;
      }
      case "playCount":
        return num(r.op, t.viewCount, Number(v));
      case "rating":
        return num(r.op, (t.userRating ?? 0) / 2, Number(v));
      case "duration":
        return num(r.op, t.durationMs / 60_000, Number(v));
      case "addedAt":
      case "lastPlayed":
      case "playedHere": {
        const at = r.field === "addedAt" ? (a?.addedAt ?? null) : r.field === "lastPlayed" ? t.lastViewedAt : null;
        const within = at !== null && at >= t0 - (span(v) ?? 0);
        return r.op === "inLast" ? within : !within;
      }
      case "liked":
        return ((t.userRating ?? 0) >= 10) === (v === "true");
      case "explicit":
        return v === "false";
      case "releaseType": {
        const is = (a?.section ?? "albums") === v;
        return r.op === "is" ? is : !is;
      }
    }
  }

  /** The songs, in order, after rules, picks, sort and limit. */
  songs(rules: SmartRules, picks: string[] = []): Track[] {
    let hit = this.fx.tracks.filter((t) => {
      if (!rules.rules.length) return true;
      const a = this.hooks.album(t.albumId);
      const each = rules.rules.map((r) => this.matches(r, t, a));
      return rules.matchAll ? each.every(Boolean) : each.some(Boolean);
    });
    const by = {
      none: () => 0,
      random: (x: Track, y: Track) => (x.id.length * 31 + x.id.charCodeAt(x.id.length - 1)) % 7 - ((y.id.length * 31 + y.id.charCodeAt(y.id.length - 1)) % 7),
      mostPlayed: (x: Track, y: Track) => y.viewCount - x.viewCount,
      leastPlayed: (x: Track, y: Track) => x.viewCount - y.viewCount,
      recentlyAdded: (x: Track, y: Track) => (this.hooks.album(y.albumId)?.addedAt ?? 0) - (this.hooks.album(x.albumId)?.addedAt ?? 0),
      recentlyPlayed: (x: Track, y: Track) => (y.lastViewedAt ?? 0) - (x.lastViewedAt ?? 0),
      rating: (x: Track, y: Track) => (y.userRating ?? 0) - (x.userRating ?? 0),
    }[rules.sort];
    hit = [...hit].sort(by);
    const picked = picks.map((id) => this.hooks.track(id)).filter((t): t is Track => !!t);
    let out = [...picked, ...hit.filter((t) => !picks.includes(t.id))];
    if (rules.limit?.unit === "songs") out = out.slice(0, rules.limit.value);
    if (rules.limit?.unit === "hours") {
      const cap = rules.limit.value * 3_600_000;
      let total = 0;
      out = out.filter((t) => {
        if (total + t.durationMs > cap) return false;
        total += t.durationMs;
        return true;
      });
    }
    return out;
  }

  private check(rules: SmartRules) {
    for (const r of rules.rules) if (!checkRule(r)) invalid(`Check the value for ${r.field}.`);
    if (rules.limit && (rules.limit.value <= 0 || !Number.isFinite(rules.limit.value))) invalid("Check the limit.");
  }

  preview(rules: SmartRules, picks: string[] = []): SmartPreview {
    this.check(rules);
    const songs = this.songs(rules, picks);
    return { count: songs.length, durationMs: songs.reduce((s, t) => s + t.durationMs, 0), tracks: songs.slice(0, 50), plexGaps: plexGaps(rules) };
  }

  private summary(l: MockSmartList, tracks?: Track[]): SmartSummary {
    const ts = tracks ?? this.songs(l.rules, l.picks);
    const period = l.rules.update === "daily" ? DAY : l.rules.update === "weekly" ? 7 * DAY : null;
    const t0 = now();
    return {
      id: l.id,
      title: l.title,
      summary: l.summary,
      trackCount: ts.length,
      durationMs: ts.reduce((s, t) => s + t.durationMs, 0),
      update: l.rules.update,
      nextUpdateAt: period ? t0 - (t0 % period) + period : null,
      updatedAt: l.updatedAt,
      prompted: !!l.prompt,
    };
  }

  list(): SmartSummary[] {
    return this.lists.map((l) => this.summary(l));
  }

  page(id: string): SmartPage {
    const l = this.lists.find((x) => x.id === id);
    if (!l) throw { kind: "notFound", message: `smart playlist ${id}` };
    const tracks = this.songs(l.rules, l.picks);
    return { playlist: this.summary(l, tracks), rules: l.rules, picks: l.picks, prompt: l.prompt, tracks };
  }

  private changed() {
    this.hooks.emit(this.list());
    this.hooks.libraryChanged();
  }

  private savePlatter(req: SmartSaveRequest, note: string | null): SmartSaved {
    const t = now();
    let id = req.id && req.id.startsWith("smart:") ? req.id : null;
    const existing = id ? this.lists.find((l) => l.id === id) : undefined;
    const fields = { title: req.title.trim(), summary: req.summary?.trim() || null, rules: req.rules, picks: req.picks ?? [], updatedAt: t };
    if (existing) Object.assign(existing, fields, req.prompt ? { prompt: req.prompt } : {});
    else {
      id = `smart:${Math.random().toString(16).slice(2, 10)}`;
      this.lists.push({ id, prompt: req.prompt ?? null, createdAt: t, ...fields });
    }
    this.changed();
    return { id: id!, platter: true, note };
  }

  private savePlex(req: SmartSaveRequest, id: string | null): SmartSaved {
    const songs = this.songs(req.rules).map((t) => t.id);
    let p: Playlist | undefined = id ? this.fx.playlists.find((x) => x.id === id) : undefined;
    if (!p) {
      p = { id: `${70000 + this.fx.playlists.length}`, title: req.title.trim(), smart: true, trackCount: 0, durationMs: 0, thumb: null };
      this.fx.playlists.push(p);
    }
    p.title = req.title.trim();
    p.trackCount = songs.length;
    p.durationMs = songs.reduce((s, x) => s + (this.hooks.track(x)?.durationMs ?? 0), 0);
    this.fx.playlistTracks.set(p.id, songs);
    this.plexRules.set(p.id, req.rules);
    this.hooks.libraryChanged();
    return { id: p.id, platter: false, note: null };
  }

  private plexRules = new Map<string, SmartRules>();

  save(req: SmartSaveRequest): SmartSaved {
    this.check(req.rules);
    if (!req.title.trim()) invalid("Playlist name is required.");
    const gaps = plexGaps(req.rules);
    if (req.id && req.id.startsWith("smart:")) return this.savePlatter(req, null);
    if (req.id) {
      if (gaps.length || req.picks?.length)
        invalid(`This playlist lives in Plex, and ${(gaps[0] ?? "Plex can't keep picked songs").toLowerCase()}. Save these rules as a new Platter smart playlist instead.`);
      if (this.offline) throw { kind: "network", message: "Your server is offline. Plex smart playlists change when it's back." };
      return this.savePlex(req, req.id);
    }
    if (req.platter || req.picks?.length) return this.savePlatter(req, null);
    if (gaps.length) return this.savePlatter(req, `Saved as a Platter smart playlist on this computer: ${gaps.join("; ")}.`);
    if (this.offline) throw { kind: "network", message: "Your server is offline. Save it as a Platter smart playlist, or try again when the server is back." };
    if (this.refusePlex) {
      this.refusePlex = false;
      return this.savePlatter(req, "Plex didn't accept these rules, so this is a Platter smart playlist on this computer.");
    }
    return this.savePlex(req, null);
  }

  delete(id: string) {
    if (!id.startsWith("smart:")) invalid("Delete Plex playlists from the playlist's menu.");
    this.lists = this.lists.filter((l) => l.id !== id);
    this.changed();
  }

  /** A Plex smart playlist's rules: ones saved here, or a stand-in per fixture. */
  plexRulesOf(id: string): SmartRules {
    const known = this.plexRules.get(id);
    if (known) return known;
    const p = this.fx.playlists.find((x) => x.id === id);
    if (!p?.smart) invalid("This isn't a smart playlist.");
    const genre = this.fx.albums[0].genres[0] ?? "Rock";
    return { matchAll: true, rules: [{ field: "genre", op: "is", value: genre }, { field: "playCount", op: "gt", value: "0" }], limit: { unit: "songs", value: 100 }, sort: "mostPlayed", update: "live" };
  }

  // ------------------------------------------------------------ AI provider

  aiStatus(): AiStatus {
    const s = this.hooks.settings();
    const hasKey = this.keys.has("anthropic");
    return { provider: s.aiProvider, model: s.aiModel || "claude-opus-5-5", hasKey, ready: s.aiProvider !== "none" && hasKey };
  }

  aiKeySet(key: string | null): AiStatus {
    if (key && key.trim()) this.keys.add("anthropic");
    else this.keys.delete("anthropic");
    return this.aiStatus();
  }

  prompt(text: string): PromptResult {
    if (!this.aiStatus().ready) invalid("Choose an AI provider and add its key in Settings first.");
    if (!text.trim()) invalid("Describe the playlist first.");
    const genres = [...new Set(this.fx.albums.flatMap((a) => a.genres))];
    const hit = genres.find((g) => text.toLowerCase().includes(g.toLowerCase())) ?? genres[0];
    const rules: SmartRules = { matchAll: true, rules: [{ field: "genre", op: "is", value: hit }], limit: { unit: "songs", value: 30 }, sort: "mostPlayed", update: "live" };
    const picks = this.songs({ ...rules, limit: null }).slice(0, 8).reverse();
    const title = text.trim().replace(/^./, (c) => c.toUpperCase()).slice(0, 40);
    return { title, rules, picks };
  }
}

// ---------------------------------------------------------------- scrobbling

export class MockScrobble {
  private st: ScrobbleStatus = MockScrobble.fresh();
  private keys = { lastfmKey: true, secret: false, session: false, lbToken: false };
  constructor(private emit: (queued: number) => void) {}

  private static fresh(): ScrobbleStatus {
    return {
      lastfm: { hasKey: true, hasSecret: false, signedIn: false, user: null, rejected: false, pending: false },
      listenbrainz: { hasToken: false, user: null, rejected: false },
      queued: 0,
    };
  }

  reset() {
    this.st = MockScrobble.fresh();
    this.keys = { lastfmKey: true, secret: false, session: false, lbToken: false };
  }

  status(): ScrobbleStatus {
    return structuredClone(this.st);
  }

  /** The ListenBrainz token was saved in Artist info. */
  setListenBrainzToken(on: boolean) {
    this.keys.lbToken = on;
    this.st.listenbrainz = { hasToken: on, user: null, rejected: false };
  }

  setSecret(secret: string | null): ScrobbleStatus {
    this.keys.secret = !!secret?.trim();
    this.st.lastfm.hasSecret = this.keys.secret;
    return this.status();
  }

  start(): ScrobbleStatus {
    if (!this.st.lastfm.hasKey) invalid("Add your Last.fm API key first.");
    if (!this.st.lastfm.hasSecret) invalid("Add your Last.fm shared secret first.");
    this.st.lastfm.pending = true;
    return this.status();
  }

  finish(): ScrobbleStatus {
    if (!this.st.lastfm.pending) invalid("Start signing in to Last.fm first.");
    this.st.lastfm = { ...this.st.lastfm, signedIn: true, user: "listener", rejected: false, pending: false };
    this.st.queued = 0;
    this.emit(0);
    return this.status();
  }

  signOut(): ScrobbleStatus {
    this.st.lastfm = { ...this.st.lastfm, signedIn: false, user: null, pending: false };
    return this.status();
  }

  check(): ScrobbleStatus {
    if (this.st.listenbrainz.hasToken) this.st.listenbrainz = { hasToken: true, user: "listener", rejected: false };
    return this.status();
  }

  /** The service turned the sign-in away (test hook). */
  reject(service: "lastfm" | "listenbrainz") {
    if (service === "lastfm") this.st.lastfm.rejected = true;
    else this.st.listenbrainz.rejected = true;
    this.st.queued += 2;
    this.emit(this.st.queued);
  }
}
