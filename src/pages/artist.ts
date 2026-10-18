/** Artist page decisions, kept apart from the component so they can be tested. */
import { SECTION_LABELS } from "../lib/format";
import type { Album, Artist, ArtistInfo, ArtistPage, ArtistPageSection, BioLink, CatAlbum, CatSection, InfoRelated, LidarrMissingItem, ReleaseSection } from "../lib/types";
import { sectionId, sectionLabel, type SectionId } from "./discography";

/** Popular songs shown at first, and after "See more". */
export const POPULAR_FIRST = 5;
export const POPULAR_MORE = 10;
/** How long Popular waits for the global list before showing plays order. */
export const POPULAR_WAIT_MS = 1500;

/** Cards in the "Popular releases" row. */
export const POPULAR_RELEASES_N = 10;

/**
 * Banner header when Plex has background art for the artist (or an outside
 * source has one and Plex doesn't), else the round photo.
 */
export function headerVariant(artist: Pick<Artist, "art">, fallbackBanner?: string | null): "banner" | "photo" {
  return artist.art || fallbackBanner ? "banner" : "photo";
}

/** The banner image: Plex art always wins. */
export function bannerPath(artist: Pick<Artist, "art">, info?: Pick<ArtistInfo, "banner"> | null): string | null {
  return artist.art ?? info?.banner ?? null;
}

/** The artist photo: Plex first, then an outside source's. */
export function photoPath(artist: Pick<Artist, "thumb">, info?: Pick<ArtistInfo, "photo"> | null): string | null {
  return artist.thumb ?? info?.photo ?? null;
}

/** The bio shown: the outside sources' pick (which already weighs Plex), else Plex's. */
export function aboutText(plex: string | null, info?: Pick<ArtistInfo, "bio"> | null): string | null {
  return (info?.bio ?? plex)?.trim() || null;
}

/** Display name of where listener numbers come from. */
export function listenersSource(from: ArtistInfo["listenersFrom"] | undefined): string {
  return from === "listenbrainz" ? "ListenBrainz" : "Last.fm";
}

/** "48,210 listeners on Last.fm" (or on ListenBrainz when Last.fm has none). */
export function listenersLine(listeners: number, from?: ArtistInfo["listenersFrom"]): string {
  return `${listeners.toLocaleString("en-US")} ${listeners === 1 ? "listener" : "listeners"} on ${listenersSource(from)}`;
}

/** Bio paragraphs, for the About dialog. */
export function bioParagraphs(bio: string): string[] {
  return bio
    .split(/\n\s*\n|\n/)
    .map((p) => p.trim())
    .filter(Boolean);
}

/** Origin and years active rows for the About dialog, leaving out unknowns. */
export function aboutFacts(info: Pick<ArtistInfo, "origin" | "yearsActive"> | null | undefined): { label: string; value: string }[] {
  const out: { label: string; value: string }[] = [];
  if (info?.origin) out.push({ label: "From", value: info.origin });
  if (info?.yearsActive) out.push({ label: "Years active", value: info.yearsActive });
  return out;
}

/** Related artists grouped by relation ("Member of", "Collaboration"), in first-seen order. */
export function relatedGroups(related: InfoRelated[]): [string, InfoRelated[]][] {
  const groups = new Map<string, InfoRelated[]>();
  for (const r of related) {
    const g = groups.get(r.relation);
    if (g) g.push(r);
    else groups.set(r.relation, [r]);
  }
  return [...groups];
}

export type DiscographyFilter = "popular" | SectionId;

export interface Chip {
  id: DiscographyFilter;
  label: string;
}

/**
 * One release on an artist's discography: one in the library (`album`), or
 * one you can get (`cat`, from the catalogue, with Get).
 */
export interface DiscoEntry {
  key: string;
  section: SectionId;
  sectionLabel: string;
  title: string;
  year: number | null;
  /** Plays for a library release; for one you can get, how popular it is (higher is more). */
  plays: number;
  addedAt: number;
  album?: Album;
  cat?: CatAlbum;
  /** A release Lidarr can get (its card keeps Lidarr's request state). */
  missing?: LidarrMissingItem;
}

