/**
 * Mock library import: what an import from each source brings in, matched
 * to the fixture library, with a few songs unsure and a few missing. The
 * same rules as the core in miniature: songs once per source, review
 * decisions kept, reports counted from what's stored.
 */
import type {
  ImportDecision,
  ImportHistory,
  PlayedHidden,
  PlayedPage,
  PlayedQuery,
  PlayedRow,
  ImportOverview,
  ImportReport,
  ImportReviewItem,
  ImportSkipped,
  ImportSourceId,
  ImportedList,
  ImportedListPage,
  MissingSong,
} from "../importTypes";
import type { Track } from "../types";
import type { Fixtures } from "./fixtures";

interface Song {
  songId: number;
  source: ImportSourceId;
  artist: string;
  title: string;
  album: string | null;
  durationMs: number | null;
  status: "matched" | "unsure" | "missing";
  trackId: string | null;
  candidates: string[];
  reviewed: boolean;
}

interface List {
  id: string;
  source: ImportSourceId;
  name: string;
  liked: boolean;
  songIds: number[];
}

interface Run {
  importedAt: number;
  fileName: string | null;
  albums: { total: number; matched: number };
  plays: { total: number; matched: number };
  playsAdded: number;
  history: ImportHistory;
  skipped: ImportSkipped;
}

const NO_HISTORY: ImportHistory = { plays: 0, playsMatched: 0, songs: 0, songsMatched: 0, firstPlayed: null, lastPlayed: null };
const DAY = 86_400_000;

/**
 * Songs from the made-up listening history that aren't in the fixture
 * library: plays over all time, the last year and the last 90 days, the
 * average play length, and days since the first and last play.
 */
const PLAYED: { artist: string; title: string; album: string; plays: [number, number, number]; avgMs: number; first: number; last: number }[] = [
  { artist: "Harbor Lights", title: "Paper Boats", album: "Low Water", plays: [48, 12, 4], avgMs: 214_000, first: 2100, last: 12 },
  { artist: "The Long Weekends", title: "Neon Driveway", album: "Night Errands", plays: [31, 2, 0], avgMs: 188_000, first: 1900, last: 200 },
  { artist: "Cass Morrow", title: "Thin Ice Waltz", album: "Winter Rooms", plays: [22, 22, 9], avgMs: 241_000, first: 300, last: 3 },
  { artist: "Harbor Lights", title: "Undertow", album: "Low Water", plays: [17, 5, 1], avgMs: 199_000, first: 1500, last: 40 },
  { artist: "Mira Vale", title: "Glass Orchard", album: "Second Bloom", plays: [9, 0, 0], avgMs: 176_000, first: 1700, last: 900 },
  { artist: "Osa Lind", title: "Field Notes", album: "Field Notes", plays: [6, 6, 6], avgMs: 305_000, first: 60, last: 1 },
  { artist: "Paper Boats", title: "Quiet Rooms", album: "Low Water (Deluxe)", plays: [3, 1, 0], avgMs: 150_000, first: 800, last: 100 },
];
/** Songs the fixture library already has from these albums. */
const IN_LIBRARY: Record<string, number> = { "harbor lights\u001flow water": 1 };

const fold = (s: string) => s.normalize("NFKC").trim().replace(/\s+/g, " ").toLowerCase();
export const playedKey = (kind: PlayedHidden["kind"], artist: string, name: string) =>
  kind === "artist" ? `artist:${fold(artist)}` : `${kind}:${fold(artist)}\u001f${fold(name)}`;

const NONE: ImportSkipped = { privatePlays: 0, podcasts: 0, noMetadata: 0, shortPlays: 0, incompleteRows: 0, unknownFiles: 0 };

/** Songs the fixture library doesn't have (invented names). */
const ABSENT: [string, string, string][] = [
  ["Harbor Lights", "Paper Boats", "Low Water"],
  ["Mira Vale", "Glass Orchard", "Second Bloom"],
  ["The Long Weekends", "Neon Driveway", "Night Errands"],
  ["Cass Morrow", "Thin Ice Waltz", "Winter Rooms"],
  ["Paper Boats", "Undertow", "Low Water"],
  ["Osa Lind", "Field Notes", "Field Notes"],
];

