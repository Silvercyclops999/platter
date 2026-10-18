/**
 * Music that isn't in the library, in the browser mock: a small invented
 * catalogue standing in for MusicBrainz and ListenBrainz (names made up
 * here). One library artist is in it too, so search shows the folding.
 */
import type {
  CatAlbum,
  CatArtist,
  CatalogueAlbumPage,
  CatalogueArtistPage,
  CataloguePart,
  CataloguePartKind,
  CataloguePhoto,
  CatalogueSearch,
  CatSection,
  FeaturedArtist,
  GetAlbumsPlan,
  CatSong,
  CatTrack,
  LidarrRequest,
  MissingAlbum,
  NewToYou,
  Settings,
  WantedAlbum,
} from "../types";
import type { Fixtures } from "./fixtures";
import { coverThumb, sectionWanted } from "./lidarr";

export interface CatalogueHooks {
  settings: () => Settings;
  offline: () => boolean;
  requests: () => LidarrRequest[];
  /** Lidarr has an address and a key (its search can be asked). */
  lidarrSetUp?: () => boolean;
}

/** A made-up photo for most catalogue artists; Ostra (5) has none anywhere, so her card shows a monogram. */
const photoOf = (n: number): CataloguePhoto =>
  n === 5
    ? { photo: null, credit: null }
    : n % 2 === 0
      ? { photo: `/ext/https://commons.wikimedia.org/wiki/Special:FilePath/Catalogue_${n}.jpg?width=600`, credit: "Wikimedia Commons" }
      : { photo: `/ext/https://assets.fanart.tv/fanart/music/catalogue-${n}.jpg`, credit: null };

interface Rel {
  n: number;
  title: string;
  kind: "album" | "ep" | "single";
  secondary?: string[];
  year: number;
}

interface Who {
  n: number;
  name: string;
  kind: "Group" | "Person";
  disambiguation?: string;
  area: string;
  since: string;
  releases: Rel[];
}

const mbid = (n: number) => `c0ffee00-0000-4000-8000-${String(n).padStart(12, "0")}`;
const rgid = (n: number) => `c0ffee11-0000-4000-8000-${String(n).padStart(12, "0")}`;

const WORLD: Who[] = [
  {
    n: 1,
    name: "Cold Coast",
    kind: "Group",
    disambiguation: "Glasgow band",
    area: "Glasgow",
    since: "2012–present",
    releases: [
      { n: 11, title: "Low Tide Radio", kind: "album", year: 2021 },
      { n: 12, title: "Harbour Static", kind: "album", year: 2018 },
      { n: 13, title: "Weatherglass", kind: "ep", year: 2019 },
      { n: 14, title: "First Frost", kind: "single", year: 2023 },
      { n: 15, title: "Live at the Lido", kind: "album", secondary: ["live"], year: 2022 },
    ],
  },
  {
    n: 2,
    name: "Tidewater Hymns",
    kind: "Group",
    area: "Bristol",
    since: "2014–present",
    releases: [
      { n: 21, title: "Saltmarsh", kind: "album", year: 2020 },
      { n: 22, title: "Tidewater Hymns", kind: "album", year: 2016 },
      { n: 23, title: "Ebb", kind: "ep", year: 2017 },
    ],
  },
  {
    n: 3,
    name: "Fen Lights",
    kind: "Person",
    disambiguation: "singer-songwriter",
    area: "Norwich",
    since: "2017–present",
    releases: [
      { n: 31, title: "Reedbeds", kind: "album", year: 2022 },
      { n: 32, title: "Night Ferry", kind: "single", year: 2024 },
    ],
  },
  { n: 4, name: "Marrow & Vine", kind: "Group", area: "Leeds", since: "2009–2019", releases: [{ n: 41, title: "Copper Seasons", kind: "album", year: 2019 }] },
  { n: 5, name: "Ostra", kind: "Person", area: "Lisbon", since: "2018–present", releases: [{ n: 51, title: "Salt Choir", kind: "album", year: 2021 }] },
  {
    n: 6,
    name: "Paper Satellites",
    kind: "Group",
    area: "Cork",
    since: "2011–present",
    releases: [
      { n: 61, title: "Orbit Songs", kind: "album", year: 2015 },
      { n: 62, title: "Lowlight", kind: "ep", year: 2017 },
    ],
  },
];

