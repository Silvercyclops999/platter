/**
 * Mock of the archive read side (docs/IPC.md "Archive"): an archive-heavy
 * demo artist with eras, builds, demos, leaks and versions, plus release
 * status with evidence and marks, Other versions, artist page sections
 * (custom ones too), eras and search grouping. Every name is invented.
 *
 * Versions are worked out for the demo artist only: the other fixture
 * artists draw song titles from a small word list, so their repeats aren't
 * real versions.
 */
import type {
  Album,
  AlbumArchive,
  AlbumReleaseStatus,
  ArtistPageSection,
  Era,
  EraPage,
  Evidence,
  EvidenceKind,
  ReleaseMark,
  ReleaseSection,
  ReleaseStatus,
  SearchResults,
  Settings,
  SongStatus,
  Track,
  TrackBadges,
  AlbumBadges,
  VersionItem,
} from "../types";
import type { Fixtures } from "./fixtures";
import { ARCHIVE_ARTIST_ID, mockReleaseStatus } from "./fixtures";

export { ARCHIVE_ARTIST_ID };
const ARTIST = "Vesper Lowe";
const NOW = Date.UTC(2026, 9, 13, 12, 0, 0) / 1000;

interface SongDef {
  title: string;
  /** Seconds. */
  len: number;
  version?: string;
  quality?: string;
  date?: string;
  plays?: number;
}

interface AlbumDef {
  id: string;
  title: string;
  format: string | null;
  subformats: string[];
  year: number | null;
  era?: string;
  date?: string;
  edition?: string;
  source?: string;
  /** The albums' folder on the server, for the folder name hint. */
  folder?: string;
  /** ARCHIVE_VERSION most of its files carry. */
  version?: string;
  plays: number;
  songs: SongDef[];
}

const ALBUMS: AlbumDef[] = [
  {
    id: "9001",
    title: "Glass Season",
    format: "album",
    subformats: [],
    year: 2019,
    era: "Glass Season",
    plays: 140,
    songs: [
      { title: "Glasshouse", len: 214, plays: 310 },
      { title: "Ferryman", len: 248, plays: 260 },
      { title: "Lantern Code", len: 201, plays: 190 },
      { title: "Halflight", len: 233, plays: 120 },
      { title: "Northern Line", len: 262, plays: 88 },
      { title: "Quiet Engine", len: 187, plays: 74 },
      { title: "Tin Crown", len: 240, plays: 51 },
    ],
  },
  {
    id: "9002",
    title: "Glass Season (Build v1)",
    format: null,
    subformats: [],
    year: null,
    era: "Glass Season",
    date: "2018-11",
    edition: "Build v1",
    source: "Studio bounce, shared by the mixing engineer",
    folder: "Builds",
    version: "build",
    plays: 36,
    songs: [
      { title: "Glasshouse (Demo)", len: 198, version: "demo", date: "2018-09-14", quality: "cdq", plays: 41 },
      { title: "Ferryman - Snippet", len: 42, version: "snippet", date: "2018-10", quality: "lq", plays: 12 },
      { title: "Lantern Code [v1]", len: 205, version: "build", date: "2018-11-02", quality: "cdq", plays: 22 },
      { title: "Weathervane", len: 176, version: "build", date: "2018-11-02", quality: "cdq", plays: 18 },
    ],
  },
  {
    id: "9003",
    title: "Glass Season (Build v2)",
    format: null,
    subformats: [],
    year: null,
    era: "Glass Season",
    date: "2019-02",
    edition: "Build v2",
    source: "Late build, lossless",
    folder: "Builds",
    version: "build",
    plays: 52,
    songs: [
      { title: "Glasshouse (v2)", len: 220, version: "build", date: "2019-02-11", quality: "lossless", plays: 37 },
      { title: "Ferryman (Session)", len: 251, version: "session", date: "2019-01-20", quality: "lossless", plays: 29 },
      { title: "Halflight (Alt Mix)", len: 229, version: "alternate", date: "2019-02-11", quality: "lossless", plays: 17 },
      { title: "Weathervane", len: 181, version: "build", date: "2019-02-11", quality: "lossless", plays: 15 },
    ],
  },
  {
    id: "9004",
    title: "Night Ferry",
    format: "ep",
    subformats: [],
    year: 2021,
    era: "Night Ferry",
    plays: 96,
    songs: [
      { title: "Night Ferry", len: 236, plays: 180 },
      { title: "Harbor Lights", len: 214, plays: 140 },
      { title: "Undertow Hymn", len: 198, plays: 66 },
      { title: "Last Crossing", len: 271, plays: 43 },
    ],
  },
  {
    id: "9005",
    title: "Harbor Tapes",
    format: "album",
    subformats: ["mixtape/street"],
    year: 2020,
    era: "Night Ferry",
    plays: 58,
    songs: [
      { title: "Harbor Tapes Intro", len: 64, plays: 30 },
      { title: "Saltwater Mile", len: 202, plays: 61 },
      { title: "Dockside", len: 188, plays: 38 },
      { title: "Night Ferry (Rough)", len: 230, plays: 27 },
    ],
  },
  {
    id: "9006",
    title: "Night Ferry Sessions",
    format: "album",
    subformats: ["demo"],
    year: 2021,
    era: "Night Ferry",
    date: "2021-06",
    version: "session",
    plays: 21,
    songs: [
      { title: "Harbor Lights (Session)", len: 225, version: "session", date: "2021-06-03", quality: "lossless", plays: 14 },
      { title: "Last Crossing (Session)", len: 280, version: "session", date: "2021-06-03", quality: "lossless", plays: 9 },
      { title: "Moth Light", len: 196, version: "session", date: "2021-06-04", quality: "lossless", plays: 11 },
    ],
  },
  {
    id: "9007",
    title: "Paper Kingdom",
    format: null,
    subformats: [],
    year: null,
    era: "Paper Kingdom",
    date: "2023-08",
    folder: "Leaks",
    plays: 44,
    songs: [
      { title: "Paper Kingdom", len: 41, date: "2023-08-12", quality: "lq", plays: 52 },
      { title: "Crown of Paper", len: 38, date: "2023-08-12", quality: "lq", plays: 23 },
      { title: "Halflight (2023)", len: 55, date: "2023-08-19", quality: "lq", plays: 19 },
    ],
  },
  {
    id: "9008",
    title: "Basement Takes",
    format: null,
    subformats: [],
    year: null,
    version: "demo",
    plays: 12,
    songs: [
      { title: "Glasshouse (Piano Demo)", len: 167, version: "demo", quality: "cdq", plays: 8 },
      { title: "Tin Crown (Demo)", len: 201, version: "demo", quality: "cdq", plays: 6 },
    ],
  },
  {
    id: "9009",
    title: "Live at the Lantern",
    format: "album",
    subformats: ["live"],
    year: 2022,
    plays: 33,
    songs: [
      { title: "Glasshouse (Live)", len: 252, plays: 21 },
      { title: "Night Ferry (Live)", len: 266, plays: 18 },
      { title: "Harbor Lights (Live)", len: 240, plays: 11 },
    ],
  },
];

