/** Library header controls: filter chips, search, and the sort and view menu. */
import { AlignJustify, Grid3x3, LayoutGrid, LibraryBig, List, Search, X } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { IconButton } from "../components/Button";
import { ChipRow } from "../components/Tabs";
import { openContextMenu } from "../components/ContextMenu";
import {
  KIND_LABEL,
  LIB_VIEWS,
  SORT_LABEL,
  SUB_LABEL,
  chipRowWithDownloads,
  pressChip,
  searchPlaceholder,
  useLibraryView,
  type LibFilter,
  type LibKind,
  type LibSort,
  type LibSub,
  type LibView,
  VIEW_LABEL,
} from "../stores/library";
import { useUi, type MenuItem } from "../stores/ui";

export function LibraryChips({
  filter,
  sub,
  kinds,
  downloaded = { available: false, on: false },
  shelf = false,
}: {
  filter: LibFilter;
  sub: LibSub | null;
  kinds: LibKind[];
  downloaded?: { available: boolean; on: boolean };
  /** Offer the albums' "On the shelf" chip. */
  shelf?: boolean;
}) {
  const chips = chipRowWithDownloads(filter, sub, kinds, downloaded, shelf);
  if (chips.length === 0) return null;
  return (
    // One line that scrolls sideways, fading at an edge with more.
    <ChipRow label="Show in library" className="lib-chips" scroll>
      {chips.map((c) => {
        const press = () => {
          const lib = useLibraryView.getState();
          if (c.kind === "downloaded") return lib.setDownloaded(!downloaded.on);
          if (c.kind === "clear") lib.setDownloaded(false);
          const next = pressChip(c, filter, sub);
          lib.setFilter(next.filter, next.sub);
        };
        if (c.kind === "clear")
          return (
            <button key="clear" type="button" className="chip chip--sm chip--clear lib-chips__clear" aria-label="Clear filter" onClick={press}>
              <X size={16} aria-hidden="true" />
            </button>
          );
        const label = c.kind === "type" ? KIND_LABEL[c.type] : c.kind === "sub" ? SUB_LABEL[c.sub] : "Downloaded";
        return (
          <button
            key={c.kind === "type" ? c.type : c.kind === "sub" ? c.sub : "downloaded"}
            type="button"
            aria-pressed={c.selected}
            className={`chip chip--sm ${c.selected ? "is-selected" : ""}`}
            onClick={press}
          >
            {label}
          </button>
        );
      })}
    </ChipRow>
  );
}

/**
 * Library search. At rest a magnifier button that opens into a field (or
 * always a field, in wide mode). Filters live as you type; Escape clears the
 * text and keeps focus; leaving it empty closes it again.
 */
export function LibrarySearch({
  query,
  setQuery,
  filter,
  alwaysOpen = false,
}: {
  query: string;
  setQuery: (q: string) => void;
  filter: LibFilter;
  alwaysOpen?: boolean;
}) {
  const [open, setOpen] = useState(false);
  const input = useRef<HTMLInputElement>(null);
  const placeholder = searchPlaceholder(filter);
  useEffect(() => {
    if (open) input.current?.focus();
  }, [open]);
  // Ctrl/⌘+F opens the field and focuses it.
  const filterTick = useUi((s) => s.libraryFilterTick);
  const seenTick = useRef(filterTick);
  useEffect(() => {
    if (filterTick === seenTick.current) return;
    seenTick.current = filterTick;
    setOpen(true);
    input.current?.focus();
    input.current?.select();
  }, [filterTick]);
  if (!open && !alwaysOpen && !query)
    return <IconButton size="sm" label={placeholder} icon={<Search size={16} />} onClick={() => setOpen(true)} className="lib-search-btn" />;
  return (
    <label className={`lib-search ${alwaysOpen ? "lib-search--pill" : ""}`}>
      <Search size={16} aria-hidden="true" />
      <input
        ref={input}
        type="search"
        value={query}
        placeholder={placeholder}
        aria-label={placeholder}
        onChange={(e) => setQuery(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Escape") {
            e.preventDefault();
            e.stopPropagation();
            setQuery("");
          }
          // Arrows stay in the field.
          if (e.key === "ArrowDown" || e.key === "ArrowUp") e.stopPropagation();
        }}
        onBlur={() => !query && setOpen(false)}
      />
    </label>
  );
}

const VIEW_ICON: Record<LibView, typeof List> = { list: List, compactList: AlignJustify, compactGrid: Grid3x3, grid: LayoutGrid, shelf: LibraryBig };

/** The button that shows the sort name and view icon, and opens their menu. */
export function LibrarySortView({
  filter,
  sort,
  view,
  setView,
  views = LIB_VIEWS,
}: {
  filter: LibFilter;
  sort: LibSort;
  view: LibView;
  setView: (v: LibView) => void;
  /** The views on offer (wide mode adds the shelf). */
  views?: LibView[];
}) {
  const sorts: LibSort[] = ["recents", "added", "alpha", "creator", ...(filter === "playlist" ? (["custom"] as LibSort[]) : [])];
  const Icon = VIEW_ICON[view];
  return (
    <button
      type="button"
      className="lib-sort"
      aria-label={`Sort and view: ${SORT_LABEL[sort]}, ${VIEW_LABEL[view].toLowerCase()} view`}
      aria-haspopup="menu"
      onClick={(e) =>
        openContextMenu(
          "Sort and view",
          [
            ...sorts.map(
              (s, i): MenuItem => ({
                heading: i === 0 ? "Sort by" : undefined,
                label: SORT_LABEL[s],
                checked: sort === s,
                onSelect: () => useLibraryView.getState().setSort(s),
              }),
            ),
            ...views.map(
              (v, i): MenuItem => ({
                separator: i === 0,
                heading: i === 0 ? "View as" : undefined,
                label: VIEW_LABEL[v],
                checked: view === v,
                onSelect: () => setView(v),
              }),
            ),
          ],
          e.currentTarget,
        )
      }
    >
      <span>{SORT_LABEL[sort]}</span>
      <Icon size={16} aria-hidden="true" />
    </button>
  );
}
