/** Artist page decisions, kept apart from the component so they can be tested. */
import { SECTION_LABELS } from "../lib/format";
import type { Album, Artist, ArtistPage, ArtistPageSection, ReleaseSection } from "../lib/types";

/** Popular songs shown at first, and after "See more". */
export const POPULAR_FIRST = 5;
export const POPULAR_MORE = 10;

/** Cards in the "Popular releases" row. */
export const POPULAR_RELEASES_N = 10;

/** Banner header when Plex has background art for the artist, else the round photo. */
export function headerVariant(artist: Pick<Artist, "art">): "banner" | "photo" {
  return artist.art ? "banner" : "photo";
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

export type ArtistSectionId = "popular" | "discography" | "about" | "fansAlsoLike";

/** Sections in page order, leaving out the ones with nothing to show. */
export function artistSections(page: Pick<ArtistPage, "popular" | "sections" | "about" | "similar">): ArtistSectionId[] {
  const out: ArtistSectionId[] = [];
  if (page.popular.length) out.push("popular");
  if (page.sections.some((s) => s.albums.length)) out.push("discography");
  if (page.about?.trim()) out.push("about");
  if (page.similar.length) out.push("fansAlsoLike");
  return out;
}

/** "1,204 plays in your library". */
export function playsLine(plays: number): string {
  return `${plays.toLocaleString("en-US")} ${plays === 1 ? "play" : "plays"} in your library`;
}