function invalid(message: string): never {
  throw { kind: "invalid", message };
}

export class MockImport {
  private songs: Song[] = [];
  private lists: List[] = [];
  private runs = new Map<ImportSourceId, Run>();
  /** History was imported (the made-up plays above are in). */
  private history = false;
  private hidden = new Map<string, PlayedHidden>();
  private seeds = PLAYED.map((s) => ({ ...s }));
  private seq = 1;
  /** Test hook: the next import fails with this message. */
  failNext: string | null = null;
  constructor(
    private fx: Fixtures,
    private enabled: () => string[],
  ) {}

  reset() {
    this.songs = [];
    this.lists = [];
    this.runs.clear();
    this.history = false;
    this.hidden.clear();
    this.seeds = PLAYED.map((s) => ({ ...s }));
    this.seq = 1;
    this.failNext = null;
  }

  private track(id: string): Track | undefined {
    return this.fx.tracks.find((t) => t.id === id);
  }

  /** A song from the library, matched. */
  private matched(source: ImportSourceId, t: Track): number {
    return this.add({ source, artist: t.trackArtist ?? t.artistTitle, title: t.title, album: t.albumTitle, durationMs: t.durationMs, status: "matched", trackId: t.id, candidates: [t.id] });
  }

  /** A song only a person can decide (a live cut, or two albums). */
  private unsure(source: ImportSourceId, t: Track, other: Track): number {
    return this.add({
      source,
      artist: t.artistTitle,
      title: `${t.title} (Live)`,
      album: "Live Sessions",
      durationMs: t.durationMs + 21_000,
      status: "unsure",
      trackId: null,
      candidates: [t.id, other.id],
    });
  }

  private missing(source: ImportSourceId, i: number): number {
    const [artist, title, album] = ABSENT[i % ABSENT.length];
    return this.add({ source, artist, title, album, durationMs: null, status: "missing", trackId: null, candidates: [] });
  }

  private add(s: Omit<Song, "songId" | "reviewed">): number {
    const same = this.songs.find((x) => x.source === s.source && x.artist === s.artist && x.title === s.title && x.album === s.album);
    if (same) {
      if (!same.reviewed) Object.assign(same, { status: s.status, trackId: s.trackId, candidates: s.candidates });
      return same.songId;
    }
    const songId = this.seq++;
    this.songs.push({ ...s, songId, reviewed: false });
    return songId;
  }

  private putList(source: ImportSourceId, name: string, liked: boolean, songIds: number[]) {
    const id = liked ? `liked:${source}` : (this.lists.find((l) => l.source === source && l.name === name)?.id ?? `pl:${this.seq++}`);
    this.lists = this.lists.filter((l) => l.id !== id);
    this.lists.push({ id, source, name, liked, songIds });
  }

