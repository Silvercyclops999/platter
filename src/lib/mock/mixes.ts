/**
 * Mixes and radio for the mock backend: a few generated mixes from the
 * fixtures (artist photos and covers are the fixtures' fake art), shaped
 * like the core's. Not the real engine; just enough to demo and test the UI.
 */
import type { MixKind, MixPage, MixShelves, MixSteer, MixSummary, Track } from "../types";
import { mulberry32, type Fixtures } from "./fixtures";

const PALETTE = ["#f2a7c3", "#f5b971", "#8fc1f0", "#9fd8a6", "#c9b1f2", "#f2d16b", "#7fd6d0", "#f29b8f", "#b8d77a", "#e3b5e8"];
const MOODS = ["Late Night", "Mellow", "Upbeat"];
const PARTS: [number, string][] = [
  [5, "late night"],
  [9, "early morning"],
  [12, "morning"],
  [17, "afternoon"],
  [21, "evening"],
  [24, "night"],
];
const DAYS = ["sunday", "monday", "tuesday", "wednesday", "thursday", "friday", "saturday"];

function hash(s: string): number {
  let h = 2166136261;
  for (let i = 0; i < s.length; i++) h = Math.imul(h ^ s.charCodeAt(i), 16777619);
  return h >>> 0;
}

function shuffled<T>(list: T[], seed: string): T[] {
  const r = mulberry32(hash(seed));
  const out = [...list];
  for (let i = out.length - 1; i > 0; i--) {
    const j = Math.floor(r() * (i + 1));
    [out[i], out[j]] = [out[j], out[i]];
  }
  return out;
}

/** Keeps an artist to two songs in a row where it can. */
function spaced(list: Track[]): Track[] {
  const out = [...list];
  for (let i = 2; i < out.length; i++) {
    const a = out[i].artistId;
    if (out[i - 1].artistId === a && out[i - 2].artistId === a) {
      const j = out.findIndex((t, k) => k > i && t.artistId !== a);
      if (j > 0) out.splice(i, 0, ...out.splice(j, 1));
    }
  }
  return out;
}

const familiar = (t: Track) => t.viewCount >= 20 || t.userRating === 10;

export function partOfDay(hour: number): string {
  return PARTS.find(([end]) => hour < end)![1];
}

interface Made {
  mix: MixSummary;
  tracks: Track[];
  familiar: boolean[];
}

export class MockMixes {
  private made: Map<string, Made> | null = null;
  private salt = 0;

  constructor(
    private fx: Fixtures,
    /** Tracks that can go in mixes (the fan edit and explicit filters). */
    private visible: (tracks: Track[]) => Track[],
    private now: () => Date = () => new Date(),
  ) {}

  private summary(id: string, kind: MixKind, title: string, description: string, color: number, tracks: Track[], extra: Partial<MixSummary> = {}): MixSummary {
    const lead = this.leading(tracks);
    const art = lead.map((a) => this.fx.artists.find((x) => x.id === a)?.thumb).find(Boolean) ?? null;
    return {
      id,
      kind,
      title,
      description,
      color: PALETTE[color % PALETTE.length],
      art,
      covers: [...new Set(tracks.map((t) => t.thumb).filter((t): t is string => !!t))].slice(0, 4),
      artistIds: lead,
      tag: null,
      trackCount: tracks.length,
      durationMs: tracks.reduce((n, t) => n + t.durationMs, 0),
      nextUpdateAt: null,
      ...extra,
    };
  }

  private leading(tracks: Track[]): string[] {
    const n = new Map<string, number>();
    tracks.forEach((t) => n.set(t.artistId, (n.get(t.artistId) ?? 0) + 1));
    return [...n.entries()].sort((a, b) => b[1] - a[1]).map(([a]) => a).slice(0, 6);
  }

  private line(tracks: Track[], prefix = ""): string {
    const names = this.leading(tracks)
      .slice(0, 3)
      .map((a) => this.fx.artists.find((x) => x.id === a)?.title)
      .filter(Boolean);
    return names.length ? `${prefix}${names.join(", ")} and more` : "";
  }