/** Featured guests on library songs (fixtures' GUEST_NAMES), as the catalogue knows them. */
const GUESTS: Who[] = [
  {
    n: 71,
    name: "Juno Ashby",
    kind: "Person",
    area: "Bristol",
    since: "2012–present",
    releases: [
      { n: 711, title: "Overcast Bloom", kind: "album", year: 2023 },
      { n: 712, title: "Small Rooms", kind: "album", year: 2019 },
      { n: 713, title: "Paper Weather", kind: "album", year: 2015 },
      { n: 714, title: "Hollow Light", kind: "single", year: 2024 },
      { n: 715, title: "Late Ferry", kind: "single", year: 2022 },
      { n: 716, title: "Four Walls", kind: "ep", year: 2017 },
      { n: 717, title: "Collected Rooms", kind: "album", secondary: ["compilation"], year: 2021 },
      { n: 718, title: "Live at the Fleece", kind: "album", secondary: ["live"], year: 2020 },
    ],
  },
  {
    n: 72,
    name: "Pell Rivers",
    kind: "Person",
    area: "Cardiff",
    since: "2016–present",
    releases: [
      { n: 721, title: "Brightwater", kind: "album", year: 2021 },
      { n: 722, title: "Rivers EP", kind: "ep", year: 2018 },
    ],
  },
];

/** The library's section a catalogue release shows in (same ids as the library's). */
export function sectionOf(r: Pick<Rel, "kind" | "secondary">): string | null {
  const sec = r.secondary ?? [];
  if (!sec.length) return r.kind === "album" ? "albums" : "singlesEps";
  if (sec.includes("compilation")) return "compilations";
  if (sec.includes("live")) return "live";
  if (sec.includes("soundtrack")) return "soundtracks";
  if (sec.includes("remix")) return "remixes";
  return null;
}

const SECTION_ORDER = ["albums", "singlesEps", "compilations", "live", "soundtracks", "remixes", "mixtapes", "demos"];


const SONG_WORDS = ["Undertow", "Glass Weather", "Breakwater", "Salt Lines", "Pilot Light", "Low Sun", "Rook", "Shoreline", "The Long Field", "Lantern Hours", "Foghorn", "Quiet Engine"];

const fold = (s: string) => s.trim().toLowerCase().replace(/\s+/g, " ");

export class MockCatalogue {
  private wanted: WantedAlbum[] = [];
  /** Every search asked (tests read it). */
  readonly searches: string[] = [];
  /** Every search part asked, as "part:query:source" (tests read it). */
  readonly parts: string[] = [];
  /** Every photo asked for, by MusicBrainz id (tests read it). */
  readonly photos: string[] = [];

  constructor(
    private fx: Fixtures,
    private hooks: CatalogueHooks,
  ) {}

  /** The library artist the catalogue also knows (to show folding). */
  private get libraryTwin() {
    return this.fx.artists[2];
  }

  private artist(w: Who): CatArtist {
    return { mbid: mbid(w.n), name: w.name, disambiguation: w.disambiguation ?? null, kind: w.kind, photo: null, libraryId: null };
  }

  private album(w: Who, r: Rel): CatAlbum {
    const id = rgid(r.n);
    return {
      releaseGroupId: id,
      title: r.title,
      artist: w.name,
      artistMbid: mbid(w.n),
      kind: r.kind,
      secondary: r.secondary ?? [],
      year: r.year,
      thumb: coverThumb(id),
      libraryAlbumId: null,
      request: this.hooks.requests().find((q) => q.releaseGroupId === id)?.state ?? null,
      refusal: null,
      inMissing: this.wanted.some((x) => x.releaseGroupId === id),
    };
  }