  /** Imports "the file" for a source: the same made-up export every time. */
  run(source: ImportSourceId, path: string): ImportReport {
    if (!this.enabled().includes(source)) invalid("Turn this source on in Settings first.");
    if (this.failNext) {
      const m = this.failNext;
      this.failNext = null;
      invalid(m);
    }
    // One of each song (the fixtures repeat some titles on an album).
    const seen = new Set<string>();
    const t = this.fx.tracks.filter((x) => {
      const k = `${x.artistTitle}\u001f${x.title}\u001f${x.albumTitle}`;
      if (seen.has(k)) return false;
      seen.add(k);
      return true;
    });
    const pick = (from: number, n: number) => t.slice(from, from + n);
    const fileName = path.split(/[\\/]/).pop() ?? null;
    const now = Date.now();
    const history: ImportHistory = { plays: 5210, playsMatched: 4870, songs: 1240, songsMatched: 610, firstPlayed: now - 2200 * DAY, lastPlayed: now - DAY };
    if (source === "streaming" && /history|extended/i.test(fileName ?? "")) {
      // A listening history download alone: plays, no playlists or likes.
      if (/empty/i.test(fileName ?? "")) invalid("Those files had no songs Platter could use. Left out: 212 podcast or audiobook records, 640 plays under 30 seconds.");
      const before = this.runs.get(source);
      this.history = true;
      this.runs.set(source, {
        importedAt: now,
        fileName,
        albums: before?.albums ?? { total: 0, matched: 0 },
        plays: { total: history.plays, matched: history.playsMatched },
        playsAdded: before?.history.plays ? 0 : history.plays,
        history,
        skipped: { ...NONE, podcasts: 212, shortPlays: 640, privatePlays: 37 },
      });
      return this.report(source)!;
    }
    if (source === "streaming") {
      const found = pick(0, 10).map((x) => this.matched(source, x));
      const road = [...found.slice(0, 3), this.missing(source, 0), ...found.slice(3, 7), this.missing(source, 1), ...found.slice(7)];
      const rainy = [...pick(12, 6).map((x) => this.matched(source, x)), this.unsure(source, t[20], t[21]), this.missing(source, 0)];
      const liked = [...pick(30, 17).map((x) => this.matched(source, x)), this.unsure(source, t[50], t[51]), this.missing(source, 2), this.missing(source, 0)];
      this.putList(source, "Road Trip", false, road);
      this.putList(source, "Rainy Day", false, rainy);
      this.putList(source, "Liked songs", true, liked);
      const before = this.runs.get(source);
      this.runs.set(source, {
        importedAt: Date.now(),
        fileName,
        albums: { total: 14, matched: 11 },
        plays: { total: 5210, matched: 4870 },
        playsAdded: before ? 0 : 5210,
        history,
        skipped: { ...NONE, privatePlays: 37, podcasts: 212, noMetadata: 9, shortPlays: 640, unknownFiles: 4 },
      });
    } else if (source === "transfer") {
      const workout = [...pick(60, 8).map((x) => this.matched(source, x)), this.missing(source, 3), this.missing(source, 4)];
      this.putList(source, "Workout", false, workout);
      this.runs.set(source, { importedAt: Date.now(), fileName, albums: { total: 0, matched: 0 }, plays: { total: 0, matched: 0 }, playsAdded: 0, history: NO_HISTORY, skipped: { ...NONE, incompleteRows: 2 } });
    } else {
      const list = [...pick(80, 4).map((x) => this.matched(source, x)), this.missing(source, 5)];
      this.putList(source, (fileName ?? "List").replace(/\.[^.]+$/, ""), false, list);
      this.runs.set(source, { importedAt: Date.now(), fileName, albums: { total: 0, matched: 0 }, plays: { total: 0, matched: 0 }, playsAdded: 0, history: NO_HISTORY, skipped: NONE });
    }
    // Songs nothing wants any more.
    const wanted = new Set(this.lists.flatMap((l) => l.songIds));
    this.songs = this.songs.filter((s) => wanted.has(s.songId));
    return this.report(source)!;
  }

  private report(source: ImportSourceId): ImportReport | null {
    const run = this.runs.get(source);
    if (!run) return null;
    if (source === "streaming" && run.history.plays) this.history = true;
    const own = this.songs.filter((s) => s.source === source);
    const liked = this.lists.find((l) => l.id === `liked:${source}`);
    const likedSongs = (liked?.songIds ?? []).map((id) => this.songs.find((s) => s.songId === id)!);
    return {
      source,
      importedAt: run.importedAt,
      fileName: run.fileName,
      songs: {
        total: own.length,
        matched: own.filter((s) => s.status === "matched").length,
        unsure: own.filter((s) => s.status === "unsure").length,
        missing: own.filter((s) => s.status === "missing").length,
      },
      liked: { total: likedSongs.length, matched: likedSongs.filter((s) => s.status === "matched").length },
      playlists: this.lists.filter((l) => l.source === source && !l.liked).length,
      albums: run.albums,
      plays: run.plays,
      playsAdded: run.playsAdded,
      history: run.history,
      skipped: run.skipped,
    };
  }

  private summary(l: List): ImportedList {
    const songs = l.songIds.map((id) => this.songs.find((s) => s.songId === id)!);
    return { id: l.id, source: l.source, name: l.name, liked: l.liked, total: songs.length, matched: songs.filter((s) => s.status === "matched").length };
  }