const SECTION_OF = (d: AlbumDef): ReleaseSection => {
  if (d.subformats.includes("demo")) return "demos";
  if (d.subformats.includes("mixtape/street")) return "mixtapes";
  if (d.subformats.includes("live")) return "live";
  return d.format === "ep" || d.format === "single" ? "singlesEps" : "albums";
};

interface TrackInfo {
  version?: string;
  quality?: string;
  date?: string;
  era?: string;
}

/** Adds the demo artist to the fixtures. Run once, after the file tags mock is built. */
export function addArchiveArtist(fx: Fixtures): { tracks: Record<string, TrackInfo>; albums: Record<string, AlbumDef> } {
  const tracks: Record<string, TrackInfo> = {};
  const albums: Record<string, AlbumDef> = {};
  if (fx.artists.some((a) => a.id === ARCHIVE_ARTIST_ID)) return { tracks, albums };
  fx.artists.push({
    id: ARCHIVE_ARTIST_ID,
    title: ARTIST,
    sortTitle: ARTIST,
    thumb: `/library/metadata/${ARCHIVE_ARTIST_ID}/thumb/1`,
    art: `/library/metadata/${ARCHIVE_ARTIST_ID}/art/1`,
    viewCount: 1640,
  });
  let seq = 90000;
  ALBUMS.forEach((d, i) => {
    albums[d.id] = d;
    const album: Album = {
      id: d.id,
      title: d.title,
      sortTitle: d.title,
      artistId: ARCHIVE_ARTIST_ID,
      artistTitle: ARTIST,
      year: d.year,
      thumb: `/library/metadata/${d.id}/thumb/${(i % 9) + 1}`,
      format: d.format,
      subformats: d.subformats,
      section: SECTION_OF(d),
      genres: ["Dream Pop"],
      trackCount: d.songs.length,
      addedAt: NOW - (ALBUMS.length - i) * 86400 * 20,
      viewCount: d.plays,
      lastViewedAt: NOW - i * 86400,
      tint: null,
    };
    fx.albums.push(album);
    const list: Track[] = d.songs.map((s, n) => {
      const id = String(seq++);
      tracks[id] = { version: s.version, quality: s.quality, date: s.date, era: d.era };
      const lossy = s.quality === "lq";
      return {
        id,
        title: s.title,
        albumId: d.id,
        albumTitle: d.title,
        artistId: ARCHIVE_ARTIST_ID,
        artistTitle: ARTIST,
        trackArtist: null,
        trackNumber: n + 1,
        discNumber: 1,
        durationMs: s.len * 1000 + 300,
        thumb: album.thumb,
        partKey: `/library/parts/${id}/1/file.${lossy ? "mp3" : "flac"}`,
        codec: lossy ? "mp3" : "flac",
        container: lossy ? "mp3" : "flac",
        bitrate: lossy ? 192 : 1011,
        bitDepth: lossy ? null : 16,
        sampleRate: 44100,
        userRating: null,
        viewCount: s.plays ?? 0,
        lastViewedAt: null,
        ratingCount: d.format ? 2000 + (s.plays ?? 0) * 40 : undefined,
      };
    });
    fx.tracks.push(...list);
    fx.tracksByAlbum.set(d.id, list);
  });
  return { tracks, albums };
}

