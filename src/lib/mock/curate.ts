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
  Described,
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
  era: "Era",
  unreleased: "Unreleased",
  lossless: "Lossless",
  skips: "Skips",
  finished: "Album finished",
  played: "Played",
  trackNumber: "Track number",
  playlist: "Playlist",
  sampleRate: "Sample rate",
  bitDepth: "Bit depth",
};

/** Every rule, those in groups too. */
export const allRules = (r: SmartRules): SmartRule[] => [...r.rules, ...(r.groups ?? []).flatMap((g) => g.rules)];

/** "1990..1999": two numbers, the first not more than the second. */
function numberRange(v: string): [number, number] | null {
  const m = /^\s*(\d+(?:\.\d+)?)\s*\.\.\s*(\d+(?:\.\d+)?)\s*$/.exec(v);
  if (!m) return null;
  const [a, b] = [Number(m[1]), Number(m[2])];
  return a <= b ? [a, b] : null;
}

/** Why Plex can't hold these rules (same rules as the core). */
export function plexGaps(r: SmartRules): string[] {
  const out: string[] = [];
  const add = (s: string) => !out.includes(s) && out.push(s);
  const dates = ["addedAt", "lastPlayed", "lastSkipped"];
  for (const rule of allRules(r)) {
    const l = LABEL[rule.field];
    if (l) add(`${l} isn't something Plex can filter on`);
    else if (rule.op === "between" && dates.includes(rule.field)) add("Plex can't filter between two dates");
    else if (rule.op === "between") {
      const scale = rule.field === "rating" ? 2 : 1;
      const range = numberRange(rule.value);
      if (!range || !Number.isInteger(range[0] * scale) || !Number.isInteger(range[1] * scale))
        add(`Plex filters ${rule.field === "playCount" ? "plays" : rule.field} between whole numbers only`);
    }
  }
  if (r.limit?.unit === "hours") out.push("Plex limits by number of songs, not hours");
  if (r.update !== "live") out.push("Plex smart playlists are always up to date");
  return out;
}

/** "2026-06-01..2026-08-31" as local unix seconds, end of the last day included. */
function range(v: string): [number, number] | null {
  const m = /^(\d{4})-(\d{2})-(\d{2})\.\.(\d{4})-(\d{2})-(\d{2})$/.exec(v.trim());
  if (!m) return null;
  const a = new Date(+m[1], +m[2] - 1, +m[3]).getTime() / 1000;
  const b = new Date(+m[4], +m[5] - 1, +m[6] + 1).getTime() / 1000;
  return a < b ? [a, b] : null;
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
    case "era":
    case "title":
    case "playlist":
      return v.length > 0;
    case "year":
    case "playCount":
    case "skips":
    case "duration":
    case "trackNumber":
    case "sampleRate":
    case "bitDepth":
      return r.op === "between" ? numberRange(v) !== null : v !== "" && Number(v) >= 0;
    case "rating":
      if (r.op === "between") return (numberRange(v)?.[1] ?? 6) <= 5;
      return v !== "" && Number(v) >= 0 && Number(v) <= 5;
    case "decade":
      return /^\d{4}$/.test(v) && Number(v) % 10 === 0;
    case "addedAt":
    case "lastPlayed":
    case "playedHere":
    case "played":
    case "lastSkipped":
      return r.op === "between" ? range(v) !== null : span(v) !== null;
    case "liked":
    case "explicit":
    case "unreleased":
    case "lossless":
    case "finished":
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
    case "beginsWith":
      return h.some((x) => x.startsWith(w));
    case "endsWith":
      return h.some((x) => x.endsWith(w));
    default:
      return false;
  }
}

