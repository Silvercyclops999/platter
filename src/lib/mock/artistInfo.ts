/**
 * Made-up outside artist info for the mock backend: longer bios, origins,
 * members, Last.fm style numbers and source credits. Follows the core's
 * rules (Plex first, bio order, art only where Plex has none, credits for
 * what's shown) so the About card and dialog can be checked in a browser.
 * Nothing here comes from a real service.
 */
import type { Artist, ArtistInfo, InfoCredit, InfoKeyState, InfoKeysStatus, KeyedInfoSource, Settings } from "../types";

const TOWNS = ["Harbourside", "Millbrook", "North Fenwick", "Calder Vale", "Port Ellery", "Ashgrove"];
const GENRE_TAGS = [
  ["indie rock", "shoegaze", "dream pop"],
  ["folk", "singer-songwriter", "acoustic"],
  ["electronic", "ambient", "downtempo"],
  ["post-punk", "new wave"],
  ["jazz", "soul", "neo-soul"],
];
const FIRST = ["Mara", "Jonas", "Ilse", "Tobin", "Petra", "Rafe", "Odile", "Casimir"];
const LAST = ["Quill", "Hollis", "Brandt", "Okafor", "Lund", "Vasquez", "Merrin"];

/** The longer "encyclopedia" bio a Wikipedia lead section stands in for. */
function longBio(name: string, i: number, town: string, start: number): string {
  return [
    `${name} is a music act formed in ${town} in ${start}. Over the years the line-up has changed a few times, but the core of the sound has stayed the same: patient songs, close harmonies and records made mostly at home.`,
    `The early releases came out on small labels and sold at shows. A run of singles in the following years found a wider audience on college radio, and a self-released album later reached the independent charts.`,
    i % 2 === 0
      ? `Since then the band has kept a steady pace of a record every two or three years, with demos and alternate takes released in between. Several members also play in side projects.`
      : `Recent work leans further into long instrumental passages. The act still tours, mostly in small rooms, and is known for reworking older songs on stage.`,
  ].join("\n\n");
}

export interface MockInfoKeys {
  keys: Partial<Record<KeyedInfoSource, string>>;
  rejected: Set<KeyedInfoSource>;
}

/** Keys starting with "bad" are refused, like a wrong key would be. */
export function mockKeyState(state: MockInfoKeys, source: KeyedInfoSource): InfoKeyState {
  return { saved: !!state.keys[source], rejected: state.rejected.has(source) };
}

export function mockKeysStatus(state: MockInfoKeys): InfoKeysStatus {
  return {
    lastfm: mockKeyState(state, "lastfm"),
    fanart: mockKeyState(state, "fanart"),
    audiodb: mockKeyState(state, "audiodb"),
    listenbrainz: mockKeyState(state, "listenbrainz"),
  };
}

export function mockSetKey(state: MockInfoKeys, source: KeyedInfoSource, key: string | null): InfoKeyState {
  const k = key?.trim();
  state.rejected.delete(source);
  if (k) {
    state.keys[source] = k;
    if (k.toLowerCase().startsWith("bad")) state.rejected.add(source);
  } else {
    delete state.keys[source];
  }
  return mockKeyState(state, source);
}

const usable = (keys: MockInfoKeys, src: KeyedInfoSource, on: boolean) =>
  on && !!keys.keys[src] && !keys.rejected.has(src);

/**
 * Info for one fixture artist. `fetched` false gives the Plex-only answer
 * the core returns before its first background fetch lands.
 */