/** The file tag badges the demo artist's files carry (for the tags mock). */
export function archiveBadges(info: { tracks: Record<string, TrackInfo>; albums: Record<string, AlbumDef> }): {
  tracks: Record<string, TrackBadges>;
  albums: Record<string, AlbumBadges>;
} {
  const tracks: Record<string, TrackBadges> = {};
  for (const [id, t] of Object.entries(info.tracks)) {
    if (t.version || t.quality) tracks[id] = { ...(t.version ? { version: t.version } : {}), ...(t.quality ? { quality: t.quality } : {}) };
  }
  const albums: Record<string, AlbumBadges> = {};
  for (const [id, a] of Object.entries(info.albums)) {
    if (a.version || a.edition) albums[id] = { ...(a.version ? { version: a.version } : {}), ...(a.edition ? { edition: a.edition } : {}) };
  }
  return { tracks, albums };
}

// ---------------------------------------------------------------- folding

const VERSION_WORDS =
  /\b(demo|snippet|session|sessions|live|remaster|remastered|mix|edit|version|alt|alternate|leak|leaked|acoustic|instrumental|early|rough|piano|v\d+|\d{4})\b/i;

/** The mock's title fold: version notes in brackets or after " - " go. */
export function mockFoldTitle(title: string): string {
  let s = title.replace(/[([{]([^)\]}]*)[)\]}]/g, (m, inner: string) => (VERSION_WORDS.test(inner) || /^\s*(feat|ft)\b/i.test(inner) ? " " : m));
  s = s
    .split(/ [-–] /)
    .filter((p, i) => i === 0 || !VERSION_WORDS.test(p))
    .join(" ");
  return s
    .normalize("NFD")
    .replace(/\p{M}/gu, "")
    .toLowerCase()
    .replace(/[^\p{L}\p{N}]+/gu, " ")
    .trim();
}

const GENERIC = new Set(["intro", "outro", "interlude", "skit", "untitled"]);

// ---------------------------------------------------------------- the mock

function demoType(a: Album): boolean {
  return !!a.format?.includes("demo") || a.subformats.some((s) => s.includes("demo"));
}

const UNRELEASED_VERSIONS = ["unreleased", "leak", "demo", "snippet", "session", "build", "alternate"];

export class MockArchive {
  readonly marks = new Map<string, ReleaseMark>();
  /** album id → section name and artist. */
  readonly custom = new Map<string, { name: string; artistId: string }>();
  private info: { tracks: Record<string, TrackInfo>; albums: Record<string, AlbumDef> } = { tracks: {}, albums: {} };

  constructor(
    private fx: Fixtures,
    private settings: () => Settings,
  ) {}

  /** Add the demo artist (after the tags mock is built). */
  install(): { tracks: Record<string, TrackBadges>; albums: Record<string, AlbumBadges> } {
    this.info = addArchiveArtist(this.fx);
    return archiveBadges(this.info);
  }

  // ---- release status

