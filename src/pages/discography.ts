/** Discography page decisions: the type filter, the sort, the release lines and the saved view. */
import { SECTION_LABELS, catalogueKind, plural, releaseTypeLabel } from "../lib/format";
import type { ArtistPageSection, ReleaseSection, Settings } from "../lib/types";
import { discographyChips, newerEntry, type DiscoEntry } from "./artist";

/** A section's id: its release type, or `custom:<name>` for one of the user's own. */
export type SectionId = ReleaseSection | `custom:${string}`;

export function sectionId(s: Pick<ArtistPageSection, "section" | "custom">): SectionId {
  return s.custom ? `custom:${s.custom}` : s.section;
}

export function sectionLabel(s: Pick<ArtistPageSection, "section" | "custom">): string {
  return s.custom ?? SECTION_LABELS[s.section];
}

export type DiscographyType = "all" | SectionId;
export type DiscographySort = "date" | "name";
export type DiscographyView = "list" | "grid";

export const SORT_LABEL: Record<DiscographySort, string> = { date: "Release date", name: "Name" };
export const VIEW_LABEL: Record<DiscographyView, string> = { list: "List", grid: "Grid" };

export interface TypeOption {
  id: DiscographyType;
  label: string;
}

/** All, then one entry per release type the artist has (in the library or to get), in the shelf's chip order. */
export function typeOptions(entries: DiscoEntry[]): TypeOption[] {
  return [{ id: "all", label: "All" }, ...discographyChips(entries).filter((c) => c.id !== "popular").map((c) => ({ id: c.id as SectionId, label: c.label }))];
}

/** A type the artist no longer has falls back to All. */
export function validType(entries: DiscoEntry[], type: DiscographyType | undefined): DiscographyType {
  return type && type !== "all" && entries.some((e) => e.section === type) ? type : "all";
}

/** A to Z by the sort title, numbers in number order. */
export function byName(a: DiscoEntry, b: DiscoEntry): number {
  return (a.album?.sortTitle || a.title).localeCompare(b.album?.sortTitle || b.title, undefined, { numeric: true, sensitivity: "base" });
}

export interface DiscographyGroup {
  section: SectionId;
  label: string;
  entries: DiscoEntry[];
}

/**
 * The releases for a type, sectioned by release type (one section when a
 * type is picked), each in the chosen order (newest first or A to Z, or the
 * other way round). One release in two sections shows once.
 */
export function discographyGroups(entries: DiscoEntry[], type: DiscographyType, sort: DiscographySort, reverse = false): DiscographyGroup[] {
  const order = sort === "name" ? byName : newerEntry;
  const seen = new Set<string>();
  return typeOptions(entries)
    .filter((o) => o.id !== "all" && (type === "all" || o.id === type))
    .map((o) => ({
      section: o.id as SectionId,
      label: o.label,
      entries: entries
        .filter((e) => e.section === o.id && !seen.has(e.key) && !!seen.add(e.key))
        .sort((a, b) => (reverse ? -1 : 1) * order(a, b)),
    }))
    .filter((g) => g.entries.length > 0);
}

/** "Album · 2024 · 12 songs" under a release's title in the list view ("Album · 2024" for one you can get). */
export function releaseLine(e: DiscoEntry): string {
  if (e.album) {
    const a = e.album;
    // A server that sent no count says nothing rather than "0 songs".
    return [releaseTypeLabel(a), a.year ? String(a.year) : null, a.trackCount > 0 ? plural(a.trackCount, "song") : null].filter(Boolean).join(" · ");
  }
  return [e.cat ? catalogueKind(e.cat) : e.sectionLabel, e.year ? String(e.year) : null].filter(Boolean).join(" · ");
}

export interface DiscographyPrefs {
  sort: DiscographySort;
  reverse: boolean;
  view: DiscographyView;
}

export const DISCOGRAPHY_PREFS_DEFAULT: DiscographyPrefs = { sort: "date", reverse: false, view: "list" };

/** Sort and view as Settings keeps them (this device). */
export function discographyPrefs(s: Partial<Pick<Settings, "discographySort" | "discographyReverse" | "discographyView">> | null | undefined): DiscographyPrefs {
  const d = DISCOGRAPHY_PREFS_DEFAULT;
  return {
    sort: s?.discographySort === "name" || s?.discographySort === "date" ? s.discographySort : d.sort,
    reverse: s?.discographyReverse ?? d.reverse,
    view: s?.discographyView === "grid" || s?.discographyView === "list" ? s.discographyView : d.view,
  };
}

/** The Settings change for new sort and view choices. */
export function prefsPatch(p: Partial<DiscographyPrefs>): Partial<Settings> {
  return {
    ...(p.sort ? { discographySort: p.sort } : {}),
    ...(p.reverse != null ? { discographyReverse: p.reverse } : {}),
    ...(p.view ? { discographyView: p.view } : {}),
  };
}