  private tracks(r: Rel): CatTrack[] {
    const n = r.kind === "single" ? 2 : r.kind === "ep" ? 5 : 10;
    return Array.from({ length: n }, (_, i) => ({
      disc: null,
      number: i + 1,
      title: i === 0 ? r.title : SONG_WORDS[(r.n + i * 5) % SONG_WORDS.length],
      artist: null,
      durationMs: 150_000 + ((r.n * 37 + i * 53) % 140) * 1000,
    }));
  }

  private dismissed(): Set<string> {
    return new Set((this.hooks.settings().dismissedArtists ?? []).map((d) => d.mbid));
  }

  search(query: string): CatalogueSearch {
    const s = this.hooks.settings();
    if (!s.catalogueSearch || this.hooks.offline()) return { shown: false, artists: [], albums: [], songs: [] };
    this.searches.push(query);
    const q = fold(query);
    if (q.length < 2) return { shown: true, artists: [], albums: [], songs: [] };
    const artists = WORLD.filter((w) => fold(w.name).includes(q)).map((w) => this.artist(w));
    // The library artist matches too, and folds into the library results.
    const albums: CatAlbum[] = [];
    const songs: CatSong[] = [];
    for (const w of WORLD) {
      for (const r of w.releases) {
        if (fold(r.title).includes(q) || fold(w.name).includes(q)) albums.push(this.album(w, r));
        for (const t of this.tracks(r)) {
          if (fold(t.title).includes(q) && !songs.some((x) => x.title === t.title && x.artist === w.name))
            songs.push({
              recordingId: `${rgid(r.n)}-${t.number}`,
              title: t.title,
              artist: w.name,
              artistMbid: mbid(w.n),
              album: r.title,
              releaseGroupId: rgid(r.n),
              durationMs: t.durationMs,
              thumb: coverThumb(rgid(r.n)),
            });
        }
      }
    }
    return { shown: true, artists: artists.slice(0, 6), albums: albums.slice(0, 6), songs: songs.slice(0, 6) };
  }

  /** Where search parts come from now: Lidarr's search when it's set up and the setting allows. */
  source(part: CataloguePartKind): "lidarr" | "musicBrainz" {
    const s = this.hooks.settings();
    return part !== "songs" && s.catalogueSource !== "musicbrainz" && !!this.hooks.lidarrSetUp?.() ? "lidarr" : "musicBrainz";
  }

  /** One kind of results. Lidarr's answers carry the artist's photo; the library twin folds in. */
  searchPart(query: string, part: CataloguePartKind): CataloguePart {
    const s = this.hooks.settings();
    const empty: CataloguePart = { shown: false, part, source: "", artists: [], albums: [], songs: [], inLibrary: [] };
    if (!s.catalogueSearch || this.hooks.offline()) return empty;
    const source = this.source(part);
    this.parts.push(`${part}:${query}:${source}`);
    const all = this.search(query);
    this.searches.pop();
    const twin = this.libraryTwin;
    const inLibrary = part === "artists" && fold(twin.title).includes(fold(query)) && fold(query).length >= 2 ? [twin.id] : [];
    // Lidarr's answer carries the fanart.tv pictures; the rest are asked for as the rows show.
    const fromLidarr = (a: CatArtist) => {
      const p = photoOf(this.nOf(a.mbid));
      return p.photo && !p.credit ? p.photo : null;
    };
    const artists = part === "artists" ? all.artists.map((a) => (source === "lidarr" ? { ...a, photo: fromLidarr(a) } : a)) : [];
    return {
      shown: true,
      part,
      source,
      artists,
      albums: part === "albums" ? all.albums : [],
      songs: part === "songs" ? all.songs : [],
      inLibrary,
    };
  }

  private nOf(id: string): number {
    return WORLD.find((w) => mbid(w.n) === id)?.n ?? 0;
  }

  /** A photo for a catalogue artist (Lidarr, Commons or TheAudioDB in the real app). */
  photo(id: string): CataloguePhoto {
    this.photos.push(id);
    if (this.hooks.offline()) return { photo: null, credit: null };
    return photoOf(this.nOf(id));
  }