export function mockArtistInfo(
  artist: Artist,
  idx: number,
  plexSummary: string | null,
  library: Artist[],
  settings: Settings,
  keys: MockInfoKeys,
  fetched: boolean,
): ArtistInfo {
  const wikiOn = fetched && settings.infoWikipedia;
  const lastfmOn = fetched && usable(keys, "lastfm", settings.infoLastfm);
  const fanartOn = fetched && usable(keys, "fanart", settings.infoFanart);
  const audiodbOn = fetched && usable(keys, "audiodb", settings.infoAudiodb);

  // Every fourth artist isn't on Wikipedia at all.
  const onWiki = idx % 4 !== 3;
  const town = TOWNS[idx % TOWNS.length];
  const start = 1988 + ((idx * 7) % 30);
  const ended = idx % 5 === 4 ? start + 9 : null;
  const isBand = idx % 3 !== 1;
  const wikiBio = wikiOn && onWiki ? longBio(artist.title, idx, town, start) : null;
  // A band's members: two invented people plus, for some, another artist in the library.
  const other = library[(idx + 3) % library.length];
  const members =
    wikiOn && onWiki && isBand
      ? [
          { name: `${FIRST[idx % FIRST.length]} ${LAST[idx % LAST.length]}`, artistId: null },
          { name: `${FIRST[(idx + 3) % FIRST.length]} ${LAST[(idx + 2) % LAST.length]}`, artistId: null },
          ...(idx % 2 === 0 && other && other.id !== artist.id ? [{ name: other.title, artistId: other.id }] : []),
        ]
      : [];
  const lastfmBio = lastfmOn ? `${artist.title} is an act from ${town}. Their music is often described as warm and unhurried.` : null;
  const audiodbBio = audiodbOn ? `${artist.title} formed in ${town}. A short bio from a fan-made database.` : null;

  // Bio order: Plex when over 300 characters, else the first source with more.
  const plex = plexSummary?.trim() || null;
  let bio = plex;
  let bioSource: ArtistInfo["bioSource"] = plex ? "plex" : null;
  if (!plex || plex.length <= 300) {
    const pick = (
      [
        [wikiBio, "wikipedia"],
        [lastfmBio, "lastfm"],
        [audiodbBio, "audiodb"],
      ] as const
    ).find(([t]) => t && t.length > (plex?.length ?? 0));
    if (pick) {
      bio = pick[0];
      bioSource = pick[1];
    }
  }

  const slug = encodeURIComponent(artist.title.replace(/ /g, "_"));
  const origin = wikiOn && onWiki ? town : null;
  const yearsActive = wikiOn && onWiki ? (ended ? `${start}–${ended}` : `${start}–present`) : null;
  const listeners = lastfmOn ? 1200 + ((idx * 7919) % 480_000) : null;
  const tags = lastfmOn ? GENRE_TAGS[idx % GENRE_TAGS.length] : [];
  const banner = fanartOn && !artist.art ? `/ext/fanart/${artist.id}/artistbackground` : null;
  const photo = !artist.thumb && fanartOn ? `/ext/fanart/${artist.id}/artistthumb` : null;
  // Generated art makes a poor logo, so the mock has none.
  const logo: string | null = null;

  const credits: InfoCredit[] = [];
  if (bioSource === "wikipedia")
    credits.push({
      source: "wikipedia",
      text: "From Wikipedia",
      url: `https://en.wikipedia.org/wiki/${slug}`,
      licence: "CC BY-SA 4.0",
      licenceUrl: "https://creativecommons.org/licenses/by-sa/4.0/",
    });
  if (origin || yearsActive || members.length)
    credits.push({
      source: "wikidata",
      text: "Facts from Wikidata",
      url: `https://www.wikidata.org/wiki/Q${1000 + idx}`,
      licence: "CC0",
      licenceUrl: "https://creativecommons.org/publicdomain/zero/1.0/",
    });
  if (lastfmOn)
    credits.push({
      source: "lastfm",
      text: "Data from Last.fm",
      url: `https://www.last.fm/music/${encodeURIComponent(artist.title)}`,
      licence: bioSource === "lastfm" ? "CC BY-SA" : null,
      licenceUrl: bioSource === "lastfm" ? "https://creativecommons.org/licenses/by-sa/3.0/" : null,
    });
  if (banner || photo || logo)
    credits.push({ source: "fanart", text: "Images from Fanart.tv", url: "https://fanart.tv", licence: null, licenceUrl: null });
  if (bioSource === "audiodb")
    credits.push({ source: "audiodb", text: "From TheAudioDB", url: "https://www.theaudiodb.com", licence: null, licenceUrl: null });

  return {
    id: artist.id,
    bio,
    bioSource,
    origin,
    yearsActive,
    members,
    tags,
    listeners,
    scrobbles: listeners ? listeners * 23 : null,
    photo,
    banner,
    logo,
    credits,
  };
}