/** The library's releases, by section. */
export function libraryEntries(sections: ArtistPageSection[]): DiscoEntry[] {
  return sections.flatMap((s) =>
    s.albums.map((album) => ({
      key: album.id,
      section: sectionId(s),
      sectionLabel: sectionLabel(s),
      title: album.title,
      year: album.year,
      plays: album.viewCount,
      addedAt: album.addedAt,
      album,
    })),
  );
}

function catEntry(cat: CatAlbum, section: ReleaseSection, plays = 0): DiscoEntry {
  return { key: `rg:${cat.releaseGroupId}`, section, sectionLabel: SECTION_LABELS[section], title: cat.title, year: cat.year, plays, addedAt: 0, cat };
}

const isSection = (k: string): k is ReleaseSection => k in SECTION_LABELS;

/**
 * Catalogue releases by section. `popular` (most popular first) ranks them
 * for Popular releases; `skip` leaves out library albums already shown.
 */
export function catalogueEntries(sections: CatSection[], popular: CatAlbum[] = [], skip: ReadonlySet<string> = new Set()): DiscoEntry[] {
  const rank = new Map(popular.map((a, i) => [a.releaseGroupId, popular.length - i]));
  return sections.flatMap((s) =>
    isSection(s.kind)
      ? s.albums.filter((a) => !a.libraryAlbumId || !skip.has(a.libraryAlbumId)).map((a) => catEntry(a, s.kind as ReleaseSection, rank.get(a.releaseGroupId) ?? 0))
      : [],
  );
}

/** Releases Lidarr can get for a library artist, as catalogue releases. Older cores leave the section out: albums and EPs only then. */
export function missingEntries(items: LidarrMissingItem[], artist: string): DiscoEntry[] {
  return items.map((i) => ({
    ...catEntry(
      {
        releaseGroupId: i.releaseGroupId,
        title: i.title,
        artist,
        artistMbid: null,
        kind: i.kind,
        secondary: i.secondary ?? [],
        year: i.year,
        thumb: i.thumb,
        libraryAlbumId: null,
        request: i.request,
        refusal: i.refusal ?? null,
        inMissing: false,
      },
      i.section ?? (i.kind === "album" ? "albums" : "singlesEps"),
    ),
    missing: i,
  }));
}

/** Chips in this order first; the artist's other release types follow in page order. */
const CHIP_ORDER: SectionId[] = ["albums", "singlesEps", "compilations"];

/** "Popular releases" first, then one chip per release type the artist has (in the library or to get). */
export function discographyChips(entries: DiscoEntry[]): Chip[] {
  const seen = new Map<SectionId, string>();
  for (const e of entries) if (!seen.has(e.section)) seen.set(e.section, e.sectionLabel);
  const ids = [...CHIP_ORDER.filter((k) => seen.has(k)), ...[...seen.keys()].filter((k) => !CHIP_ORDER.includes(k))];
  return [{ id: "popular", label: "Popular releases" }, ...ids.map((id) => ({ id, label: seen.get(id)! }))];
}

/** The chip that's on: the one asked for, or Popular releases when the artist has none of that type. */
export function activeChip(chips: Chip[], wanted: string | null | undefined): DiscographyFilter {
  return chips.find((c) => c.id === wanted)?.id ?? "popular";
}

export interface DiscographyItem {
  entry: DiscoEntry;
  /** The newest release, marked on its card in the popular row. */
  latest: boolean;
}

/** Newest first: by year, then by when it was added. Releases without a year go last. */
export function newerEntry(a: DiscoEntry, b: DiscoEntry): number {
  return (b.year ?? -1) - (a.year ?? -1) || b.addedAt - a.addedAt;
}

/**
 * Cards for a discography chip. Popular releases: the newest release first,
 * then the most played (library releases before ones you can get), `limit`
 * cards in all. A release type: newest first, every release. A chip the
 * artist no longer has falls back to popular releases. One release in two
 * sections shows once.
 */