  /** `album_release_status`: the core's evidence list and precedence. */
  status(album: Album): AlbumReleaseStatus {
    const mark = this.marks.get(`album:${album.id}`) ?? null;
    const s = this.settings();
    const def = this.info.albums[album.id];
    const ev: Evidence[] = [];
    const add = (kind: EvidenceKind, text: string, weight: number, decides = false) => ev.push({ kind, text, weight, decides });
    let status: ReleaseStatus;
    const version = def?.version ?? null;
    const overridden = (unreleased: boolean) => mark && (mark === "unreleased") !== unreleased;
    if (demoType(album)) {
      add("tag", "Its release type is tagged as a demo.", 100, true);
      if (overridden(true)) add("markOverridden", "Your mark is overridden by the file's tags.", 0);
      status = "taggedUnreleased";
    } else if (version) {
      const un = UNRELEASED_VERSIONS.includes(version);
      add("tag", `Its files are tagged as version "${version}".`, un ? 100 : -100, true);
      if (overridden(un)) add("markOverridden", "Your mark is overridden by the file's tags.", 0);
      status = un ? "taggedUnreleased" : "released";
    } else if (mark) {
      if (mark === "unreleased") add("mark", "You confirmed it's unreleased.", 90, true);
      else add("mark", "You marked it as released.", -90, true);
      status = mark === "unreleased" ? "confirmedUnreleased" : "released";
    } else if (!def) {
      // The fixture library: the existing mock rule.
      const base = mockReleaseStatus(this.fx.albums, album, s);
      status = base.status;
      for (const r of base.reasons) {
        const kind: EvidenceKind = r.startsWith("Plex couldn't")
          ? "unmatched"
          : r.startsWith("Nothing in its tags")
            ? "untyped"
            : r.startsWith("MusicBrainz")
              ? "musicbrainz"
              : "tag";
        const weight = kind === "unmatched" ? 30 : kind === "untyped" ? 10 : kind === "tag" ? 100 : status === "plexMissed" ? -40 : 40;
        add(kind, r, weight, kind === "tag" || status === "plexMissed");
      }
      if (status === "probablyUnreleased" && !ev.some((e) => e.kind === "musicbrainz")) add("notChecked", "MusicBrainz not checked.", 0);
    } else if (album.format || album.subformats.length) {
      status = "released";
    } else if (s.countUnmatchedUnreleased === false) {
      status = "released";
    } else {
      add("unmatched", "Plex couldn't match it to a known release.", 30);
      add("untyped", "Nothing in its tags says what kind of release it is.", 10);
      if (s.checkUnmatchedMusicbrainz !== false && s.infoMusicbrainz !== false) {
        add("musicbrainz", "MusicBrainz has no release by this artist with this title.", 40);
      } else add("notChecked", "MusicBrainz not checked.", 0);
      status = "probablyUnreleased";
    }
    if (def) {
      const keywords = (s.archiveFolderKeywords ?? []).map((k) => k.toLowerCase().replace(/s$/, ""));
      if (def.folder && keywords.includes(def.folder.toLowerCase().replace(/s$/, ""))) {
        const shown = (s.archiveFolderKeywords ?? []).find((k) => k.toLowerCase().replace(/s$/, "") === def.folder!.toLowerCase().replace(/s$/, ""));
        add("folder", `Folder name: ${shown}`, 10);
      }
      const short = def.songs.filter((x) => x.len < 60).length;
      if (short * 2 > def.songs.length) add("duration", "Most of its songs are under a minute long.", 5);
    }
    return {
      id: album.id,
      artistId: album.artistId,
      status,
      reasons: ev.map((e) => e.text),
      evidence: ev,
      mark,
      score: ev.reduce((n, e) => n + e.weight, 0),
    };
  }

  private albumUnreleased(album: Album): boolean {
    const st = this.status(album).status;
    return st === "taggedUnreleased" || st === "confirmedUnreleased" || st === "probablyUnreleased";
  }

  setMark(kind: "album" | "track", id: string, mark: ReleaseMark | null) {
    const key = `${kind}:${id}`;
    if (mark) this.marks.set(key, mark);
    else this.marks.delete(key);
  }

  // ---- versions

  private groupKey(t: Track): string {
    const title = mockFoldTitle(t.title);
    return GENERIC.has(title) || title.length < 2 ? `g:${t.albumId}:${title}` : `t:${t.artistId}:${title}`;
  }

  /** The song's status, like the core: tags, then marks, then the album's guess. */
  songStatus(t: Track): SongStatus {
    const base = this.baseStatus(t);
    if (base !== "probablyUnreleased") return base;
    return this.othersOf(t).some((o) => this.baseStatus(o) === "released") ? "probablyAlternate" : "probablyUnreleased";
  }