function num(op: SmartRule["op"], have: number | null, want: number, value = ""): boolean {
  if (have === null) return op === "isNot";
  if (op === "between") {
    const r = numberRange(value);
    return !!r && have >= r[0] && have <= r[1];
  }
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
      case "title":
        return text(r.op, [t.title], v);
      case "trackNumber":
        return num(r.op, t.trackNumber || null, Number(v), v);
      case "sampleRate":
        return num(r.op, t.sampleRate ? t.sampleRate / 1000 : null, Number(v), v);
      case "bitDepth":
        return num(r.op, t.bitDepth ?? null, Number(v), v);
      case "playlist": {
        const names = this.fx.playlists.filter((p) => (this.fx.playlistTracks.get(p.id) ?? []).includes(t.id)).map((p) => p.title.toLowerCase());
        const is = names.includes(v.trim().toLowerCase());
        return r.op === "is" ? is : !is;
      }
      case "lastSkipped":
        return r.op === "notInLast";
      case "version":
      case "era":
        return text(r.op, [], v);
      case "skips":
        return num(r.op, 0, Number(v));
      case "unreleased": {
        const is = a?.section === "demos" || !!a?.format?.includes("demo");
        return is === (v === "true");
      }
      case "lossless":
        return /^(flac|alac|wav|aiff)$/i.test(t.codec ?? "") === (v === "true");
      case "finished": {
        const all = this.fx.tracks.filter((x) => x.albumId === t.albumId);
        return all.every((x) => x.viewCount > 0) === (v === "true");
      }
      case "year":
        return num(r.op, a?.year ?? 0, Number(v), v);
      case "decade": {
        const is = a?.year != null && a.year - (a.year % 10) === Number(v);
        return r.op === "is" ? is : !is;
      }
      case "playCount":
        return num(r.op, t.viewCount, Number(v), v);
      case "rating":
        return num(r.op, (t.userRating ?? 0) / 2, Number(v), v);
      case "duration":
        return num(r.op, t.durationMs / 60_000, Number(v), v);
      case "addedAt":
      case "lastPlayed":
      case "playedHere":
      case "played": {
        const at = r.field === "addedAt" ? (a?.addedAt ?? null) : r.field === "lastPlayed" || r.field === "played" ? t.lastViewedAt : null;
        if (r.op === "between") {
          const span = range(v);
          return at !== null && !!span && at >= span[0] && at < span[1];
        }
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
      const groups = rules.groups ?? [];
      if (!rules.rules.length && !groups.length) return true;
      const a = this.hooks.album(t.albumId);
      const one = (r: SmartRule) => this.matches(r, t, a);
      const each = [...rules.rules.map(one), ...groups.map((g) => (g.matchAll ? g.rules.every(one) : g.rules.some(one)))];
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
    for (const r of allRules(rules)) if (!checkRule(r)) invalid(`Check the value for ${r.field}.`);
    if ((rules.groups ?? []).some((g) => !g.rules.length)) invalid("A group needs at least one rule.");
    if (rules.limit && (rules.limit.value <= 0 || !Number.isFinite(rules.limit.value))) invalid("Check the limit.");
  }

  preview(rules: SmartRules, picks: string[] = []): SmartPreview {
    this.check(rules);
    const songs = this.songs(rules, picks);
    const names = this.fx.playlists.map((p) => p.title.toLowerCase());
    const notes = [
      ...new Set(
        allRules(rules)
          .filter((r) => r.field === "playlist" && !names.includes(r.value.trim().toLowerCase()))
          .map((r) => `No playlist is called "${r.value.trim()}", so that rule can't match it.`),
      ),
    ];
    return { count: songs.length, durationMs: songs.reduce((s, t) => s + t.durationMs, 0), tracks: songs.slice(0, 50), plexGaps: plexGaps(rules), notes };
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

  /** Describe a playlist: a small reading of the words over the fixtures' names. */
  describe(text: string): Described {
    if (!text.trim()) invalid("Describe the playlist first.");
    return mockDescribe(text, {
      genre: [...new Set(this.fx.albums.flatMap((a) => a.genres))],
      artist: this.fx.artists.map((a) => a.title),
      album: this.fx.albums.map((a) => a.title),
    });
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

/** Folded words: lower case, no accents, hyphens split. */
function foldWords(s: string): string[] {
  return s
    .normalize("NFKD")
    .replace(/[\u0300-\u036f']/g, "")
    .toLowerCase()
    .split(/[^a-z0-9+]+/)
    .filter(Boolean);
}

const MOCK_FILLER = new Set(["a", "an", "the", "some", "my", "me", "i", "from", "in", "of", "with", "and", "songs", "song", "stuff", "music", "tracks", "only", "just", "by", "style", "like", "that", "era"]);
const MOCK_MOODS: Record<string, string> = { chill: "Ambient", mellow: "Ambient", upbeat: "Electronic", dreamy: "Dream Pop" };

/** The mock's reading: library names, a few fixed phrases, unknown words listed. */
export function mockDescribe(text: string, names: { genre: string[]; artist: string[]; album: string[] }): Described {
  const w = foldWords(text);
  const rules: SmartRule[] = [];
  const unknown: string[] = [];
  const add = (r: SmartRule) => {
    if (!rules.some((x) => x.field === r.field && x.op === r.op && x.value === r.value)) rules.push(r);
  };
  const lists: [SmartRule["field"], string[]][] = [
    ["genre", names.genre],
    ["artist", names.artist],
    ["album", names.album],
  ];
  let i = 0;
  let negate = false;
  let orSeen = false;
  let lastUnknown = -2;
  outer: while (i < w.length) {
    const at = (k: number) => w[i + k] ?? "";
    const rest = w.slice(i).join(" ");
    const phrase = (re: RegExp, make: (m: RegExpMatchArray) => SmartRule | null): boolean => {
      const m = rest.match(re);
      if (!m || m.index !== 0) return false;
      const r = make(m);
      if (r) add(r);
      i += m[0].split(" ").length;
      return true;
    };
    if (phrase(/^(havent|never|not) played (lately|recently)/, () => ({ field: "lastPlayed", op: "notInLast", value: "3m" }))) continue;
    if (phrase(/^(never|not) played/, () => ({ field: "playCount", op: "is", value: "0" }))) continue;
    if (phrase(/^(never|not) finished/, () => ({ field: "finished", op: "is", value: "false" }))) continue;
    if (phrase(/^(under|less than|shorter than) (\d+) (minutes|minute|min|mins)/, (m) => ({ field: "duration", op: "lt", value: m[2] }))) continue;
    if (phrase(/^(over|more than|longer than) (\d+) (minutes|minute|min|mins)/, (m) => ({ field: "duration", op: "gt", value: m[2] }))) continue;
    if (phrase(/^(last summer)/, () => {
      const now = new Date();
      const y = now.getMonth() >= 8 ? now.getFullYear() : now.getFullYear() - 1;
      return { field: "played", op: "between", value: `${y}-06-01..${y}-08-31` };
    }))
      continue;
    if (phrase(/^i (loved|liked|like|love)/, () => ({ field: "liked", op: "is", value: "true" }))) continue;
    const word = at(0);
    if (word === "or") {
      orSeen = true;
      i++;
      continue;
    }
    if (["no", "not", "without"].includes(word)) {
      negate = true;
      i++;
      continue;
    }
    const year = /^(19|20)\d\d$/.test(word) ? word : null;
    const decade = /^(\d0|\d{3}0)s$/.exec(word);
    const kw: Record<string, SmartRule> = {
      unreleased: { field: "unreleased", op: "is", value: "true" },
      leaks: { field: "unreleased", op: "is", value: "true" },
      lossless: { field: "lossless", op: "is", value: "true" },
      flac: { field: "lossless", op: "is", value: "true" },
      liked: { field: "liked", op: "is", value: "true" },
      skipped: { field: "skips", op: "gt", value: "0" },
      explicit: { field: "explicit", op: "is", value: "true" },
      albums: { field: "releaseType", op: "is", value: "albums" },
      live: { field: "releaseType", op: "is", value: "live" },
    };
    let r: SmartRule | null = kw[word] ?? null;
    if (year) r = { field: "year", op: "is", value: year };
    if (decade) {
      const n = Number(decade[1]);
      r = { field: "decade", op: "is", value: String(n >= 1000 ? n : n >= 30 ? 1900 + n : 2000 + n) };
    }
    if (!r)
      for (const [field, list] of lists)
        for (const name of list) {
          const nw = foldWords(name);
          if (nw.length && nw.every((x, k) => w[i + k] === x)) {
            add({ field, op: negate ? "isNot" : "is", value: name });
            negate = false;
            i += nw.length;
            continue outer;
          }
        }
    if (!r && MOCK_MOODS[word] && names.genre.includes(MOCK_MOODS[word])) r = { field: "genre", op: "is", value: MOCK_MOODS[word] };
    if (r) {
      add(negate ? { ...r, op: r.op === "is" && r.value !== "true" ? "isNot" : r.op, value: r.value === "true" ? "false" : r.value } : r);
      negate = false;
    } else if (!MOCK_FILLER.has(word)) {
      if (lastUnknown === i - 1 && unknown.length) unknown[unknown.length - 1] += ` ${word}`;
      else unknown.push(word);
      lastUnknown = i;
    }
    i++;
  }
  const title = text.trim().replace(/[.!?,]+$/, "").replace(/^./, (c) => c.toUpperCase()).slice(0, 100);
  // "Rock or jazz, lossless": either genre in a group, and the rest (as the core reads it).
  const fields = new Set(rules.map((r) => r.field));
  const doubled = [...fields].filter((f) => rules.filter((r) => r.field === f && r.op === "is" && r.value !== "true" && r.value !== "false").length > 1);
  if (orSeen && fields.size === 1 && rules.length > 1) return { title, rules: { matchAll: false, rules, limit: null, sort: "none", update: "live" }, unknown, choices: [], notes: [] };
  if (orSeen && doubled.length === 1 && fields.size > 1) {
    const either = (r: SmartRule) => r.field === doubled[0] && r.op === "is";
    return {
      title,
      rules: { matchAll: true, rules: rules.filter((r) => !either(r)), groups: [{ matchAll: false, rules: rules.filter(either) }], limit: null, sort: "none", update: "live" },
      unknown,
      choices: [],
      notes: [],
    };
  }
  return { title, rules: { matchAll: true, rules, limit: null, sort: "none", update: "live" }, unknown, choices: [], notes: [] };
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