  /** Favourites first, a share of less played songs mixed in, 50 at most. */
  private blend(pool: Track[], seed: string, share: number, n = 50): { tracks: Track[]; familiar: boolean[] } {
    const fav = shuffled(pool.filter(familiar), seed).sort((a, b) => b.viewCount - a.viewCount);
    const fresh = shuffled(pool.filter((t) => !familiar(t)), seed);
    const wantFav = Math.round(n * share);
    const f = fav.slice(0, wantFav);
    const nw = fresh.slice(0, n - f.length);
    const more = fav.slice(f.length, f.length + (n - f.length - nw.length));
    const favs = [...f, ...more];
    const out: Track[] = [];
    const every = nw.length ? Math.max(1, Math.round((favs.length + nw.length) / nw.length)) : 0;
    let fi = 0;
    let ni = 0;
    while (fi < favs.length || ni < nw.length) {
      if (ni < nw.length && (fi >= favs.length || (every && (out.length + 1) % every === 0))) out.push(nw[ni++]);
      else out.push(favs[fi++]);
    }
    const tracks = spaced(out);
    return { tracks, familiar: tracks.map((t) => favs.includes(t)) };
  }

  private make(): Map<string, Made> {
    const out = new Map<string, Made>();
    const all = this.visible(this.fx.tracks);
    const genres = [...new Set(this.fx.albums.flatMap((a) => a.genres))];
    const genreOf = (t: Track) => this.fx.albums.find((a) => a.id === t.albumId)?.genres[0] ?? "";
    const salt = String(this.salt);
    // Daily Mixes: three pairs of genres.
    for (let i = 0; i < 3; i++) {
      const pair = genres.slice(i * 2, i * 2 + 2);
      const pool = all.filter((t) => pair.includes(genreOf(t)));
      const { tracks, familiar } = this.blend(pool, `daily${i}${salt}`, 0.7);
      const id = `daily:${i + 1}`;
      out.set(id, { mix: this.summary(id, "daily", `Daily Mix ${i + 1}`, this.line(tracks), i, tracks), tracks, familiar });
    }
    const now = this.now();
    const part = partOfDay(now.getHours());
    {
      const pool = all.filter((t) => genreOf(t) === genres[(now.getHours() >> 2) % genres.length] || genreOf(t) === genres[1]);
      const { tracks, familiar } = this.blend(pool, `daylist${now.getHours()}${salt}`, 0.7);
      const words = [...new Set(tracks.map(genreOf))].slice(0, 2).map((g) => g.toLowerCase());
      const next = new Date(now);
      next.setHours(Math.floor(now.getHours() / 3) * 3 + 3, 0, 0, 0);
      const desc = `Here's some ${[...new Set(tracks.map(genreOf))].slice(0, 6).map((g) => g.toLowerCase()).join(", ")}, inspired by your listening on ${DAYS[now.getDay()][0].toUpperCase() + DAYS[now.getDay()].slice(1)}s ${part === "night" ? "at night" : part === "late night" ? "late at night" : `in the ${part}`}.`;
      out.set("daylist", {
        mix: this.summary("daylist", "daylist", `${words.join(" ")} ${DAYS[now.getDay()]} ${part}`, desc, 5, tracks, { nextUpdateAt: next.getTime() }),
        tracks,
        familiar,
      });
    }
    {
      const pool = shuffled(all.filter((t) => t.viewCount === 0), `discover${salt}`).slice(0, 30);
      out.set("discover", { mix: this.summary("discover", "discover", "Discover", "Songs from your library you haven't played yet, picked for you.", 2, pool), tracks: pool, familiar: pool.map(() => false) });
    }
    {
      const top = [...all].filter((t) => t.viewCount > 0).sort((a, b) => b.viewCount - a.viewCount).slice(0, 30);
      out.set("onRepeat", { mix: this.summary("onRepeat", "onRepeat", "On Repeat", "Songs you can't stop playing this month.", 0, top), tracks: top, familiar: top.map(() => true) });
    }
    {
      const top = [...all].filter((t) => t.viewCount > 0).sort((a, b) => b.viewCount - a.viewCount).slice(0, 100);
      out.set("topSongs", { mix: this.summary("topSongs", "topSongs", "All-Time Top Songs", "Your most played songs, ever.", 8, top), tracks: top, familiar: top.map(() => true) });
    }
    MOODS.forEach((m, i) => {
      const pool = all.filter((t) => hash(t.id + m) % 3 === 0);
      const { tracks, familiar } = this.blend(pool, `mood${m}${salt}`, 0.7);
      const id = `mood:${m.toLowerCase()}`;
      out.set(id, { mix: this.summary(id, "mood", `${m} Mix`, this.line(tracks), 3 + i, tracks, { tag: m }), tracks, familiar });
    });
    genres.slice(0, 5).forEach((g, i) => {
      const pool = all.filter((t) => genreOf(t) === g);
      if (pool.length < 25) return;
      const { tracks, familiar } = this.blend(pool, `genre${g}${salt}`, 0.7);
      const id = `genre:${g.toLowerCase()}`;
      out.set(id, { mix: this.summary(id, "genre", `${g} Mix`, this.line(tracks), 7 + i, tracks, { tag: g }), tracks, familiar });
    });
    return out;
  }

