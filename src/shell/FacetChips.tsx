/**
 * The library's filter chips: kind of release, files (lossless, hi-res),
 * decade and added recently, for albums, artists and songs. Type and decade
 * open a short menu; the others toggle. Chosen chips are "on" (brass in
 * Platter's looks).
 */
import { ChevronDown, X } from "lucide-react";
import { useMemo } from "react";
import { openContextMenu } from "../components/ContextMenu";
import { useRovingRow } from "../components/roving";
import {
  FORMAT_LABEL,
  TYPE_LABEL,
  albumFacts,
  anyMatches,
  chipOptions,
  chipsOn,
  decadeLabel,
  matchesChips,
  pressFacet,
  trackQuality,
  useLibraryChips,
  type ChipFacts,
  type ChipOptions,
  type ChipView,
  type LibChips,
} from "../lib/libraryChips";
import type { LibraryItem, Track } from "../lib/types";
import type { LibEntry } from "../stores/library";
import type { MenuItem } from "../stores/ui";
import "./facets.css";

const nowSec = () => Math.floor(Date.now() / 1000);

/** The chips in use for a view, and a setter. */
export function useFacets(view: ChipView): [LibChips, (c: LibChips) => void] {
  const chips = useLibraryChips((s) => s.chips[view]);
  return [chips, (c) => useLibraryChips.getState().set(view, c)];
}

/** Album facts by album id and by artist id, from the library's items. */
export function useAlbumFacts(items: LibraryItem[] | null): { byAlbum: Map<string, ChipFacts>; byArtist: Map<string, ChipFacts[]> } {
  return useMemo(() => {
    const byAlbum = new Map<string, ChipFacts>();
    const byArtist = new Map<string, ChipFacts[]>();
    for (const i of items ?? []) {
      if (i.kind !== "album") continue;
      const f = albumFacts(i);
      byAlbum.set(i.id, f);
      if (i.creatorId) byArtist.set(i.creatorId, [...(byArtist.get(i.creatorId) ?? []), f]);
    }
    return { byAlbum, byArtist };
  }, [items]);
}

/** Library rows narrowed by the chips: albums by their own facts, artists by any of their albums. */
export function facetEntries(entries: LibEntry[], chips: LibChips, facts: ReturnType<typeof useAlbumFacts>, now = nowSec()): LibEntry[] {
  if (!chipsOn(chips)) return entries;
  return entries.filter((e) => {
    if (e.kind === "album") {
      const f = facts.byAlbum.get(e.id);
      return !!f && matchesChips(f, chips, now);
    }
    if (e.kind === "artist") return anyMatches(facts.byArtist.get(e.id) ?? [], chips, now);
    return false;
  });
}

/** A song's facts: its own files, and its album's type, year and date added. */
export function songFacts(t: Pick<Track, "albumId" | "codec" | "bitDepth" | "sampleRate">, byAlbum: Map<string, ChipFacts>): ChipFacts {
  const a = byAlbum.get(t.albumId);
  return { type: a?.type ?? null, year: a?.year ?? null, addedAt: a?.addedAt ?? null, quality: trackQuality(t) };
}

/** Songs narrowed by the chips. */
export function facetSongs<T extends Pick<Track, "albumId" | "codec" | "bitDepth" | "sampleRate">>(tracks: T[], chips: LibChips, byAlbum: Map<string, ChipFacts>, now = nowSec()): T[] {
  if (!chipsOn(chips)) return tracks;
  return tracks.filter((t) => matchesChips(songFacts(t, byAlbum), chips, now));
}

/** The options worth offering for a view's rows. */
export function facetOptions(view: ChipView, facts: ReturnType<typeof useAlbumFacts>, now = nowSec()): ChipOptions {
  if (view === "archive") return { ...chipOptions([...facts.byAlbum.values()].filter((f) => f.type === "archive"), now), types: [] };
  return chipOptions(view === "songs" ? [] : [...facts.byAlbum.values()], now);
}