  /** "Get albums": the artist's albums of these kinds the library lacks and nobody asked for, the oldest first here. */
  getAlbumsPlan(id: string, kinds: string[]): GetAlbumsPlan {
    if (this.hooks.offline()) throw { kind: "network", message: "You're offline." };
    const w = WORLD.find((x) => mbid(x.n) === id);
    if (!w) throw { kind: "notFound", message: "MusicBrainz doesn't know this artist." };
    const k = kinds.length ? kinds : ["studio"];
    const wanted = (r: Rel) =>
      r.secondary?.includes("live")
        ? k.includes("live")
        : r.secondary?.includes("compilation")
          ? k.includes("compilation")
          : r.kind === "album"
            ? k.includes("studio")
            : r.kind === "ep" && k.includes("ep");
    const requested = new Set(this.hooks.requests().filter((q) => q.state !== "failed").map((q) => q.releaseGroupId));
    const albums = w.releases
      .filter((r) => wanted(r) && !requested.has(rgid(r.n)))
      .sort((a, b) => a.year - b.year)
      .map((r) => this.album(w, r));
    return { artist: w.name, artistMbid: mbid(w.n), kinds: k, albums };
  }

  /** The catalogue album with this artist and title (a record on the shelf), whatever the search setting. */
  findAlbum(artist: string, title: string): CatAlbum | null {
    if (this.hooks.offline()) throw { kind: "network", message: "You're offline." };
    for (const w of WORLD) {
      if (fold(w.name) !== fold(artist)) continue;
      const r = w.releases.find((x) => fold(x.title) === fold(title));
      if (r) return this.album(w, r);
    }
    return null;
  }

  /** Releases by section (page order, newest first), only the "Get albums" kinds. */
  private sectioned(w: Who): CatSection[] {
    const kinds = this.hooks.settings().getAlbumsKinds;
    return SECTION_ORDER.filter((k) => sectionWanted(k, kinds))
      .map((k) => ({
        kind: k,
        albums: w.releases
          .filter((r) => sectionOf(r) === k)
          .sort((a, b) => b.year - a.year)
          .map((r) => this.album(w, r)),
      }))
      .filter((s) => s.albums.length > 0);
  }

  /** A featured guest (a `credit:` page): their photo and releases by section; empty offline or for names the catalogue doesn't know. */
  featured(_id: string, name: string): FeaturedArtist {
    const none: FeaturedArtist = { shown: false, mbid: null, photo: null, photoCredit: null, sections: [] };
    if (this.hooks.offline()) return none;
    const w = [...GUESTS, ...WORLD].find((x) => fold(x.name) === fold(name));
    if (!w) return none;
    const pic = photoOf(w.n);
    return { shown: true, mbid: mbid(w.n), photo: pic.photo, photoCredit: pic.credit, sections: this.sectioned(w) };
  }

  artistPage(id: string): CatalogueArtistPage {
    if (this.hooks.offline()) throw { kind: "network", message: "You're offline." };
    const w = WORLD.find((x) => mbid(x.n) === id);
    if (!w) throw { kind: "notFound", message: "MusicBrainz doesn't know this artist." };
    const sections = this.sectioned(w);
    const dismissed = this.dismissed();
    const twin = this.libraryTwin;
    const similar: CatArtist[] = [
      { mbid: mbid(99), name: twin.title, disambiguation: null, kind: "Group", photo: null, libraryId: twin.id },
      ...WORLD.filter((x) => x.n !== w.n && !dismissed.has(mbid(x.n))).map((x) => this.artist(x)),
    ];
    const pic = photoOf(w.n);
    const popular = w.releases
      .filter((r) => !r.secondary?.length && (r.kind === "album" || r.kind === "ep"))
      .slice(0, 5)
      .map((r) => this.album(w, r));
    return {
      artist: { ...this.artist(w), photo: pic.photo, photoCredit: pic.credit },
      popular,
      area: w.area,
      lifeSpan: w.since,
      bio:
        w.n === 5
          ? null
          : `${w.name} ${w.kind === "Group" ? "are a band" : "is a musician"} from ${w.area}, active ${w.since.replace("–", " to ")}. Their records mix field recordings with close, quiet arrangements, and they tour small rooms more than festivals.`,
      bioCredit: w.n === 5 ? null : "Wikipedia",
      bioUrl: w.n === 5 ? null : `https://en.wikipedia.org/wiki/${encodeURIComponent(w.name.replace(/ /g, "_"))}`,
      banner: null,
      sections,
      similar,
      dismissed: dismissed.has(mbid(w.n)),
    };
  }

