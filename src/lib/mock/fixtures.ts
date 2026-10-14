/**
 * Invented fixture library for the mock backend. Every name here is made up.
 * Generation is deterministic (seeded PRNG) so screenshots and tests are stable.
 */
import type { Album, Artist, Playlist, ReleaseSection, Track } from "../types";

export function mulberry32(seed: number): () => number {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

export function hashString(s: string): number {
  let h = 2166136261;
  for (let i = 0; i < s.length; i++) {
    h ^= s.charCodeAt(i);
    h = Math.imul(h, 16777619);
  }
  return h >>> 0;
}

const ARTIST_NAMES = [
  "Copperline Choir",
  "Odessa Wren",
  "The Lantern Tides",
  "Kesto Marrow",
  "Saltglass",
  "Ivo Brightwater",
  "Neon Orchard",
  "Halcyon Drift Unit",
  "Rue Calloway",
  "Moth & Meridian",
  "Tamsin Vale",
  "Gravel Psalms",
];

/** Featured artists with no library entry. One has a comma in its name. */
export const GUEST_NAMES = ["Juno Ashby", "Pell Rivers", "Tyler, The Maker", "Wren Okoro"];
/** Names with separators the core would have learned (kept whole in splits). */
export const LEARNED_NAMES = ["Tyler, The Maker"];

const GENRES = ["Indie Rock", "Dream Pop", "Hip-Hop", "Electronic", "Folk", "Soul", "Ambient", "Post-Punk"];

const WORDS_A = [
  "Amber", "Paper", "Velvet", "Hollow", "Silver", "Quiet", "Northern", "Broken", "Golden", "Distant",
  "Electric", "Winter", "Midnight", "Static", "Copper", "Glass", "Borrowed", "Faded", "Wild", "Slow",
];
const WORDS_B = [
  "Rooms", "Signals", "Harbors", "Gardens", "Machines", "Letters", "Rivers", "Satellites", "Mirrors",
  "Lanterns", "Weather", "Highways", "Ghosts", "Orchards", "Tides", "Engines", "Houses", "Stars",
];
const TRACK_WORDS = [
  "Undertow", "Glow", "Parallel", "Small Hours", "Overpass", "Kite", "Fever", "Signal Fire", "Lowlight",
  "Arcade", "Saltwater", "Northbound", "Vapor", "Ribbon", "Daybreak", "Static Bloom", "Cinder",
  "Postcard", "Orbit", "Hush", "Ferris", "Ivory", "Tremor", "Afterglow", "Windows", "Driftwood",
  "Paper Moon", "Echo Park", "Satellite", "Lull", "Marigold", "Neon Rain", "Stillwater", "Haze",
  "Rust", "Velour", "Gravity", "Wildfire", "Second Wind", "Blue Hour",
];

type Plan = {
  section: ReleaseSection;
  format: string | null;
  subformats: string[];
  tracks: [number, number];
  title?: (r: () => number) => string;
};

const PLANS: Record<string, Plan> = {
  album: { section: "albums", format: "album", subformats: [], tracks: [11, 16] },
  ep: { section: "singlesEps", format: "ep", subformats: [], tracks: [4, 7] },
  single: { section: "singlesEps", format: "single", subformats: [], tracks: [1, 2] },
  live: {
    section: "live",
    format: "album",
    subformats: ["live"],
    tracks: [13, 18],
    title: (r) => `Live at the ${pick(r, ["Foundry", "Palladian", "Old Mill", "Harbor Hall", "Glasshouse"])}`,
  },
  soundtrack: {
    section: "soundtracks",
    format: "album",
    subformats: ["soundtrack"],
    tracks: [12, 14],
    title: () => "Lanternfall (Original Score)",
  },
  compilation: {
    section: "compilations",
    format: "album",
    subformats: ["compilation"],
    tracks: [12, 14],
    title: () => "Collected B-Sides 2014–2022",
  },
  remix: {
    section: "remixes",
    format: "ep",
    subformats: ["remix"],
    tracks: [4, 6],
    title: (r) => `${pick(r, WORDS_A)} ${pick(r, WORDS_B)} (Remixes)`,
  },
  demo: {
    section: "demos",
    format: "album",
    subformats: ["demo"],
    tracks: [6, 9],
    title: () => "Basement Demos",
  },
  // Untagged unreleased builds: no format at all → listed under Albums.
  build: {
    section: "albums",
    format: null,
    subformats: [],
    tracks: [8, 12],
    title: (r) => `Untitled ${pick(r, ["Build", "Sessions", "Tape"])} (v${1 + Math.floor(r() * 4)})`,
  },
};

// Release plan per artist (index-aligned with ARTIST_NAMES). ~60 releases.
const ARTIST_PLANS: string[][] = [
  ["album", "album", "ep", "live", "single", "build"],
  ["album", "album", "album", "single", "remix"],
  ["album", "ep", "single", "live", "compilation"],
  ["album", "album", "build", "build", "ep"],
  ["album", "album", "soundtrack", "single"],
  ["album", "ep", "ep", "single", "demo"],
  ["album", "album", "album", "remix", "single"],
  ["album", "live", "ep", "single", "build"],
  ["album", "album", "ep", "single", "single"],
  ["album", "album", "demo", "ep", "live"],
  ["album", "album", "single", "ep", "compilation"],
  ["album", "ep", "single", "build", "album"],
];

type Quality = { codec: string; container: string; bitrate: number; bitDepth: number | null; sampleRate: number };
const QUALITIES: Quality[] = [
  { codec: "flac", container: "flac", bitrate: 1011, bitDepth: 16, sampleRate: 44100 },
  { codec: "flac", container: "flac", bitrate: 3200, bitDepth: 24, sampleRate: 96000 },
  { codec: "flac", container: "flac", bitrate: 1650, bitDepth: 24, sampleRate: 48000 },
  { codec: "mp3", container: "mp3", bitrate: 320, bitDepth: null, sampleRate: 44100 },
  { codec: "aac", container: "mp4", bitrate: 256, bitDepth: null, sampleRate: 44100 },
];

function pick<T>(r: () => number, arr: readonly T[]): T {
  return arr[Math.floor(r() * arr.length)];
}

function sortTitle(t: string): string {
  return t.replace(/^(the|a|an)\s+/i, "");
}

export interface Fixtures {
  artists: Artist[];
  albums: Album[];
  tracks: Track[];
  playlists: Playlist[];
  playlistTracks: Map<string, string[]>;
  tracksByAlbum: Map<string, Track[]>;
}

/** Track IDs the mock treats as unplayable (to demo "Skipped: file unavailable"). */
export const BROKEN_TRACK_TITLE_SUFFIX = " (corrupt rip)";

const NOW = Date.UTC(2026, 9, 13, 12, 0, 0) / 1000;

export function buildFixtures(seed = 7): Fixtures {
  const r = mulberry32(seed);
  const artists: Artist[] = [];
  const albums: Album[] = [];
  const tracks: Track[] = [];
  const tracksByAlbum = new Map<string, Track[]>();
  let albumSeq = 1000;
  let trackSeq = 10000;
  const usedTitles = new Set<string>();

  ARTIST_NAMES.forEach((name, ai) => {
    const artistId = String(100 + ai);
    artists.push({
      id: artistId,
      title: name,
      sortTitle: sortTitle(name),
      thumb: `/library/metadata/${artistId}/thumb/1`,
      // Every other artist has background art, so both artist page headers show up.
      art: ai % 2 === 0 ? `/library/metadata/${artistId}/art/1` : null,
      viewCount: Math.floor(r() * 900),
    });
    const genre = GENRES[ai % GENRES.length];
    let year = 2008 + Math.floor(r() * 6);

    for (const planKey of ARTIST_PLANS[ai]) {
      const plan = PLANS[planKey];
      const albumId = String(albumSeq++);
      let title = plan.title ? plan.title(r) : `${pick(r, WORDS_A)} ${pick(r, WORDS_B)}`;
      if (planKey === "single") title = pick(r, TRACK_WORDS);
      while (usedTitles.has(title)) title = `${title} II`;
      usedTitles.add(title);
      year += 1 + Math.floor(r() * 2);
      const q = planKey === "build" ? QUALITIES[3] : pick(r, QUALITIES);
      const n = plan.tracks[0] + Math.floor(r() * (plan.tracks[1] - plan.tracks[0] + 1));
      const lastViewed = r() < 0.6 ? NOW - Math.floor(r() * 60 * 86400) : null;
      const album: Album = {
        id: albumId,
        title,
        sortTitle: sortTitle(title),
        artistId,
        artistTitle: name,
        year: planKey === "build" ? null : year,
        thumb: `/library/metadata/${albumId}/thumb/${1 + Math.floor(r() * 9)}`,
        format: plan.format,
        subformats: plan.subformats,
        section: plan.section,
        genres: [genre],
        trackCount: n,
        addedAt: NOW - Math.floor(r() * 400 * 86400),
        viewCount: lastViewed ? Math.floor(r() * 120) : 0,
        lastViewedAt: lastViewed,
        // About half the albums carry a server-computed tint; the rest are
        // tinted client-side from the artwork.
        tint: r() < 0.5 ? null : tintFor(albumId),
      };
      albums.push(album);

      const list: Track[] = [];
      for (let i = 0; i < n; i++) {
        const trackId = String(trackSeq++);
        let ttitle = n <= 2 && i === 0 ? title : pick(r, TRACK_WORDS);
        if (planKey === "build" && i === 3) ttitle += BROKEN_TRACK_TITLE_SUFFIX;
        if (planKey === "remix") ttitle += ` (${pick(r, ARTIST_NAMES)} Remix)`;
        const feat = r() < 0.12 ? `${name}, ${pick(r, ARTIST_NAMES.filter((x) => x !== name))}` : null;
        const tq = planKey === "build" && r() < 0.3 ? QUALITIES[4] : q;
        const t: Track = {
          id: trackId,
          title: ttitle,
          albumId,
          albumTitle: title,
          artistId,
          artistTitle: name,
          trackArtist: feat,
          // Live albums come on two discs, numbered from 1 on each.
          trackNumber: planKey === "live" && i >= Math.ceil(n / 2) ? i + 1 - Math.ceil(n / 2) : i + 1,
          discNumber: planKey === "live" && i >= Math.ceil(n / 2) ? 2 : 1,
          durationMs: (95 + Math.floor(r() * 240)) * 1000 + Math.floor(r() * 1000),
          thumb: album.thumb,
          partKey: `/library/parts/${trackId}/1/file.${tq.container === "mp4" ? "m4a" : tq.container}`,
          codec: tq.codec,
          container: tq.container,
          bitrate: tq.bitrate,
          bitDepth: tq.bitDepth,
          sampleRate: tq.sampleRate,
          userRating: r() < 0.08 ? 10 : null,
          viewCount: Math.floor(r() * r() * 300),
          lastViewedAt: r() < 0.4 ? NOW - Math.floor(r() * 30 * 86400) : null,
        };
        list.push(t);
        tracks.push(t);
      }
      tracksByAlbum.set(albumId, list);
    }
  });

  // Playlists
  const playlistDefs: [string, boolean, number][] = [
    ["Late Night Drive", false, 24],
    ["Sunday Coffee", false, 18],
    ["Workout Rotation", false, 32],
    ["Most Played", true, 50],
    ["Rainy Afternoons", false, 15],
    ["Unreleased Favorites", false, 12],
  ];
  const playlists: Playlist[] = [];
  const playlistTracks = new Map<string, string[]>();
  playlistDefs.forEach(([title, smart, count], i) => {
    const id = String(500 + i);
    let ids: string[];
    if (smart) {
      ids = [...tracks].sort((a, b) => b.viewCount - a.viewCount).slice(0, count).map((t) => t.id);
    } else if (title.startsWith("Unreleased")) {
      ids = albums
        .filter((a) => a.format === null)
        .flatMap((a) => tracksByAlbum.get(a.id)!.map((t) => t.id))
        .slice(0, count);
    } else {
      const set = new Set<string>();
      while (set.size < count) set.add(pick(r, tracks).id);
      ids = [...set];
    }
    playlistTracks.set(id, ids);
    const dur = ids.reduce((s, tid) => s + (tracks.find((t) => t.id === tid)?.durationMs ?? 0), 0);
    playlists.push({
      id,
      title,
      smart,
      trackCount: ids.length,
      durationMs: dur,
      thumb: `/playlists/${id}/composite/1`,
    });
  });

  // Plex rating counts on a bit under half the released tracks, like a
  // real library. Derived from the id so the random stream above is untouched.
  const unreleased = new Set(albums.filter((a) => a.format === null).map((a) => a.id));
  for (const t of tracks) {
    const h = hashString(`count:${t.id}`);
    if (!unreleased.has(t.albumId) && h % 100 < 44) t.ratingCount = 500 + (h % 2_000_000);
  }

  // Featured guests with no albums of their own (credited-artist pages), on
  // a few released songs. Also derived from the id.
  for (const t of tracks) {
    if (t.trackArtist || unreleased.has(t.albumId)) continue;
    const h = hashString(`guest:${t.id}`);
    if (h % 100 < 6) t.trackArtist = `${t.artistTitle} feat. ${GUEST_NAMES[h % GUEST_NAMES.length]}`;
  }

  return { artists, albums, tracks, playlists, playlistTracks, tracksByAlbum };
}

function tintFor(id: string): string {
  // Deliberately dark, saturated tints like a server palette would give.
  const h = hashString(id) % 360;
  return hslToHex(h, 45, 28);
}

export function hslToHex(h: number, s: number, l: number): string {
  s /= 100;
  l /= 100;
  const k = (n: number) => (n + h / 30) % 12;
  const a = s * Math.min(l, 1 - l);
  const f = (n: number) => l - a * Math.max(-1, Math.min(k(n) - 3, Math.min(9 - k(n), 1)));
  const to = (x: number) => Math.round(x * 255).toString(16).padStart(2, "0");
  return `${to(f(0))}${to(f(8))}${to(f(4))}`;
}