  private baseStatus(t: Track): SongStatus {
    const album = this.fx.albums.find((a) => a.id === t.albumId);
    if (!album) return "released";
    const own = this.marks.get(`track:${t.id}`);
    const version = this.info.tracks[t.id]?.version;
    if (version) return UNRELEASED_VERSIONS.includes(version) ? "unreleased" : "released";
    if (own) return own === "unreleased" ? "unreleased" : "released";
    const st = this.status(album).status;
    if (st === "released" || st === "plexMissed") return "released";
    return st === "probablyUnreleased" ? "probablyUnreleased" : "unreleased";
  }

  /** Other versions of a song (the demo artist's only). */
  othersOf(t: Track): Track[] {
    if (t.artistId !== ARCHIVE_ARTIST_ID) return [];
    const key = this.groupKey(t);
    return this.fx.tracks.filter((o) => o.id !== t.id && o.artistId === ARCHIVE_ARTIST_ID && this.groupKey(o) === key);
  }

  annotate(list: Track[]): Track[] {
    return list.map((t) => {
      if (t.artistId !== ARCHIVE_ARTIST_ID) return t;
      const n = this.othersOf(t).length;
      return { ...t, otherVersions: n || null, songStatus: this.songStatus(t) };
    });
  }

  /** `track_versions`: released first, then newest. */
  versions(trackId: string): VersionItem[] {
    const t = this.fx.tracks.find((x) => x.id === trackId);
    if (!t) return [];
    const all = this.annotate([t, ...this.othersOf(t)]);
    const items: VersionItem[] = all.map((x) => {
      const info = this.info.tracks[x.id] ?? {};
      const album = this.fx.albums.find((a) => a.id === x.albumId);
      const def = this.info.albums[x.albumId];
      return {
        track: x,
        status: x.songStatus ?? "released",
        version: info.version ?? null,
        quality: info.quality ?? null,
        date: info.date ?? def?.date ?? null,
        era: info.era ?? null,
        edition: def?.edition ?? null,
        year: album?.year ?? null,
        current: x.id === trackId,
      };
    });
    const when = (v: VersionItem) => (v.date ? Number(v.date.replace(/-/g, "").padEnd(8, "0")) : v.year ? v.year * 10000 : -1);
    return items.sort((a, b) => Number(a.status !== "released") - Number(b.status !== "released") || when(b) - when(a));
  }

  /** Search: one row per song, shown as its released version. */
  groupSearch(r: SearchResults): SearchResults {
    const seen = new Set<string>();
    const songs: Track[] = [];
    const versions: Record<string, Track[]> = {};
    const face = (t: Track): Track => {
      if (t.artistId !== ARCHIVE_ARTIST_ID) return t;
      if (this.songStatus(t) === "released") return t;
      const rel = this.othersOf(t)
        .filter((o) => this.songStatus(o) === "released")
        .sort((a, b) => b.viewCount - a.viewCount)[0];
      return rel ?? t;
    };
    for (const t of r.songs) {
      if (t.artistId !== ARCHIVE_ARTIST_ID) {
        songs.push(t);
        continue;
      }
      const key = this.groupKey(t);
      if (seen.has(key)) continue;
      seen.add(key);
      const f = this.annotate([face(t)])[0];
      const others = this.annotate(this.othersOf(f));
      if (others.length) versions[f.id] = others;
      songs.push(f);
    }
    const top = r.top?.kind === "track" && r.top.track ? { ...r.top, track: this.annotate([face(r.top.track)])[0] } : r.top;
    return { ...r, top, songs, versions };
  }

  // ---- sections

  sections(albums: Album[]): ArtistPageSection[] {
    const order: ReleaseSection[] = ["albums", "singlesEps", "mixtapes", "unreleased", "live", "compilations", "soundtracks", "remixes", "demos"];
    const placed = albums.map((a) => ({
      a,
      un: this.albumUnreleased(a),
      demo: (this.info.albums[a.id]?.version ?? "") === "demo",
      custom: this.custom.get(a.id)?.name ?? null,
    }));
    const un = placed.filter((p) => !p.custom && p.un);
    const split = un.some((p) => p.demo) && un.some((p) => !p.demo);
    const by = new Map<ReleaseSection, Album[]>();
    const custom = new Map<string, Album[]>();
    for (const p of placed) {
      if (p.custom) {
        custom.set(p.custom, [...(custom.get(p.custom) ?? []), p.a]);
        continue;
      }
      const section: ReleaseSection = p.un ? (split && p.demo ? "demos" : "unreleased") : p.a.section === "demos" ? "albums" : p.a.section;
      by.set(section, [...(by.get(section) ?? []), { ...p.a, section }]);
    }
    const newest = (a: Album, b: Album) => (b.year ?? 9999) - (a.year ?? 9999) || b.addedAt - a.addedAt;
    const out: ArtistPageSection[] = order.filter((s) => by.has(s)).map((s) => ({ section: s, albums: by.get(s)!.sort(newest) }));
    for (const name of [...custom.keys()].sort((a, b) => a.localeCompare(b))) out.push({ section: "albums", albums: custom.get(name)!, custom: name });
    return out;
  }