  overview(): ImportOverview {
    const order: ImportSourceId[] = ["lists", "streaming", "transfer"];
    return {
      reports: order.map((s) => this.report(s)).filter((r): r is ImportReport => !!r),
      lists: [...this.lists]
        .sort((a, b) => Number(b.liked) - Number(a.liked) || a.name.localeCompare(b.name))
        .map((l) => this.summary(l)),
      reviewCount: this.songs.filter((s) => s.status === "unsure" && !s.reviewed).length,
      missingCount: this.songs.filter((s) => s.status === "missing").length,
    };
  }

  reviewQueue(): ImportReviewItem[] {
    return this.songs
      .filter((s) => s.status === "unsure" && !s.reviewed)
      .map((s) => ({
        songId: s.songId,
        source: s.source,
        artist: s.artist,
        title: s.title,
        album: s.album,
        durationMs: s.durationMs,
        candidates: s.candidates.map((id) => this.track(id)).filter((x): x is Track => !!x),
      }));
  }

  review(songId: number, d: ImportDecision) {
    const s = this.songs.find((x) => x.songId === songId);
    if (!s) throw { kind: "notFound", message: `import song ${songId}` };
    const pick = d.kind === "accept" ? s.candidates[0] : d.kind === "pick" ? d.trackId : null;
    if (d.kind === "accept" && !pick) invalid("There's nothing to accept.");
    if (pick && !this.track(pick)) throw { kind: "notFound", message: `track ${pick}` };
    Object.assign(s, { status: pick ? "matched" : "missing", trackId: pick, reviewed: true });
  }

  listPage(id: string): ImportedListPage {
    const l = this.lists.find((x) => x.id === id);
    if (!l) throw { kind: "notFound", message: `imported list ${id}` };
    return {
      list: this.summary(l),
      rows: l.songIds.map((sid) => {
        const s = this.songs.find((x) => x.songId === sid)!;
        const track = s.status === "matched" && s.trackId ? (this.track(s.trackId) ?? null) : null;
        return { songId: s.songId, artist: s.artist, title: s.title, album: s.album, durationMs: s.durationMs, status: s.status, track };
      }),
    };
  }

  /** Takes a song out of one list; a song nothing wants is forgotten. */
  removeFromList(listId: string, songId: number) {
    const l = this.lists.find((x) => x.id === listId);
    if (!l || !l.songIds.includes(songId)) throw { kind: "notFound", message: `song ${songId} in imported list ${listId}` };
    l.songIds = l.songIds.filter((id) => id !== songId);
    if (!this.lists.some((x) => x.songIds.includes(songId))) this.songs = this.songs.filter((s) => s.songId !== songId);
  }

  missingSongs(): MissingSong[] {
    const groups = new Map<string, { s: Song; wants: Set<string>; names: Set<string>; sources: Set<ImportSourceId> }>();
    for (const l of this.lists) {
      for (const sid of l.songIds) {
        const s = this.songs.find((x) => x.songId === sid)!;
        if (s.status !== "missing") continue;
        const key = `${s.artist.toLowerCase()}\u001f${s.title.toLowerCase()}`;
        const g = groups.get(key) ?? { s, wants: new Set(), names: new Set(), sources: new Set() };
        g.wants.add(l.id);
        g.names.add(l.name);
        g.sources.add(s.source);
        groups.set(key, g);
      }
    }
    const q = (s: Song) => encodeURIComponent(`${s.artist} ${s.title}`);
    return [...groups.values()]
      .map(({ s, wants, names, sources }) => ({
        artist: s.artist,
        title: s.title,
        album: s.album,
        wants: wants.size,
        wantedBy: [...names].sort(),
        sources: [...sources].sort(),
        links: {
          bandcamp: `https://bandcamp.com/search?q=${q(s)}&item_type=t`,
          qobuz: `https://www.qobuz.com/search?q=${q(s)}`,
          itunes: `https://music.apple.com/search?term=${q(s)}`,
        },
      }))
      .sort((a, b) => b.wants - a.wants || a.artist.localeCompare(b.artist) || a.title.localeCompare(b.title));
  }

