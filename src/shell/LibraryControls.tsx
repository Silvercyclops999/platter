/** Library header controls: filter chips, search, and the sort and view menu. */
import { AlignJustify, Grid3x3, LayoutGrid, List, Search, X } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { IconButton } from "../components/Button";
import { openContextMenu } from "../components/ContextMenu";
import {
  KIND_LABEL,
  LIB_VIEWS,
  SORT_LABEL,
  SUB_LABEL,
  chipRow,
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
import type { MenuItem } from "../stores/ui";

export function LibraryChips({ filter, sub, kinds }: { filter: LibFilter; sub: LibSub | null; kinds: LibKind[] }) {
  const chips = chipRow(filter, sub, kinds);
  if (chips.length === 0) return null;
  return (
    <div role="group" aria-label="Show in library" className="lib-chips">
      {chips.map((c) => {
        const press = () => {
          const next = pressChip(c, filter, sub);
          useLibraryView.getState().setFilter(next.filter, next.sub);
        };
        if (c.kind === "clear")
          return (
            <button key="clear" type="button" className="chip chip--sm lib-chips__clear" aria-label="Clear filter" onClick={press}>
              <X size={16} aria-hidden="true" />
            </button>
          );
        const label = c.kind === "type" ? KIND_LABEL[c.type] : SUB_LABEL[c.sub];
        return (
          <button
            key={c.kind === "type" ? c.type : c.sub}
            type="button"
            aria-pressed={c.selected}
            className={`chip chip--sm ${c.selected ? "is-selected" : ""}`}
            onClick={press}
          >
            {label}
          </button>
        );
      })}
    </div>
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

const VIEW_ICON: Record<LibView, typeof List> = { list: List, compactList: AlignJustify, compactGrid: Grid3x3, grid: LayoutGrid };

/** The button that shows the sort name and view icon, and opens their menu. */
export function LibrarySortView({ filter, sort, view, setView }: { filter: LibFilter; sort: LibSort; view: LibView; setView: (v: LibView) => void }) {
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
            ...LIB_VIEWS.map(
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
