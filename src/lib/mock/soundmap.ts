/**
 * The sound map in the browser mock (docs/IPC.md "Sound map"): made-up
 * points where each genre gathers around its own spot and each album
 * around a spot near it, so the map has shape. Unreleased songs come from
 * the same rule as the rest of the mock.
 */
import type { Album, SoundMapArea, SoundMapChunk, SoundMapFound, SoundMapInfo, SoundMapSong, Track } from "../types";
import type { Fixtures } from "./fixtures";

interface Pt {
  id: string;
  x: number;
  y: number;
  genre: string | null;
  year: number;
  unreleased: boolean;
}

/** A steady number from 0 to 1 per string. */
function hash01(s: string): number {
  let h = 2166136261;
  for (const c of s) h = Math.imul(h ^ c.charCodeAt(0), 16777619) >>> 0;
  // Mixed again so strings that differ in one place land far apart.
  h ^= h >>> 16;
  h = Math.imul(h, 0x85ebca6b) >>> 0;
  h ^= h >>> 13;
  h = Math.imul(h, 0xc2b2ae35) >>> 0;
  h ^= h >>> 16;
  return (h >>> 0) / 4294967296;
}

export class MockSoundMap {
  /** Songs on the map at most (test hook; null: every song). */
  limit: number | null = null;
  /** Song analysis running (test hook). */
  analysing = true;
  /** Goes up when the map changes (test hook). */
  revision = 0;
  private cache: Pt[] | null = null;

  constructor(
    private readonly fx: Fixtures,
    private readonly unreleased: (a: Album) => boolean,
  ) {}

  private albums(): Map<string, Album> {
    return new Map(this.fx.albums.map((a) => [a.id, a]));
  }

  private all(): Pt[] {
    if (this.cache) return this.cache;
    const albums = this.albums();
    const genres = [...new Set(this.fx.albums.map((a) => a.genres[0]).filter(Boolean))].sort();
    const out: Pt[] = [];
    for (const t of this.fx.tracks) {
      const a = albums.get(t.albumId);
      const genre = t.genres?.[0] ?? a?.genres[0] ?? null;
      const gi = genre ? genres.indexOf(genre) : -1;
      const ga = gi < 0 ? 0 : (gi / Math.max(1, genres.length)) * Math.PI * 2;
      const gr = gi < 0 ? 0 : 0.6;
      const aa = hash01(t.albumId) * Math.PI * 2;
      const ar = 0.08 + hash01(`${t.albumId}r`) * 0.18;
      // A soft cloud round the album's spot (roughly normal, from two steady numbers).
      const ja = hash01(`${t.id}a`) * Math.PI * 2;
      const jr = Math.sqrt(-2 * Math.log(Math.max(1e-6, hash01(`${t.id}r`)))) * 0.035;
      out.push({
        id: t.id,
        x: Math.cos(ga) * gr + Math.cos(aa) * ar + Math.cos(ja) * jr,
        y: Math.sin(ga) * gr + Math.sin(aa) * ar + Math.sin(ja) * jr,
        genre,
        year: a?.year ?? 0,
        unreleased: !!a && this.unreleased(a),
      });
    }
    out.sort((p, q) => (p.id < q.id ? -1 : p.id > q.id ? 1 : 0));
    this.cache = out;
    return out;
  }

  private placed(): Pt[] {
    const all = this.all();
    return this.limit === null ? all : all.slice(0, this.limit);
  }

  info(): SoundMapInfo {
    const placed = this.placed().length;
    return { placed, analysed: placed, total: this.fx.tracks.length, analysing: this.analysing, building: false, revision: this.revision };
  }

  points(after: string | null | undefined, limit: number | undefined, area: SoundMapArea | null | undefined): SoundMapChunk {
    const max = Math.max(1, Math.min(limit ?? 20000, 20000));
    const rows = this.placed().filter(
      (p) => (!after || p.id > after) && (!area || (p.x >= area.minX && p.x <= area.maxX && p.y >= area.minY && p.y <= area.maxY)),
    );
    const page = rows.slice(0, max);
    const genres: string[] = [];
    const chunk: SoundMapChunk = { ids: [], x: [], y: [], genre: [], genres, year: [], unreleased: [], next: null };
    for (const p of page) {
      let g = -1;
      if (p.genre) {
        g = genres.indexOf(p.genre);
        if (g < 0) g = genres.push(p.genre) - 1;
      }
      chunk.ids.push(p.id);
      chunk.x.push(p.x);
      chunk.y.push(p.y);
      chunk.genre.push(g);
      chunk.year.push(p.year);
      chunk.unreleased.push(p.unreleased);
    }
    if (rows.length > max) chunk.next = page[page.length - 1].id;
    return chunk;
  }

  find(query: string): SoundMapFound[] {
    const q = query.trim().toLowerCase();
    if (!q) return [];
    const placed = new Map(this.placed().map((p) => [p.id, p]));
    const tracks = this.fx.tracks.filter((t) => placed.has(t.id));
    const artists = new Map<string, { title: string; xs: number[]; ys: number[] }>();
    for (const t of tracks) {
      if (!t.artistTitle.toLowerCase().includes(q)) continue;
      const p = placed.get(t.id)!;
      const e = artists.get(t.artistId) ?? { title: t.artistTitle, xs: [], ys: [] };
      e.xs.push(p.x);
      e.ys.push(p.y);
      artists.set(t.artistId, e);
    }
    const avg = (v: number[]) => v.reduce((a, b) => a + b, 0) / v.length;
    const out: SoundMapFound[] = [...artists.entries()]
      .sort((a, b) => b[1].xs.length - a[1].xs.length)
      .slice(0, 3)
      .map(([id, e]) => ({ kind: "artist", id, title: e.title, sub: e.xs.length === 1 ? "1 song" : `${e.xs.length} songs`, x: avg(e.xs), y: avg(e.ys) }));
    for (const t of tracks.filter((t) => t.title.toLowerCase().includes(q)).slice(0, 6)) {
      const p = placed.get(t.id)!;
      out.push({ kind: "song", id: t.id, title: t.title, sub: t.trackArtist ?? t.artistTitle, x: p.x, y: p.y });
    }
    return out;
  }

  song(track: Track | null | undefined, albumThumb: string | null): SoundMapSong {
    if (!track) throw { kind: "notFound", message: "That song isn't in the library." };
    return { id: track.id, title: track.title, artist: track.trackArtist ?? track.artistTitle, albumTitle: track.albumTitle, thumb: track.thumb ?? albumThumb };
  }
}