  /** A page of "Played a lot", as the core ranks it. */
  played(q: PlayedQuery, requestFor: (artist: string, album: string) => PlayedRow["request"]): PlayedPage {
    const p = q.period === "year" ? 1 : q.period === "days90" ? 2 : 0;
    const min = Math.min(1000, Math.max(1, Math.round(q.minPlays)));
    const now = Date.now();
    const artistHidden = (a: string) => this.hidden.has(playedKey("artist", a, ""));
    const seeds = this.history ? this.seeds.filter((s) => s.plays[p] > 0 && !artistHidden(s.artist)) : [];
    const links = (artist: string, name: string, type: "t" | "a") => {
      const e = encodeURIComponent(`${artist} ${name}`);
      return { bandcamp: `https://bandcamp.com/search?q=${e}&item_type=${type}`, qobuz: `https://www.qobuz.com/search?q=${e}`, itunes: `https://music.apple.com/search?term=${e}` };
    };
    let rows: PlayedRow[];
    if (q.kind === "song") {
      rows = seeds.map((s) => ({
        kind: "song",
        key: playedKey("song", s.artist, s.title),
        artist: s.artist,
        title: s.title,
        album: s.album,
        plays: s.plays[p],
        ms: s.plays[p] * s.avgMs,
        firstPlayed: now - s.first * DAY,
        lastPlayed: now - s.last * DAY,
        songs: null,
        inLibrary: null,
        links: links(s.artist, s.title, "t"),
        thumb: null,
        request: requestFor(s.artist, s.album),
      }));
    } else {
      const by = new Map<string, PlayedRow>();
      for (const s of seeds) {
        const k = playedKey("album", s.artist, s.album);
        const r = by.get(k);
        if (r) {
          r.plays += s.plays[p];
          r.ms += s.plays[p] * s.avgMs;
          r.songs! += 1;
          r.firstPlayed = Math.min(r.firstPlayed, now - s.first * DAY);
          r.lastPlayed = Math.max(r.lastPlayed, now - s.last * DAY);
        } else {
          by.set(k, {
            kind: "album",
            key: k,
            artist: s.artist,
            title: s.album,
            album: null,
            plays: s.plays[p],
            ms: s.plays[p] * s.avgMs,
            firstPlayed: now - s.first * DAY,
            lastPlayed: now - s.last * DAY,
            songs: 1,
            inLibrary: IN_LIBRARY[`${fold(s.artist)}\u001f${fold(s.album)}`] ?? 0,
            links: links(s.artist, s.album, "a"),
            thumb: null,
            request: requestFor(s.artist, s.album),
          });
        }
      }
      rows = [...by.values()];
    }
    rows = rows
      .filter((r) => r.plays >= min && !this.hidden.has(r.key))
      .sort((a, b) => b.plays - a.plays || b.ms - a.ms || a.artist.localeCompare(b.artist) || a.title.localeCompare(b.title));
    return { kind: q.kind, total: rows.length, offset: q.offset, rows: rows.slice(q.offset, q.offset + 100), hasHistory: this.history };
  }

  hide(kind: PlayedHidden["kind"], artist: string, name: string | null, hide: boolean): PlayedHidden[] {
    if (!["song", "album", "artist"].includes(kind)) invalid("Only songs, albums and artists can be hidden.");
    const key = playedKey(kind, artist, name ?? "");
    if (hide) this.hidden.set(key, { kind, artist, name: kind === "artist" ? "" : (name ?? ""), hiddenAt: Math.floor(Date.now() / 1000) });
    else this.hidden.delete(key);
    return this.hiddenList();
  }

  hiddenList(): PlayedHidden[] {
    return [...this.hidden.values()].sort((a, b) => b.hiddenAt - a.hiddenAt);
  }

  /** Test hook: the library gained a song, so its row leaves the list. */
  libraryGained(artist: string, title: string) {
    const i = this.seeds.findIndex((s) => fold(s.artist) === fold(artist) && fold(s.title) === fold(title));
    if (i >= 0) this.seeds.splice(i, 1);
  }

  forget(source: ImportSourceId) {
    if (source === "streaming") this.history = false;
    this.lists = this.lists.filter((l) => l.source !== source);
    this.songs = this.songs.filter((s) => s.source !== source);
    this.runs.delete(source);
  }
}

/** Only store search pages open from the Missing list. */
export function importLinkAllowed(url: string): boolean {
  try {
    const u = new URL(url);
    return u.protocol === "https:" && ["bandcamp.com", "www.qobuz.com", "music.apple.com"].includes(u.hostname) && !u.port;
  } catch {
    return false;
  }
}