export function discographyItems(entries: DiscoEntry[], filter: DiscographyFilter, limit: number = POPULAR_RELEASES_N): DiscographyItem[] {
  const seen = new Set<string>();
  const unique = entries.filter((e) => !seen.has(e.key) && !!seen.add(e.key));
  const typed = filter === "popular" ? [] : unique.filter((e) => e.section === filter);
  if (typed.length) return typed.sort(newerEntry).map((entry) => ({ entry, latest: false }));
  if (!unique.length) return [];
  const latest = [...unique].sort(newerEntry)[0];
  const rest = unique
    .filter((e) => e !== latest)
    .sort((a, b) => Number(!!a.cat) - Number(!!b.cat) || b.plays - a.plays || newerEntry(a, b))
    .slice(0, Math.max(0, limit - 1));
  return [{ entry: latest, latest: true }, ...rest.map((entry) => ({ entry, latest: false }))];
}

export type ArtistSectionId = "popular" | "eras" | "discography" | "about" | "fansAlsoLike" | "appearsOn";

/**
 * Sections in page order, leaving out the ones with nothing to show. About
 * shows with a bio, a Last.fm listener count, a photo or plays (without a
 * bio it says so).
 */
export function artistSections(
  page: Pick<ArtistPage, "popular" | "sections" | "about" | "similar"> & {
    appearsOn?: Album[];
    listeners?: number | null;
    photo?: string | null;
    plays?: number;
    /** Era cards to show (none when hidden or fewer than two). */
    eras?: unknown[];
    /** Releases you can get, which show in the discography too. */
    catalogue?: number;
  },
): ArtistSectionId[] {
  const out: ArtistSectionId[] = [];
  if (page.popular.length) out.push("popular");
  if ((page.eras?.length ?? 0) >= 2) out.push("eras");
  if (page.sections.some((s) => s.albums.length) || page.catalogue) out.push("discography");
  if (page.about?.trim() || page.listeners || page.photo || page.plays) out.push("about");
  if (page.similar.length) out.push("fansAlsoLike");
  if (page.appearsOn?.length) out.push("appearsOn");
  return out;
}

/**
 * "1,204 plays in your library", or "8,043 plays" when the count also takes
 * in imported listening history (so it matches the stats strip below).
 */
export function playsLine(plays: number, withHistory = false): string {
  const n = `${plays.toLocaleString("en-US")} ${plays === 1 ? "play" : "plays"}`;
  return withHistory ? n : `${n} in your library`;
}

/** Said in the About card and dialog when no source has a bio. */
export const NO_BIO = "No biography available.";

/**
 * The About dialog's photos: the gallery the core put together, else the
 * one photo there is (Plex's, or a source's), else none.
 */
export function galleryPaths(artist: Pick<Artist, "thumb">, info?: Pick<ArtistInfo, "gallery" | "photo"> | null): string[] {
  if (info?.gallery?.length) return info.gallery;
  const one = photoPath(artist, info);
  return one ? [one] : [];
}

/** A run of bio text, linked to a library artist or not. */
export interface BioSegment {
  text: string;
  artistId?: string;
}

const isWordChar = (c: string | undefined) => !!c && /[\p{L}\p{N}]/u.test(c);

/**
 * A bio paragraph split so the library artists it names can be linked, each
 * only at its first mention in the whole bio (`used` carries that across
 * paragraphs). Matches whole words only.
 */
export function bioSegments(text: string, links: BioLink[], used: Set<string>): BioSegment[] {
  const out: BioSegment[] = [];
  let rest = text;
  for (;;) {
    let best: { at: number; link: BioLink } | null = null;
    for (const link of links) {
      if (used.has(link.artistId) || !link.text) continue;
      let at = rest.indexOf(link.text);
      while (at >= 0 && (isWordChar(rest[at - 1]) || isWordChar(rest[at + link.text.length]))) at = rest.indexOf(link.text, at + 1);
      if (at >= 0 && (!best || at < best.at || (at === best.at && link.text.length > best.link.text.length))) best = { at, link };
    }
    if (!best) break;
    if (best.at > 0) out.push({ text: rest.slice(0, best.at) });
    out.push({ text: best.link.text, artistId: best.link.artistId });
    used.add(best.link.artistId);
    rest = rest.slice(best.at + best.link.text.length);
  }
  if (rest) out.push({ text: rest });
  return out;
}

/** "48,210" for the About dialog's big numbers. */
export const bigNumber = (n: number) => n.toLocaleString("en-US");