  setSection(albumId: string, artistId: string, name: string | null) {
    if (name === null) {
      this.custom.delete(albumId);
      return;
    }
    const n = name.trim();
    if (!n || n.length > 40) throw { kind: "invalid", message: "Section names are 1 to 40 characters." };
    const same = this.sectionNames(artistId).find((x) => x.toLowerCase() === n.toLowerCase());
    this.custom.set(albumId, { name: same ?? n, artistId });
  }

  sectionNames(artistId: string): string[] {
    const names = new Set<string>();
    for (const v of this.custom.values()) if (v.artistId === artistId) names.add(v.name);
    return [...names].sort((a, b) => a.localeCompare(b));
  }

  // ---- eras

  eras(artistId: string): Era[] {
    if (artistId !== ARCHIVE_ARTIST_ID) return [];
    const byName = new Map<string, { tracks: Track[]; albums: Album[]; dates: string[]; years: number[] }>();
    for (const t of this.fx.tracks.filter((x) => x.artistId === artistId)) {
      const era = this.info.tracks[t.id]?.era;
      if (!era) continue;
      const e = byName.get(era) ?? { tracks: [], albums: [], dates: [], years: [] };
      e.tracks.push(t);
      const album = this.fx.albums.find((a) => a.id === t.albumId)!;
      if (!e.albums.includes(album)) {
        e.albums.push(album);
        if (album.year) e.years.push(album.year);
      }
      const d = this.info.tracks[t.id]?.date ?? this.info.albums[t.albumId]?.date;
      if (d) e.dates.push(d);
      byName.set(era, e);
    }
    const eras = [...byName.entries()].map(([name, e]) => {
      const dates = [...e.dates].sort();
      const years = [...e.years].sort();
      const cover = [...e.albums].sort((a, b) => b.viewCount - a.viewCount)[0];
      const start = dates[0] ?? (years[0] ? String(years[0]) : null);
      const end = dates[dates.length - 1] ?? (years.length ? String(years[years.length - 1]) : null);
      return {
        name,
        start,
        end,
        trackCount: e.tracks.length,
        albumCount: e.albums.length,
        cover: cover?.thumb ?? null,
        coverAlbumId: cover?.id ?? null,
        albumIds: e.albums.map((a) => a.id),
        trackIds: e.tracks.map((t) => t.id),
      } satisfies Era;
    });
    return eras.sort((a, b) => (a.start ?? "9999").localeCompare(b.start ?? "9999"));
  }

  eraPage(artistId: string, name: string): EraPage | null {
    const era = this.eras(artistId).find((e) => e.name.toLowerCase() === name.toLowerCase());
    const artist = this.fx.artists.find((a) => a.id === artistId);
    if (!era || !artist) return null;
    const groups = era.albumIds.map((id) => {
      const album = this.fx.albums.find((a) => a.id === id)!;
      const tracks = this.annotate((this.fx.tracksByAlbum.get(id) ?? []).filter((t) => era.trackIds.includes(t.id)));
      const dates = tracks.map((t) => this.info.tracks[t.id]?.date).filter((d): d is string => !!d).sort();
      return { album, date: dates[dates.length - 1] ?? this.info.albums[id]?.date ?? null, edition: this.info.albums[id]?.edition ?? null, tracks };
    });
    const key = (g: (typeof groups)[number]) => g.date ?? (g.album.year ? String(g.album.year) : "");
    groups.sort((a, b) => key(b).localeCompare(key(a)));
    return { artist, era, groups };
  }

  albumArchive(album: Album): AlbumArchive | null {
    const def = this.info.albums[album.id];
    if (!def || !this.albumUnreleased(album)) return null;
    return { era: def.era ?? null, date: def.date ?? null, edition: def.edition ?? null, source: def.source ?? null };
  }
}
