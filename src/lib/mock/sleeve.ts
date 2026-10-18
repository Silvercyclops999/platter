/**
 * Made-up sleeve notes for the mock backend, shaped like the core's merge:
 * the files' tags and Plex at once, MusicBrainz (and Discogs with a token)
 * once the "lookup" lands. The people match the mock album credits, so the
 * album page and the now-playing panel agree. Every fifth album has no
 * match, demos are never looked up, and every eleventh album knows nothing
 * at all (the empty state). Nothing here comes from a real service.
 */
import type { Album, CreditPage, Settings, SleeveFact, SleeveNotes, SleevePerson, SleeveRoleGroup, SleeveSource, SongCredits, Track } from "../types";
import type { Fixtures } from "./fixtures";
import { mockAlbumCredits } from "./albumCredits";

const ROLES = ["producer", "writer", "composer", "lyricist", "arranger", "performer", "featured", "conductor", "engineer", "mixer", "mastering"];
const ORDER: SleeveSource[] = ["tags", "plex", "musicbrainz", "discogs"];
const STUDIOS = ["Harbour Studio", "Fenwick Hall", "The Boathouse", "Lantern Rooms"];
const PLAYERS = ["Cy Hartwell", "Odile Marsh", "Rafe Dunmore", "Ines Coltrane-Vale"];
const INSTRUMENTS = ["guitar", "bass", "drums", "keyboards", "pedal steel", "backing vocals"];

interface Row {
  name: string;
  artistId: string | null;
  role: string;
  detail?: string;
  sources: SleeveSource[];
}

const key = (name: string) => name.trim().toLowerCase().replace(/\s+/g, " ");
const sortSources = (s: SleeveSource[]) => ORDER.filter((x) => s.includes(x));

/** Rows grouped by role in sleeve order, one person per role. */
export function groupRows(rows: Row[]): SleeveRoleGroup[] {
  const out: SleeveRoleGroup[] = [];
  for (const role of ROLES) {
    const people: SleevePerson[] = [];
    for (const r of rows.filter((x) => x.role === role)) {
      let p = people.find((x) => x.key === key(r.name));
      if (!p) {
        p = { name: r.name, key: key(r.name), artistId: r.artistId, details: [], sources: [] };
        people.push(p);
      }
      if (r.detail && !p.details.includes(r.detail)) p.details.push(r.detail);
      p.sources = sortSources([...new Set([...p.sources, ...r.sources])]);
    }
    if (people.length) out.push({ role, people });
  }
  return out;
}

const fact = (value: string | null | undefined, sources: SleeveSource[]): SleeveFact | null =>
  value ? { value, sources: sortSources(sources) } : null;

export class MockSleeve {
  constructor(
    private fx: Fixtures,
    private settings: () => Settings,
    private discogsSaved: () => boolean,
  ) {}

  /** An album's notes. `fetched`: the online lookups have landed. */
  notes(album: Album, fetched: boolean): SleeveNotes {
    const s = this.settings();
    const tracks = this.fx.tracksByAlbum.get(album.id) ?? [];
    const n = Number(album.id) || album.title.length;
    const unreleased = album.section === "demos";
    const lookupsOn = s.sleeveLookups !== false && s.showSleeveNotes !== false;
    const discogsOn = lookupsOn && this.discogsSaved();
    const lookups = { online: lookupsOn, musicbrainz: lookupsOn && s.infoMusicbrainz, discogs: discogsOn, unreleased };
    const blank: SleeveNotes = {
      albumId: album.id,
      looking: false,
      lookups,
      released: null,
      editionDate: null,
      recorded: null,
      label: null,
      catalogue: null,
      country: null,
      barcode: null,
      format: null,
      edition: null,
      status: null,
      origin: null,
      notes: null,
      studios: [],
      editionNotes: [],
      groups: [],
      tracks: tracks.map((t) => ({ trackId: t.id, title: t.title, disc: t.discNumber, number: t.trackNumber, groups: [], recorded: null })),
      links: [],
    };
    if (n % 11 === 10) return blank;

    const mbOn = lookups.musicbrainz && fetched && !unreleased && n % 5 !== 4;
    const dgOn = discogsOn && fetched && !unreleased && n % 3 !== 2;
    const mb = mockAlbumCredits(album, tracks, this.fx.artists, true);
    const year = album.year ?? 2000 + (n % 20);
    const online: SleeveSource[] = [...(mbOn ? (["musicbrainz"] as const) : []), ...(dgOn ? (["discogs"] as const) : [])];

    // Album people: the producer is in the files' tags too, writers in Plex.
    const rows: Row[] = [];
    const trackRows = new Map<string, Row[]>();
    mb.credits.forEach((p) => {
      for (const role of p.roles) {
        const src: SleeveSource[] = role === "producer" ? ["tags", ...online] : role === "writer" ? ["tags", "plex", ...online] : online;
        if (src.length) rows.push({ name: p.name, artistId: p.artistId, role, sources: src });
      }
    });
    mb.tracks.forEach((t, i) => {
      const list: Row[] = [];
      for (const p of t.credits) {
        for (const role of p.roles) {
          const src: SleeveSource[] = role === "featured" ? ["tags", ...online] : role === "writer" ? ["tags", "plex"] : ["tags", ...online];
          list.push({ name: p.name, artistId: p.artistId, role, sources: src });
        }
      }
      if (online.length) {
        const player = PLAYERS[(n + i) % PLAYERS.length];
        list.push({ name: player, artistId: null, role: "performer", detail: INSTRUMENTS[(n + i) % INSTRUMENTS.length], sources: online });
      }
      trackRows.set(t.trackId, list);
      rows.push(...list.filter((r) => r.role !== "featured"));
    });
    const sources = (base: SleeveSource[]) => [...base, ...online];
    const studio = STUDIOS[n % STUDIOS.length];
    const month = String((n % 9) + 1).padStart(2, "0");
    return {
      ...blank,
      released: fact(mbOn && mb.released ? mb.released : String(year), mbOn ? ["plex", "musicbrainz"] : ["plex"]),
      editionDate: mbOn && mb.editionDate ? fact(mb.editionDate, ["musicbrainz"]) : null,
      recorded: online.length ? fact(`${year - 1}-${month}/${year - 1}-${String(Number(month) + 2).padStart(2, "0")}`, online) : null,
      label: fact(mb.label, sources(["tags", "plex"])),
      catalogue: online.length ? fact(`${(mb.label ?? "PL").slice(0, 2).toUpperCase()}-${String(n).padStart(3, "0")}`, online) : null,
      country: mbOn ? fact(n % 2 ? "GB" : "US", ["musicbrainz"]) : null,
      barcode: mbOn ? fact(`50${String(n * 7919).padStart(11, "0")}`, ["musicbrainz"]) : null,
      format: dgOn ? fact(n % 2 ? "Vinyl" : "2×Vinyl", ["discogs"]) : mbOn ? fact("CD", ["musicbrainz"]) : null,
      edition: mbOn && mb.editionDate ? fact(`${mb.editionDate.slice(0, 4)} remaster`, ["musicbrainz"]) : null,
      status: unreleased ? fact("unreleased", ["tags"]) : null,
      notes: mbOn
        ? fact(
            `Recorded over ${n % 2 ? "three weeks" : "a long winter"} at ${studio}, mostly live to tape. The ${n % 2 ? "closing song" : "title song"} was cut in a single take.`,
            ["musicbrainz"],
          )
        : null,
      studios: online.length
        ? [
            { name: studio, role: "recorded", sources: sortSources(online) },
            ...(mbOn ? [{ name: "Mix House", role: "mixed", sources: ["musicbrainz"] as SleeveSource[] }] : []),
          ]
        : [],
      editionNotes: [],
      groups: groupRows(rows),
      tracks: blank.tracks.map((t) => {
        const list = trackRows.get(t.trackId) ?? [];
        return { ...t, groups: groupRows(list), recorded: online.length && list.length ? fact(`${year - 1}-${month}`, online) : null };
      }),
      links: [
        ...(mbOn && mb.url ? [{ source: "musicbrainz" as const, url: mb.url }] : []),
        ...(dgOn ? [{ source: "discogs" as const, url: `https://www.discogs.com/release/${1000 + n}` }] : []),
      ],
    };
  }