  private mixes(): Map<string, Made> {
    if (!this.made) this.made = this.make();
    return this.made;
  }

  /** The library or a setting changed: make them again next time. */
  invalidate() {
    this.made = null;
  }

  shelves(): MixShelves {
    const all = [...this.mixes().values()].map((m) => m.mix);
    // Top Songs lists live in search and on the profile, not on Home.
    const order: MixKind[] = ["daily", "daylist", "discover", "newInLibrary", "onRepeat", "rediscover"];
    return {
      madeForYou: all.filter((m) => order.includes(m.kind)).sort((a, b) => order.indexOf(a.kind) - order.indexOf(b.kind)),
      moods: all.filter((m) => m.kind === "mood"),
      genres: all.filter((m) => m.kind === "genre"),
    };
  }

  page(id: string, steer: MixSteer | null = null): MixPage | null {
    const m = this.mixes().get(id);
    if (!m) return null;
    if (!steer || m.mix.kind === "topSongs" || m.mix.kind === "onRepeat") return { ...m, steer: null };
    const pool = [...new Set([...m.tracks, ...this.visible(this.fx.tracks).filter((t) => m.mix.artistIds.includes(t.artistId))])];
    const { tracks, familiar } = this.blend(pool, `${id}${this.salt}`, steer === "familiar" ? 0.85 : 0.5);
    return { mix: { ...m.mix, trackCount: tracks.length, durationMs: tracks.reduce((n, t) => n + t.durationMs, 0) }, tracks, familiar, steer };
  }

  refresh(id: string): MixPage | null {
    if (!this.mixes().has(id)) return null;
    this.salt++;
    const fresh = this.make();
    const m = fresh.get(id);
    if (m) this.mixes().set(id, m);
    return m ? { ...m, steer: null } : null;
  }

  /** Songs like the seed ids, none in `avoid`. */
  similar(seedIds: string[], avoid: Set<string>, n: number, salt: string): Track[] {
    const seeds = seedIds.map((i) => this.fx.tracks.find((t) => t.id === i)).filter((t): t is Track => !!t);
    const artists = new Set(seeds.map((t) => t.artistId));
    const genres = new Set(seeds.map((t) => this.fx.albums.find((a) => a.id === t.albumId)?.genres[0]));
    const pool = this.visible(this.fx.tracks).filter(
      (t) => !avoid.has(t.id) && (artists.has(t.artistId) || genres.has(this.fx.albums.find((a) => a.id === t.albumId)?.genres[0])),
    );
    return spaced(shuffled(pool, salt)).slice(0, n);
  }

  /** A radio page: `track:<id>` or `artist:<id>`. */
  radio(seed: string, steer: MixSteer | null = null, salt = 0): MixPage | null {
    const [kind, id] = seed.split(":");
    let first: Track | undefined;
    let name: string | undefined;
    let picture: string | null = null;
    let seeds: string[];
    if (kind === "track") {
      first = this.fx.tracks.find((t) => t.id === id);
      if (!first) return null;
      name = first.title;
      picture = first.thumb;
      seeds = [first.id];
    } else if (kind === "artist") {
      const a = this.fx.artists.find((x) => x.id === id);
      if (!a) return null;
      name = a.title;
      picture = a.thumb;
      seeds = this.fx.tracks.filter((t) => t.artistId === id).slice(0, 5).map((t) => t.id);
    } else return null;
    const rest = this.similar(seeds, new Set(first ? [first.id] : []), first ? 49 : 50, `${seed}${salt}`);
    const sorted = steer === "familiar" ? [...rest].sort((a, b) => b.viewCount - a.viewCount) : steer === "new" ? [...rest].sort((a, b) => a.viewCount - b.viewCount) : rest;
    const tracks = first ? [first, ...sorted] : sorted;
    const seedArtist = first?.artistId ?? id;
    const lead = [seedArtist, ...this.leading(tracks).filter((a) => a !== seedArtist)];
    const names = lead.slice(0, 3).map((a) => this.fx.artists.find((x) => x.id === a)?.title).filter(Boolean);
    const others = lead.slice(1).map((a) => this.fx.artists.find((x) => x.id === a)?.thumb).filter((t): t is string => !!t).slice(0, 2);
    const mix = this.summary(seed, "radio", `${name} Radio`, `with ${names.join(", ")} and more`, hash(seed), tracks, {
      art: picture,
      covers: [picture, ...others].filter((t): t is string => !!t),
      artistIds: lead.slice(0, 6),
    });
    return { mix, tracks, familiar: tracks.map(familiar), steer };
  }
}
