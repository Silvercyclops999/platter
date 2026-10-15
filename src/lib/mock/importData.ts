/**
 * Mock library import: what an import from each source brings in, matched
 * to the fixture library, with a few songs unsure and a few missing. The
 * same rules as the core in miniature: songs once per source, review
 * decisions kept, reports counted from what's stored.
 */
import type {
  ImportDecision,
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
  skipped: ImportSkipped;
}

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
    if (source === "streaming") {
      const road = [...pick(0, 10).map((x) => this.matched(source, x)), this.missing(source, 0), this.missing(source, 1)];
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
        skipped: { ...NONE, privatePlays: 37, podcasts: 212, noMetadata: 9, shortPlays: 640, unknownFiles: 4 },
      });
    } else if (source === "transfer") {
      const workout = [...pick(60, 8).map((x) => this.matched(source, x)), this.missing(source, 3), this.missing(source, 4)];
      this.putList(source, "Workout", false, workout);
      this.runs.set(source, { importedAt: Date.now(), fileName, albums: { total: 0, matched: 0 }, plays: { total: 0, matched: 0 }, playsAdded: 0, skipped: { ...NONE, incompleteRows: 2 } });
    } else {
      const list = [...pick(80, 4).map((x) => this.matched(source, x)), this.missing(source, 5)];
      this.putList(source, (fileName ?? "List").replace(/\.[^.]+$/, ""), false, list);
      this.runs.set(source, { importedAt: Date.now(), fileName, albums: { total: 0, matched: 0 }, plays: { total: 0, matched: 0 }, playsAdded: 0, skipped: NONE });
    }
    // Songs nothing wants any more.
    const wanted = new Set(this.lists.flatMap((l) => l.songIds));
    this.songs = this.songs.filter((s) => wanted.has(s.songId));
    return this.report(source)!;
  }

  private report(source: ImportSourceId): ImportReport | null {
    const run = this.runs.get(source);
    if (!run) return null;
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

  forget(source: ImportSourceId) {
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