  song(track: Track, fetched: boolean): SongCredits {
    const album = this.fx.albums.find((a) => a.id === track.albumId);
    const notes = album ? this.notes(album, fetched) : null;
    return {
      trackId: track.id,
      albumId: track.albumId,
      looking: false,
      lookups: notes?.lookups ?? { online: false, musicbrainz: false, discogs: false, unreleased: false },
      song: notes?.tracks.find((t) => t.trackId === track.id) ?? null,
      links: notes?.links ?? [],
    };
  }

  /** A person's credits across every album's notes (as if all were opened). */
  creditPage(name: string): CreditPage | null {
    const k = key(name);
    if (!k) return null;
    const byRole = new Map<string, Map<string, { whole: boolean; songs: { id: string; title: string }[]; details: string[]; sources: Set<SleeveSource> }>>();
    let shown = name;
    let artistId: string | null = null;
    for (const album of this.fx.albums) {
      const notes = this.notes(album, true);
      const add = (role: string, p: SleevePerson, song: { id: string; title: string } | null) => {
        shown = p.name;
        artistId = artistId ?? p.artistId;
        const albums: Map<string, { whole: boolean; songs: { id: string; title: string }[]; details: string[]; sources: Set<SleeveSource> }> =
          byRole.get(role) ?? new Map();
        byRole.set(role, albums);
        const a = albums.get(album.id) ?? { whole: false, songs: [], details: [], sources: new Set<SleeveSource>() };
        albums.set(album.id, a);
        if (song) {
          if (!a.songs.some((x) => x.id === song.id)) a.songs.push(song);
        } else a.whole = true;
        p.details.forEach((d) => !a.details.includes(d) && a.details.push(d));
        p.sources.forEach((x: SleeveSource) => a.sources.add(x));
      };
      const onSongs = new Set<string>();
      for (const t of notes.tracks) {
        for (const g of t.groups) {
          for (const p of g.people.filter((x) => x.key === k)) {
            add(g.role, p, { id: t.trackId, title: t.title });
            onSongs.add(g.role);
          }
        }
      }
      for (const g of notes.groups) for (const p of g.people.filter((x) => x.key === k && !onSongs.has(g.role))) add(g.role, p, null);
    }
    if (!byRole.size) return null;
    const groups = ROLES.filter((r) => byRole.has(r)).map((role) => ({
      role,
      albums: [...byRole.get(role)!.entries()]
        .map(([id, a]) => {
          const album = this.fx.albums.find((x) => x.id === id)!;
          const total = this.fx.tracksByAlbum.get(id)?.length ?? 0;
          const whole = a.whole || (total > 0 && a.songs.length >= total);
          return { album, whole, songs: whole ? [] : a.songs, details: a.details, sources: sortSources([...a.sources]) };
        })
        .sort((x, y) => (y.album.year ?? 0) - (x.album.year ?? 0) || x.album.title.localeCompare(y.album.title)),
    }));
    return { key: k, name: shown, artistId, groups };
  }
}