const NOUN: Record<ChipView, string> = { albums: "albums", artists: "artists", songs: "songs", archive: "albums" };

export function FacetChips({ view, options, chips, onChange }: { view: ChipView; options: ChipOptions; chips: LibChips; onChange: (c: LibChips) => void }) {
  const roving = useRovingRow();
  const types = chips.type && !options.types.includes(chips.type) ? [...options.types, chips.type] : options.types;
  const decades = chips.decade !== null && !options.decades.includes(chips.decade) ? [...options.decades, chips.decade] : options.decades;
  const formats = chips.format && !options.formats.includes(chips.format) ? [...options.formats, chips.format] : options.formats;
  const recent = options.recent || chips.recent;
  if (!types.length && !decades.length && !formats.length && !recent) return null;
  const on = chipsOn(chips);

  const menu = (e: React.MouseEvent<HTMLButtonElement>, label: string, items: MenuItem[]) => openContextMenu(label, items, e.currentTarget);
  return (
    <div role="group" aria-label={`Filter ${NOUN[view]}`} className="facets" data-testid={`facets-${view}`} {...roving}>
      {types.length > 0 && (
        <button
          type="button"
          className={`facet facet--menu ${chips.type ? "is-on" : ""}`}
          aria-haspopup="menu"
          aria-label={chips.type ? `Type: ${TYPE_LABEL[chips.type]}` : "Type"}
          onClick={(e) =>
            menu(e, "Type", [
              { label: "Any type", checked: !chips.type, icon: null, onSelect: () => onChange({ ...chips, type: null }) },
              ...types.map((t): MenuItem => ({ label: TYPE_LABEL[t], checked: chips.type === t, icon: null, onSelect: () => onChange({ ...chips, type: t }) })),
            ])
          }
        >
          <span>{chips.type ? TYPE_LABEL[chips.type] : "Type"}</span>
          <ChevronDown size={14} aria-hidden="true" />
        </button>
      )}
      {formats.map((f) => (
        <button key={f} type="button" className={`facet ${chips.format === f ? "is-on" : ""}`} aria-pressed={chips.format === f} onClick={() => onChange(pressFacet(chips, "format", f))}>
          {FORMAT_LABEL[f]}
        </button>
      ))}
      {decades.length > 0 && (
        <button
          type="button"
          className={`facet facet--menu ${chips.decade !== null ? "is-on" : ""}`}
          aria-haspopup="menu"
          aria-label={chips.decade !== null ? `Decade: ${decadeLabel(chips.decade)}` : "Decade"}
          onClick={(e) =>
            menu(e, "Decade", [
              { label: "Any decade", checked: chips.decade === null, icon: null, onSelect: () => onChange({ ...chips, decade: null }) },
              ...decades.map((d): MenuItem => ({ label: decadeLabel(d), checked: chips.decade === d, icon: null, onSelect: () => onChange({ ...chips, decade: d }) })),
            ])
          }
        >
          <span>{chips.decade !== null ? decadeLabel(chips.decade) : "Decade"}</span>
          <ChevronDown size={14} aria-hidden="true" />
        </button>
      )}
      {recent && (
        <button type="button" className={`facet ${chips.recent ? "is-on" : ""}`} aria-pressed={chips.recent} onClick={() => onChange(pressFacet(chips, "recent", true))}>
          Added recently
        </button>
      )}
      {on && (
        <button type="button" className="facet facet--clear" onClick={() => onChange({ type: null, format: null, decade: null, recent: false })}>
          <X size={14} aria-hidden="true" />
          Clear filters
        </button>
      )}
    </div>
  );
}

/** What to say when the chips leave nothing. */
export function FacetsEmpty({ view, onClear }: { view: ChipView; onClear: () => void }) {
  return (
    <div className="facets-empty" role="status">
      <p className="facets-empty__text">No {NOUN[view]} match these filters. Clear one to see more.</p>
      <button type="button" className="facet facet--clear" onClick={onClear}>
        <X size={14} aria-hidden="true" />
        Clear filters
      </button>
    </div>
  );
}
