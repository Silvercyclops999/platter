/**
 * Listening stats for the mock backend, following the core's rules
 * (src-tauri/src/stats): a play counts when it wasn't a skip, ranges are
 * local days with both ends included, top lists rank by plays then
 * minutes, the chart is by day under 92 days and by month after. A demo
 * listening history of about eighteen months is invented from the fixture
 * library, plus whatever was played in this session.
 */
import type { Settings, Track } from "../types";
import type {
  CompareResult,
  CompareSummary,
  ItemStats,
  MonthRecap,
  StatsItemKind,
  StatsMoments,
  StatsOverview,
  StatsPoint,
  StatsRange,
  StatsSpan,
  StatsTopKind,
  StatsTotals,
  TopAlbum,
  TopArtist,
  TopGenre,
  TopSong,
  Personality,
  TimeOfDay,
  TopSongsIndex,
  TopSongsList,
  TopSongsRow,
  Wrapped,
} from "../statsTypes";
import { addDays, localDay, monthBounds, parseDay } from "../stats";
import { mockMatched, type Fixtures } from "./fixtures";
import type { MockListen } from "./recents";

const MIN = 60_000;
const DAY = 86_400_000;
export const DAILY_SERIES_MAX_DAYS = 92;
/** About three years, so last year is a whole year for Wrapped and Top Songs. */
const HISTORY_DAYS = 1100;

export interface MockPlay {
  trackId: string;
  at: number;
  ms: number;
  from: { kind: string; id: string | null } | null;
}