  albumPage(id: string): CatalogueAlbumPage {
    if (this.hooks.offline()) throw { kind: "network", message: "You're offline." };
    for (const w of WORLD) {
      const r = w.releases.find((x) => rgid(x.n) === id);
      if (!r) continue;
      const q = encodeURIComponent(`${w.name} ${r.title}`);
      return {
        album: this.album(w, r),
        tracks: this.tracks(r),
        released: `${r.year}-04-17`,
        label: "Small Rooms Records",
        links: {
          bandcamp: `https://bandcamp.com/search?q=${q}&item_type=a`,
          qobuz: `https://www.qobuz.com/search?q=${q}`,
          itunes: `https://music.apple.com/search?term=${q}`,
        },
        artistLibraryId: null,
      };
    }
    throw { kind: "notFound", message: "MusicBrainz doesn't know this album." };
  }

  fansAlsoLike(artistId: string): CatArtist[] {
    const s = this.hooks.settings();
    if (!s.fansAlsoLikeCatalogue || this.hooks.offline()) return [];
    const idx = this.fx.artists.findIndex((a) => a.id === artistId);
    // Like Plex's similar artists here, the last few artists have none.
    if (idx < 0 || idx >= this.fx.artists.length - 3) return [];
    const dismissed = this.dismissed();
    return [0, 1, 2]
      .map((d) => WORLD[(idx + d) % WORLD.length])
      .filter((w) => !dismissed.has(mbid(w.n)))
      .map((w) => this.artist(w));
  }

  newToYou(): NewToYou {
    const s = this.hooks.settings();
    if (!s.homeNewToYou) return { shown: false, artists: [] };
    const dismissed = this.dismissed();
    const seeds = this.fx.artists.slice(0, 4).map((a) => a.title);
    return {
      shown: true,
      artists: WORLD.filter((w) => !dismissed.has(mbid(w.n))).map((w, i) => ({
        mbid: mbid(w.n),
        name: w.name,
        because: [seeds[i % seeds.length]],
      })),
    };
  }

  // ---------------------------------------------------------------- Missing

  missingAlbums(): MissingAlbum[] {
    return this.wanted.map((w) => {
      const q = encodeURIComponent(`${w.artist} ${w.title}`);
      return {
        ...w,
        thumb: coverThumb(w.releaseGroupId),
        links: {
          bandcamp: `https://bandcamp.com/search?q=${q}&item_type=a`,
          qobuz: `https://www.qobuz.com/search?q=${q}`,
          itunes: `https://music.apple.com/search?term=${q}`,
        },
        libraryAlbumId: null,
      };
    });
  }

  addMissing(a: WantedAlbum): WantedAlbum[] {
    if (!/^[0-9a-f-]{36}$/i.test(a.releaseGroupId)) throw { kind: "invalid", message: "That album can't be added to Missing." };
    if (!this.wanted.some((x) => x.releaseGroupId === a.releaseGroupId)) this.wanted = [{ ...a, addedAt: Math.floor(Date.now() / 1000) }, ...this.wanted];
    return this.wanted;
  }

  removeMissing(id: string): WantedAlbum[] {
    this.wanted = this.wanted.filter((x) => x.releaseGroupId !== id);
    return this.wanted;
  }

  /** How many albums are on the Missing list (the import overview counts them). */
  get missingCount(): number {
    return this.wanted.length;
  }

  /** The first catalogue artist's id (tests and demos). */
  static artistId(n = 1): string {
    return mbid(n);
  }

  static albumId(n = 11): string {
    return rgid(n);
  }
}
