/** Artist page decisions, kept apart from the component so they can be tested. */
import { SECTION_LABELS } from "../lib/format";
import type { Album, Artist, ArtistInfo, ArtistPage, ArtistPageSection, ReleaseSection } from "../lib/types";

/** Popular songs shown at first, and after "See more". */
export const POPULAR_FIRST = 5;
export const POPULAR_MORE = 10;

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
 * shows with a bio or a Last.fm listener count.
 */
export function artistSections(
  page: Pick<ArtistPage, "popular" | "sections" | "about" | "similar"> & { appearsOn?: Album[]; listeners?: number | null },
): ArtistSectionId[] {
  const out: ArtistSectionId[] = [];
  if (page.popular.length) out.push("popular");
  if (page.sections.some((s) => s.albums.length)) out.push("discography");
  if (page.about?.trim() || page.listeners) out.push("about");
  if (page.similar.length) out.push("fansAlsoLike");
  if (page.appearsOn?.length) out.push("appearsOn");
  return out;
}

/** "1,204 plays in your library". */
export function playsLine(plays: number): string {
  return `${plays.toLocaleString("en-US")} ${plays === 1 ? "play" : "plays"} in your library`;
}
