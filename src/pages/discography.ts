/** Discography page decisions: the type filter, the sort, the release lines and the saved view. */
import { SECTION_LABELS, plural, releaseTypeLabel } from "../lib/format";
import type { Album, ArtistPageSection, ReleaseSection } from "../lib/types";

export type DiscographyType = "all" | ReleaseSection;
export type DiscographySort = "date" | "name";
export type DiscographyView = "list" | "grid";

export const SORT_LABEL: Record<DiscographySort, string> = { date: "Release date", name: "Name" };
export const VIEW_LABEL: Record<DiscographyView, string> = { list: "List", grid: "Grid" };

export interface TypeOption {
  id: DiscographyType;
  label: string;
}

/** All, then one entry per release type the artist has, in the page's section order. */
export function typeOptions(sections: ArtistPageSection[]): TypeOption[] {
  return [{ id: "all", label: "All" }, ...sections.filter((s) => s.albums.length).map((s) => ({ id: s.section, label: SECTION_LABELS[s.section] }))];
}

/** A type the artist no longer has falls back to All. */
export function validType(sections: ArtistPageSection[], type: DiscographyType | undefined): DiscographyType {
  return type && type !== "all" && sections.some((s) => s.section === type && s.albums.length) ? type : "all";
}

/** Newest first: by year, then by when it was added. Releases without a year go last. */
export function byDate(a: Album, b: Album): number {
  return (b.year ?? -1) - (a.year ?? -1) || b.addedAt - a.addedAt;
}

/** A to Z by the sort title, numbers in number order. */
export function byName(a: Album, b: Album): number {
  return (a.sortTitle || a.title).localeCompare(b.sortTitle || b.title, undefined, { numeric: true, sensitivity: "base" });
}

/** The releases for a type, in the chosen order. One release in two sections shows once. */
export function discographyReleases(sections: ArtistPageSection[], type: DiscographyType, sort: DiscographySort): Album[] {
  const seen = new Set<string>();
  const out: Album[] = [];
  for (const s of sections) {
    if (type !== "all" && s.section !== type) continue;
    for (const a of s.albums) {
      if (seen.has(a.id)) continue;
      seen.add(a.id);
      out.push(a);
    }
  }
  return out.sort(sort === "name" ? byName : byDate);
}

/** "Album · 2024 · 12 songs" under a release's title in the list view. */
export function releaseLine(a: Pick<Album, "format" | "subformats" | "year" | "trackCount">): string {
  return [releaseTypeLabel(a), a.year ? String(a.year) : null, plural(a.trackCount, "song")].filter(Boolean).join(" · ");
}

export interface DiscographyPrefs {
  sort: DiscographySort;
  view: DiscographyView;
}

export const DISCOGRAPHY_PREFS_DEFAULT: DiscographyPrefs = { sort: "date", view: "list" };
const PREFS_KEY = "platter.discography.v1"; // gitleaks:allow (localStorage key name, not a secret)

/** Sort and view, remembered on this device. */
export function loadDiscographyPrefs(): DiscographyPrefs {
  const d = DISCOGRAPHY_PREFS_DEFAULT;
  try {
    const p = JSON.parse(localStorage.getItem(PREFS_KEY) ?? "{}") as Partial<DiscographyPrefs>;
    return {
      sort: p.sort === "name" || p.sort === "date" ? p.sort : d.sort,
      view: p.view === "grid" || p.view === "list" ? p.view : d.view,
    };
  } catch {
    return { ...d };
  }
}

export function saveDiscographyPrefs(p: DiscographyPrefs) {
  try {
    localStorage.setItem(PREFS_KEY, JSON.stringify(p));
  } catch {
    /* storage unavailable: this session only */
  }
}
