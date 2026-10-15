/** Artist page decisions, kept apart from the component so they can be tested. */
import { SECTION_LABELS } from "../lib/format";
import type { Album, Artist, ArtistInfo, ArtistPage, ArtistPageSection, BioLink, ReleaseSection } from "../lib/types";

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

/** "48,210 listeners on Last.fm". */
export function listenersLine(listeners: number): string {
  return `${listeners.toLocaleString("en-US")} ${listeners === 1 ? "listener" : "listeners"} on Last.fm`;
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

export type DiscographyFilter = "popular" | ReleaseSection;

export interface Chip {
  id: DiscographyFilter;
  label: string;
}

/** "Popular releases" first, then one chip per release type the artist has, in section order. */
export function discographyChips(sections: ArtistPageSection[]): Chip[] {
  return [{ id: "popular", label: "Popular releases" }, ...sections.map((s) => ({ id: s.section, label: SECTION_LABELS[s.section] }))];
}

export interface DiscographyItem {
  album: Album;
  /** The newest release, marked on its card in the popular row. */
  latest: boolean;
}

function newer(a: Album, b: Album): number {
  return (b.year ?? -1) - (a.year ?? -1) || b.addedAt - a.addedAt;
}

/**
 * Cards for a discography chip. Popular releases: the newest release first,
 * then the most played, `limit` cards in all. A release type: newest first,
 * every release. A chip the artist no longer has falls back to popular
 * releases.
 */
export function discographyItems(
  sections: ArtistPageSection[],
  filter: DiscographyFilter,
  limit: number = POPULAR_RELEASES_N,
): DiscographyItem[] {
  const section = filter === "popular" ? undefined : sections.find((s) => s.section === filter);
  if (section) return [...section.albums].sort(newer).map((album) => ({ album, latest: false }));
  const all = sections.flatMap((s) => s.albums);
  if (!all.length) return [];
  const latest = [...all].sort(newer)[0];
  const rest = all
    .filter((a) => a.id !== latest.id)
    .sort((a, b) => b.viewCount - a.viewCount || newer(a, b))
    .slice(0, Math.max(0, limit - 1));
  return [{ album: latest, latest: true }, ...rest.map((album) => ({ album, latest: false }))];
}

export type ArtistSectionId = "popular" | "discography" | "about" | "fansAlsoLike" | "appearsOn";

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
  },
): ArtistSectionId[] {
  const out: ArtistSectionId[] = [];
  if (page.popular.length) out.push("popular");
  if (page.sections.some((s) => s.albums.length)) out.push("discography");
  if (page.about?.trim() || page.listeners || page.photo || page.plays) out.push("about");
  if (page.similar.length) out.push("fansAlsoLike");
  if (page.appearsOn?.length) out.push("appearsOn");
  return out;
}

/** "1,204 plays in your library". */
export function playsLine(plays: number): string {
  return `${plays.toLocaleString("en-US")} ${plays === 1 ? "play" : "plays"} in your library`;
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