/** A small seeded random source, so the demo history is the same every run. */
function rng(seed: number) {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

/**
 * How likely a day `back` days ago (in `year`, this year being `thisYear`)
 * had listening: a busy last month and spell this year, a busy last year,
 * quieter before that.
 */
function activeChance(back: number, year: number, thisYear: number): number {
  if (back <= 28) return 0.82;
  if (back <= 120) return 0.4;
  if (back >= 170 && back <= 205) return 0.75;
  if (year === thisYear) return 0.28;
  if (year === thisYear - 1) return 0.7;
  return 0.3;
}

/** The days back with listening for the sparse demo: a few scattered, most in the last four weeks. */
const SPARSE_DAYS = new Set([340, 290, 212, 150, 61, 27, 25, 24, 22, 20, 19, 17, 15, 14, 12, 11, 9, 8, 7, 6, 5, 3, 2, 1]);

/**
 * About three years of listening, uneven like a real library: quiet
 * stretches and busy ones, favourite artists, mostly late at night, more
 * midweek. `sparse` keeps only 24 days of the last year.
 */
export function seedStatsHistory(fx: Fixtures, now = Date.now(), sparse = false): MockPlay[] {
  const rand = rng(20261015);
  const out: MockPlay[] = [];
  const artists = fx.artists.filter((a) => fx.tracks.some((t) => t.artistId === a.id));
  if (!artists.length) return out;
  const byArtist = new Map(artists.map((a) => [a.id, fx.tracks.filter((t) => t.artistId === a.id)]));
  const today = parseDay(localDay(new Date(now)));
  for (let back = HISTORY_DAYS; back >= 1; back--) {
    const r = rand();
    const day = new Date(today);
    day.setDate(day.getDate() - back);
    if (sparse ? !SPARSE_DAYS.has(back) : r >= activeChance(back, day.getFullYear(), today.getFullYear())) continue;
    const midweek = day.getDay() === 3 ? 1.6 : day.getDay() === 5 ? 1.25 : 1;
    const n = Math.round((3 + Math.floor(rand() * 16)) * midweek);
    // Late at night mostly, evenings often, daytime now and then.
    const pick = rand();
    const startHour = pick < 0.55 ? 22 + Math.floor(rand() * 2) : pick < 0.82 ? 19 + Math.floor(rand() * 3) : 9 + Math.floor(rand() * 6);
    let at = day.getTime() + startHour * 3_600_000 + Math.floor(rand() * 40) * 60_000;
    // A favourite for the day: the first few artists come up most.
    const fav = artists[Math.floor(Math.pow(rand(), 2.2) * artists.length)];
    for (let i = 0; i < n; i++) {
      const artist = rand() < 0.55 ? fav : artists[Math.floor(Math.pow(rand(), 1.6) * artists.length)];
      const list = byArtist.get(artist.id) ?? [];
      const t = list[Math.floor(Math.pow(rand(), 1.8) * list.length)];
      if (!t) continue;
      const skip = rand() < 0.08;
      const ms = skip ? 12_000 : Math.min(t.durationMs || 3 * MIN, 7 * MIN);
      out.push({ trackId: t.id, at, ms: skip ? -1 : ms, from: { kind: "album", id: t.albumId } });
      at += ms + 2_000;
    }
  }
  return out.filter((p) => p.ms > 0 && p.at < now);
}

const dayNum = (ms: number) => {
  const d = new Date(ms);
  return Math.round(Date.UTC(d.getFullYear(), d.getMonth(), d.getDate()) / DAY);
};
const dayNumOf = (day: string) => {
  const [y, m, d] = day.split("-").map(Number);
  return Math.round(Date.UTC(y, m - 1, d) / DAY);
};
const dayStr = (n: number) => {
  const d = new Date(n * DAY);
  return `${d.getUTCFullYear()}-${String(d.getUTCMonth() + 1).padStart(2, "0")}-${String(d.getUTCDate()).padStart(2, "0")}`;
};

interface Tally {
  plays: number;
  ms: number;
}

function bump<K>(m: Map<K, Tally>, k: K, ms: number) {
  const t = m.get(k) ?? { plays: 0, ms: 0 };
  t.plays++;
  t.ms += ms;
  m.set(k, t);
}

function ranked<K>(m: Map<K, Tally>): [K, Tally][] {
  return [...m.entries()].sort((a, b) => b[1].plays - a[1].plays || b[1].ms - a[1].ms || String(a[0]).localeCompare(String(b[0])));
}

export class MockStats {
  constructor(
    private fx: Fixtures,
    private seeded: MockPlay[],
    private session: () => MockListen[],
    private settings: () => Settings,
  ) {}

  private byId: Map<string, Track> | null = null;

  private track(id: string): Track | undefined {
    this.byId ??= new Map(this.fx.tracks.map((t) => [t.id, t]));
    return this.byId.get(id);
  }

  private unreleased(t: Track): boolean {
    const a = this.fx.albums.find((x) => x.id === t.albumId);
    return !!a && !mockMatched(a);
  }

  /** Every counted play, oldest first. */
  plays(): MockPlay[] {
    const live = this.session().map((l): MockPlay => ({ trackId: l.trackId, at: l.at, ms: Math.max(0, l.end - l.at), from: l.id ? { kind: l.kind, id: l.id } : { kind: l.kind === "songs" ? "tracks" : l.kind, id: null } }));
    return [...this.seeded, ...live].sort((a, b) => a.at - b.at);
  }

  private inRange(r: StatsRange): { lo: number; hi: number; plays: MockPlay[] } {
    const all = this.plays();
    const today = dayNum(Date.now());
    const lo = r.from ? dayNumOf(r.from) : all.length ? dayNum(all[0].at) : today;
    const hi = r.to ? dayNumOf(r.to) : Math.max(today, all.length ? dayNum(all[all.length - 1].at) : today);
    if (hi < lo) throw { kind: "invalid", message: "The range ends before it starts." };
    return { lo, hi, plays: all.filter((p) => dayNum(p.at) >= lo && dayNum(p.at) <= hi) };
  }

  private song(id: string, t: Tally): TopSong {
    const tr = this.track(id)!;
    return { key: `k:${id}`, trackId: id, title: tr.title, artist: tr.trackArtist ?? tr.artistTitle, album: tr.albumTitle, plays: t.plays, ms: t.ms, unreleased: this.unreleased(tr), track: tr };
  }

  private artist(id: string, t: Tally): TopArtist {
    const a = this.fx.artists.find((x) => x.id === id)!;
    return { key: a.title.toLowerCase(), artistId: id, name: a.title, thumb: a.thumb, plays: t.plays, ms: t.ms };
  }

  private album(id: string, t: Tally): TopAlbum {
    const a = this.fx.albums.find((x) => x.id === id)!;
    return { key: `${a.artistTitle}|${a.title}`.toLowerCase(), albumId: id, title: a.title, artist: a.artistTitle, thumb: a.thumb, plays: t.plays, ms: t.ms, unreleased: !mockMatched(a) };
  }

  private genres(t: Track): string[] {
    return t.genres?.length ? t.genres : (this.fx.albums.find((a) => a.id === t.albumId)?.genres ?? []);
  }

  private tops(plays: MockPlay[]) {
    const songs = new Map<string, Tally>();
    const artists = new Map<string, Tally>();
    const albums = new Map<string, Tally>();
    const genres = new Map<string, Tally>();
    for (const p of plays) {
      const t = this.track(p.trackId);
      if (!t) continue;
      bump(songs, t.id, p.ms);
      bump(artists, t.artistId, p.ms);
      bump(albums, t.albumId, p.ms);
      for (const g of this.genres(t)) bump(genres, g, p.ms);
    }
    return { songs: ranked(songs), artists: ranked(artists), albums: ranked(albums), genres: ranked(genres) };
  }

  private totals(plays: MockPlay[]): StatsTotals {
    const days = new Set(plays.map((p) => dayNum(p.at)));
    const tr = plays.map((p) => this.track(p.trackId)).filter((t): t is Track => !!t);
    return {
      ms: plays.reduce((s, p) => s + p.ms, 0),
      plays: plays.length,
      songs: new Set(tr.map((t) => t.id)).size,
      artists: new Set(tr.map((t) => t.artistId)).size,
      albums: new Set(tr.map((t) => t.albumId)).size,
      days: days.size,
    };
  }

  private series(plays: MockPlay[], lo: number, hi: number): { bucket: "day" | "month"; points: StatsPoint[] } {
    if (hi - lo < DAILY_SERIES_MAX_DAYS) {
      const by = new Map<number, Tally>();
      for (const p of plays) bump(by, dayNum(p.at), p.ms);
      const points: StatsPoint[] = [];
      for (let d = lo; d <= hi; d++) points.push({ key: dayStr(d), plays: by.get(d)?.plays ?? 0, ms: by.get(d)?.ms ?? 0 });
      return { bucket: "day", points };
    }
    const by = new Map<string, Tally>();
    for (const p of plays) bump(by, localDay(new Date(p.at)).slice(0, 7), p.ms);
    const points: StatsPoint[] = [];
    let [y, m] = dayStr(lo).split("-").map(Number);
    const [ey, em] = dayStr(hi).split("-").map(Number);
    while (y < ey || (y === ey && m <= em)) {
      const key = `${y}-${String(m).padStart(2, "0")}`;
      points.push({ key, plays: by.get(key)?.plays ?? 0, ms: by.get(key)?.ms ?? 0 });
      [y, m] = m === 12 ? [y + 1, 1] : [y, m + 1];
    }
    return { bucket: "month", points };
  }

  overview(r: StatsRange, limit = 50): StatsOverview {
    const { lo, hi, plays } = this.inRange(r);
    const t = this.tops(plays);
    const days = [...new Set(plays.map((p) => dayNum(p.at)))].sort((a, b) => a - b);
    let best: [number, number] | null = null;
    let start = 0;
    days.forEach((d, i) => {
      if (i === 0 || d !== days[i - 1] + 1) start = d;
      if (!best || d - start >= best[1] - best[0]) best = [start, d];
    });
    const hours = Array.from({ length: 24 }, () => 0);
    const weekdays = Array.from({ length: 7 }, () => 0);
    for (const p of plays) {
      const d = new Date(p.at);
      hours[d.getHours()] += p.ms;
      weekdays[d.getDay()] += p.ms;
    }
    const busiest = (v: number[]) => (v.some((x) => x > 0) ? v.indexOf(Math.max(...v)) : null);
    const s = this.series(plays, lo, hi);
    const streak = best as [number, number] | null;
    return {
      from: dayStr(lo),
      to: dayStr(hi),
      totals: this.totals(plays),
      bucket: s.bucket,
      series: s.points,
      streak: streak ? { days: streak[1] - streak[0] + 1, from: dayStr(streak[0]), to: dayStr(streak[1]) } : null,
      hours,
      weekdays,
      busiestHour: busiest(hours),
      busiestWeekday: busiest(weekdays),
      topSongs: t.songs.slice(0, limit).map(([id, x]) => this.song(id, x)),
      topArtists: t.artists.slice(0, limit).map(([id, x]) => this.artist(id, x)),
      topAlbums: t.albums.slice(0, limit).map(([id, x]) => this.album(id, x)),
      topGenres: t.genres.slice(0, Math.min(limit, 20)).map(([name, x]): TopGenre => ({ name, plays: x.plays, ms: x.ms, thumb: this.genreCover(name, t.albums) })),
    };
  }

  span(): StatsSpan {
    const all = this.plays();
    const months = [...new Set(all.map((p) => localDay(new Date(p.at)).slice(0, 7)))].sort().reverse();
    return {
      first: all.length ? localDay(new Date(all[0].at)) : null,
      last: all.length ? localDay(new Date(all[all.length - 1].at)) : null,
      months,
      today: localDay(new Date()),
    };
  }

  item(kind: StatsItemKind, id: string): ItemStats {
    const all = this.plays();
    const match = (t: Track) => (kind === "song" ? t.id === id : kind === "artist" ? t.artistId === id : t.albumId === id);
    const mine = all.filter((p) => {
      const t = this.track(p.trackId);
      return !!t && match(t);
    });
    const today = localDay(new Date());
    const months: string[] = [];
    for (let i = 11; i >= 0; i--) {
      const [y, m] = today.split("-").map(Number);
      const d = new Date(y, m - 1 - i, 1);
      months.push(`${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}`);
    }
    const series = months.map((key) => {
      const ps = mine.filter((p) => localDay(new Date(p.at)).startsWith(key));
      return { key, plays: ps.length, ms: ps.reduce((s, p) => s + p.ms, 0) };
    });
    if (!mine.length) return { kind, plays: 0, ms: 0, first: null, lastAt: null, rank: null, series };
    const t = this.tops(all);
    const list = kind === "song" ? t.songs : kind === "artist" ? t.artists : t.albums;
    const rank = list.findIndex(([k]) => k === id) + 1;
    const first = mine[0];
    const ft = this.track(first.trackId)!;
    const fromTitle = first.from?.kind === "album" ? (this.fx.albums.find((a) => a.id === first.from?.id)?.title ?? null) : first.from?.kind === "playlist" ? (this.fx.playlists.find((p) => p.id === first.from?.id)?.title ?? null) : null;
    return {
      kind,
      plays: mine.length,
      ms: mine.reduce((s, p) => s + p.ms, 0),
      first: { at: first.at, song: ft.title, trackId: ft.id, from: first.from ? { kind: first.from.kind, id: first.from.id, title: fromTitle } : null },
      lastAt: mine[mine.length - 1].at,
      rank: rank || null,
      series,
    };
  }

  monthRecap(month: string): MonthRecap {
    const b = monthBounds(month);
    const o = this.overview({ from: b.from, to: b.to }, 5);
    const prevFrom = addDays(b.from, -1).slice(0, 7);
    const pb = monthBounds(prevFrom);
    const prev = this.inRange({ from: pb.from, to: pb.to }).plays;
    const biggest = o.series.filter((p) => p.ms > 0).sort((a, z) => z.ms - a.ms)[0] ?? null;
    return {
      month,
      totals: o.totals,
      prevMs: prev.reduce((s, p) => s + p.ms, 0),
      topSongs: o.topSongs,
      topArtists: o.topArtists,
      topAlbum: o.topAlbums[0] ?? null,
      topAlbums: o.topAlbums.slice(0, 4),
      topGenre: o.topGenres[0]?.name ?? null,
      daily: o.series,
      biggestDay: biggest,
    };
  }

  /** The cover of the most played album with a song in the genre. */
  private genreCover(genre: string, albums: [string, Tally][]): string | null {
    for (const [id] of albums.slice(0, 200)) {
      const a = this.fx.albums.find((x) => x.id === id);
      if (a?.thumb && this.fx.tracks.some((t) => t.albumId === id && this.genres(t).includes(genre))) return a.thumb;
    }
    return null;
  }

  days(from: string, to: string): StatsPoint[] {
    const lo = dayNumOf(from);
    const hi = dayNumOf(to);
    if (hi < lo || hi - lo > 400) throw { kind: "invalid", message: "That calendar range is too long." };
    const by = new Map<number, Tally>();
    for (const p of this.plays()) {
      const d = dayNum(p.at);
      if (d >= lo && d <= hi) bump(by, d, p.ms);
    }
    return [...by.entries()].sort((a, b) => a[0] - b[0]).map(([d, t]) => ({ key: dayStr(d), plays: t.plays, ms: t.ms }));
  }

  /** The core's moments rules over the demo history. */
  moments(r: StatsRange): StatsMoments {
    const all = r.from == null && r.to == null;
    const { lo, hi, plays } = this.inRange(r);
    const every = this.plays();
    const first = plays[0];
    // Per song and day.
    const songDay = new Map<string, Tally & { day: number; id: string }>();
    for (const p of plays) {
      const d = dayNum(p.at);
      const k = `${p.trackId}|${d}`;
      const t = songDay.get(k) ?? { plays: 0, ms: 0, day: d, id: p.trackId };
      t.plays++;
      t.ms += p.ms;
      songDay.set(k, t);
    }
    const rep = [...songDay.values()].filter((x) => x.plays >= 3).sort((a, b) => b.plays - a.plays || b.ms - a.ms || b.day - a.day)[0];
    let discovery: StatsMoments["discovery"] = null;
    let newArtists: number | null = null;
    if (!all) {
      const before = new Set(every.filter((p) => dayNum(p.at) < lo).map((p) => this.track(p.trackId)?.artistId));
      const fresh = new Map<string, Tally & { day: number }>();
      for (const p of plays) {
        const a = this.track(p.trackId)?.artistId;
        if (!a || before.has(a)) continue;
        const t = fresh.get(a) ?? { plays: 0, ms: 0, day: dayNum(p.at) };
        t.plays++;
        t.ms += p.ms;
        fresh.set(a, t);
      }
      newArtists = fresh.size;
      const top = [...fresh.entries()].sort((a, b) => b[1].plays - a[1].plays || b[1].ms - a[1].ms)[0];
      if (top) discovery = { artist: this.artist(top[0], top[1]), day: dayStr(top[1].day) };
    }
    // Comebacks: the longest break before a day in the range.
    const daysBySong = new Map<string, number[]>();
    for (const p of every) {
      const d = dayNum(p.at);
      if (d > hi) continue;
      const list = daysBySong.get(p.trackId) ?? [];
      if (list[list.length - 1] !== d) list.push(d);
      daysBySong.set(p.trackId, list);
    }
    let back: { id: string; day: number; gap: number } | null = null;
    for (const [id, ds] of daysBySong) {
      for (let i = 1; i < ds.length; i++) {
        const gap = ds[i] - ds[i - 1];
        if (ds[i] >= lo && gap >= 30 && (!back || gap > back.gap || (gap === back.gap && ds[i] > back.day))) back = { id, day: ds[i], gap };
      }
    }
    const songOn = (id: string, d: number) => {
      const t = songDay.get(`${id}|${d}`) ?? { plays: 1, ms: 0 };
      return this.song(id, t);
    };
    return {
      firstSong: first ? { song: songOn(first.trackId, dayNum(first.at)), day: dayStr(dayNum(first.at)), at: first.at } : null,
      replayed: rep ? { song: songOn(rep.id, rep.day), day: dayStr(rep.day), at: null } : null,
      discovery,
      comeback: back ? { song: { song: songOn(back.id, back.day), day: dayStr(back.day), at: null }, gapDays: back.gap } : null,
      newArtists,
    };
  }

  playIds(r: StatsRange, kind: StatsTopKind, limit = 50): string[] {
    const { plays } = this.inRange(r);
    const t = this.tops(plays);
    const songs = t.songs.map(([id]) => this.track(id)!);
    const pick =
      kind === "songs"
        ? songs
        : kind === "artists"
          ? songs.filter((s) => t.artists.slice(0, 10).some(([a]) => a === s.artistId))
          : kind === "albums"
            ? songs.filter((s) => t.albums.slice(0, 10).some(([a]) => a === s.albumId))
            : songs.filter((s) => this.genres(s).some((g) => t.genres.slice(0, 5).some(([x]) => x === g)));
    return pick.slice(0, limit).map((s) => s.id);
  }

  private requireWrapped() {
    if (this.settings().wrappedTopSongs === false) throw { kind: "invalid", message: "Wrapped and Top Songs are off in Settings." };
  }

  /** Plays per local year, oldest first. */
  private years(): Map<number, number> {
    const out = new Map<number, number>();
    for (const p of this.plays()) {
      const y = new Date(p.at).getFullYear();
      out.set(y, (out.get(y) ?? 0) + 1);
    }
    return out;
  }

  /** The core's rules: lists from 30 plays, this year from December 1, Home from December to January 15. */
  topSongsIndex(today = localDay(new Date())): TopSongsIndex {
    this.requireWrapped();
    const ys = [...this.years().entries()].sort((a, b) => b[0] - a[0]);
    const [ty, tm, td] = today.split("-").map(Number);
    const open = (y: number) => y < ty || (y === ty && tm === 12);
    const years = ys.filter(([y, n]) => n >= TOP_SONGS_MIN_PLAYS && open(y)).map(([y]) => y);
    const promoted = tm === 12 ? ty : tm === 1 && td <= 15 ? ty - 1 : null;
    return {
      years,
      allTime: ys.length > 0,
      promoted: promoted != null && years.includes(promoted) ? promoted : null,
      wrappedYears: ys.filter(([y, n]) => n > 0 && open(y)).map(([y]) => y),
      currentYear: ty,
    };
  }

  topSongsList(year: number | null, today = localDay(new Date())): TopSongsList {
    const ix = this.topSongsIndex(today);
    if (year != null && !ix.years.includes(year)) throw { kind: "notFound", message: `Top Songs ${year}` };
    const r: StatsRange = year == null ? { from: null, to: null } : { from: `${year}-01-01`, to: `${year}-12-31` };
    const { plays } = this.inRange(r);
    const first = year == null ? null : plays[0]?.trackId;
    const rows = this.tops(plays)
      .songs.slice(0, 100)
      .map(([id, t]): TopSongsRow => {
        const tr = this.track(id)!;
        return { track: tr, plays: t.plays, ms: t.ms, unreleased: this.unreleased(tr), firstOfYear: id === first };
      });
    return { year, title: year == null ? "All-Time Top Songs" : `Your Top Songs ${year}`, rows, durationMs: rows.reduce((n, r) => n + r.track.durationMs, 0) };
  }

  wrapped(year: number): Wrapped {
    this.requireWrapped();
    const r = { from: `${year}-01-01`, to: `${year}-12-31` };
    const o = this.overview(r, 5);
    const m = this.moments(r);
    const { plays } = this.inRange(r);
    const prev = this.inRange({ from: `${year - 1}-01-01`, to: `${year - 1}-12-31` }).plays;
    const before = new Set(this.plays().filter((p) => p.at < parseDay(r.from).getTime()).map((p) => p.trackId));
    const perSong = new Map<string, number>();
    for (const p of plays) perSong.set(p.trackId, (perSong.get(p.trackId) ?? 0) + 1);
    const repeatPlays = [...perSong.values()].filter((n) => n >= 5).reduce((a, n) => a + n, 0);
    const newPlays = plays.filter((p) => !before.has(p.trackId)).length;
    const biggest = this.days(r.from, r.to).sort((a, z) => z.ms - a.ms || a.key.localeCompare(z.key))[0] ?? null;
    return {
      year,
      totals: o.totals,
      prevMs: prev.reduce((n, p) => n + p.ms, 0),
      topArtists: o.topArtists,
      topSongs: o.topSongs,
      topAlbums: o.topAlbums,
      topGenres: o.topGenres.slice(0, 5),
      personality: mockPersonality(o.hours, plays.length, repeatPlays, newPlays),
      hours: o.hours,
      biggestDay: biggest,
      replayed: m.replayed,
      firstSong: m.firstSong,
      newArtists: m.newArtists ?? 0,
    };
  }

  private requireCompare() {
    if (!this.settings().statsCompare) throw { kind: "invalid", message: "Comparing stats with friends is off in Settings." };
  }

  compareSummary(unreleasedOk: boolean, theirOk: boolean): CompareSummary {
    this.requireCompare();
    const from = addDays(localDay(new Date()), -181);
    const with_ = unreleasedOk && theirOk;
    const plays = this.inRange({ from, to: null }).plays.filter((p) => {
      const t = this.track(p.trackId);
      return !!t && (with_ || !this.unreleased(t));
    });
    const t = this.tops(plays);
    const total = plays.length || 1;
    const w = (n: number) => Math.round((n / total) * 1000) / 1000;
    return {
      version: 1,
      name: this.settings().listenName || null,
      artists: t.artists.slice(0, 50).map(([id, x]) => ({ name: this.fx.artists.find((a) => a.id === id)!.title, weight: w(x.plays) })),
      songs: t.songs.slice(0, 50).map(([id, x]) => {
        const tr = this.track(id)!;
        return { artist: tr.trackArtist ?? tr.artistTitle, title: tr.title, weight: w(x.plays) };
      }),
      unreleasedOk,
    };
  }

  compare(theirs: CompareSummary, unreleasedOk: boolean): CompareResult {
    this.requireCompare();
    if (theirs.version !== 1) throw { kind: "invalid", message: "That stats summary is from a different version of Platter." };
    const mine = this.compareSummary(unreleasedOk, theirs.unreleasedOk);
    const norm = (list: { name: string; weight: number }[]) => {
      const total = list.reduce((s, a) => s + a.weight, 0) || 1;
      return new Map(list.map((a) => [a.name.toLowerCase().replace(/^the /, ""), { w: a.weight / total, name: a.name }]));
    };
    const a = norm(mine.artists);
    const b = norm(theirs.artists);
    let shared = 0;
    const common = [];
    for (const [k, v] of a) {
      const o = b.get(k);
      if (!o) continue;
      shared += Math.min(v.w, o.w);
      const lib = this.fx.artists.find((x) => x.title.toLowerCase() === v.name.toLowerCase());
      common.push({ name: v.name, artistId: lib?.id ?? null, thumb: lib?.thumb ?? null, mine: v.w, theirs: o.w });
    }
    common.sort((x, y) => Math.min(y.mine, y.theirs) - Math.min(x.mine, x.theirs));
    const played = new Set(this.plays().map((p) => p.trackId));
    const newSongs = theirs.songs
      .map((s) => ({ s, t: this.fx.tracks.find((t) => t.title.toLowerCase() === s.title.toLowerCase() && (t.trackArtist ?? t.artistTitle).toLowerCase() === s.artist.toLowerCase()) ?? null }))
      .filter(({ t }) => !t || !played.has(t.id))
      .filter(({ t }) => !t || (unreleasedOk && theirs.unreleasedOk) || !this.unreleased(t))
      .slice(0, 20)
      .map(({ s, t }) => ({ artist: s.artist, title: s.title, track: t }));
    return { friend: theirs.name, overlap: Math.round(Math.min(1, shared) * 100), commonArtists: common.slice(0, 20), theirSongsNew: newSongs };
  }
}

export const TOP_SONGS_MIN_PLAYS = 30;

/** The core's personality rules (stats::wrapped::personality). */
export function mockPersonality(hours: number[], plays: number, repeatPlays: number, newPlays: number): Personality | null {
  const total = hours.reduce((a, n) => a + n, 0);
  if (total <= 0 || plays <= 0) return null;
  const part = (h: number): TimeOfDay => (h >= 5 && h <= 11 ? "morning" : h >= 12 && h <= 16 ? "afternoon" : h >= 17 && h <= 21 ? "evening" : "night");
  const order: TimeOfDay[] = ["morning", "afternoon", "evening", "night"];
  const parts = [0, 0, 0, 0];
  hours.forEach((ms, h) => (parts[order.indexOf(part(h))] += ms));
  let best = 0;
  parts.forEach((ms, i) => {
    if (ms > parts[best]) best = i;
  });
  const share = (n: number) => Math.min(Math.max(n, 0), plays) / plays;
  return {
    time: order[best],
    timeShare: parts[best] / total,
    repeatShare: share(repeatPlays),
    repeats: share(repeatPlays) >= 0.5,
    newShare: share(newPlays),
    mostlyNew: share(newPlays) >= 0.5,
  };
}

/** A friend's summary for the Compare view's demo: some artists in common, some songs you've never played. */
export function mockFriendSummary(fx: Fixtures, played: Set<string>): CompareSummary {
  const artists = fx.artists.filter((a) => fx.tracks.some((t) => t.artistId === a.id));
  const pick = [artists[0], artists[2], artists[5], artists[7]].filter(Boolean);
  const unplayed = fx.tracks.filter((t) => !played.has(t.id) && fx.albums.some((a) => a.id === t.albumId && mockMatched(a))).slice(0, 4);
  return {
    version: 1,
    name: "Sam",
    artists: [
      ...pick.map((a, i) => ({ name: a.title, weight: [0.3, 0.2, 0.15, 0.1][i] ?? 0.05 })),
      { name: "The Lanternfish", weight: 0.15 },
      { name: "Paper Satellites", weight: 0.1 },
    ],
    songs: [
      ...unplayed.map((t) => ({ artist: t.trackArtist ?? t.artistTitle, title: t.title, weight: 0.05 })),
      { artist: "The Lanternfish", title: "Low Tide Radio", weight: 0.08 },
      { artist: "Paper Satellites", title: "Orbit for Two", weight: 0.06 },
    ],
    unreleasedOk: false,
  };
}
