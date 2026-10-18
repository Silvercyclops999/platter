import { ChevronLeft, Minimize2 } from "lucide-react";
import { useLayoutEffect, useMemo, useState } from "react";
import { IconButton } from "../components/Button";
import { LibraryCollection, LibraryTableHead } from "../shell/LibraryCollection";
import { LibraryChips, LibrarySearch, LibrarySortView } from "../shell/LibraryControls";
import { FacetChips, FacetsEmpty, facetEntries, facetOptions, useAlbumFacts, useFacets } from "../shell/FacetChips";
import { NO_CHIPS, chipsOn, type ChipView } from "../lib/libraryChips";
import { useLibraryLoader, useLibraryModel, useLibraryRecents } from "../shell/useLibrary";
import { WIDE_VIEWS, useLibraryView, type LibKind } from "../stores/library";
import { nav, useRouter, type LibraryTab } from "../stores/router";
import { useUi } from "../stores/ui";
import { folders } from "../stores/folders";
import "../shell/folders.css";

const TAB_KIND: Record<LibraryTab, LibKind> = { albums: "album", artists: "artist", playlists: "playlist" };

/** Leave wide mode: back to the page before it, with the sidebar as it was. */
export function minimizeLibrary() {
  const { index } = useRouter.getState();
  if (index > 0) nav.back();
  else nav.go({ name: "home" });
}

/**
 * Your Library in wide mode: the sidebar's library as a full-width page (the
 * shell hides the sidebar on this route). Same chips, search and sort as the
 * sidebar; its own view setting (with the record shelf as an extra view),
 * and a table in the list views.
 */
export function LibraryPage({ tab, archive = false }: { tab?: LibraryTab; archive?: boolean }) {
  const entryKey = useRouter((s) => s.entries[s.index].key);
  const view = useLibraryView((s) => s.wideView);
  const showArt = useUi((s) => s.sidebarArt);
  const [query, setQuery] = useState("");
  useLibraryLoader();
  useLibraryRecents();
  const base = useLibraryModel(query);
  const items = useLibraryView((s) => s.items);
  const facts = useAlbumFacts(items);
  // Filter chips for albums and artists; playlists and the mixed view have none.
  const chipView: ChipView | null = archive ? "archive" : base.filter === "album" ? "albums" : base.filter === "artist" ? "artists" : null;
  const [chips, setChips] = useFacets(chipView ?? "albums");
  const active = useMemo(() => (archive ? { ...chips, type: "archive" as const } : chipView ? chips : NO_CHIPS), [archive, chips, chipView]);
  const model = useMemo(() => ({ ...base, shown: facetEntries(base.shown, active, facts) }), [base, active, facts]);
  const options = useMemo(() => (chipView ? facetOptions(chipView, facts) : null), [chipView, facts]);

  // Arriving with a type (e.g. "Show all" albums) filters to it. The archive
  // is the albums whose type is demos and unreleased, with chips of its own.
  useLayoutEffect(() => {
    if (tab) useLibraryView.getState().setFilter(TAB_KIND[tab]);
    // Once per history entry.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [entryKey]);

  const list = view === "list" || view === "compactList";
  return (
    <div className="page-pad library-wide">
      <div className="library-wide__head">
        {model.drill ? (
          <button type="button" className="lib-head__back library-wide__title" aria-label={`Back from ${model.drill.name}`} onClick={() => folders.back()}>
            <ChevronLeft size={24} aria-hidden="true" />
            <span className="lib-head__back-name">{model.drill.name}</span>
          </button>
        ) : (
          <h1 className="library-wide__title">{archive ? "Archive" : "Your Library"}</h1>
        )}
        <IconButton label="Minimize Your Library" icon={<Minimize2 size={18} />} onClick={minimizeLibrary} />
      </div>
      {!archive && <LibraryChips filter={model.filter} sub={model.sub} kinds={model.kinds} downloaded={model.downloaded} shelf={model.shelf} />}
      <div className="lib-tools lib-tools--wide">
        <LibrarySearch query={query} setQuery={setQuery} filter={model.filter} alwaysOpen />
        <LibrarySortView filter={model.filter} sort={model.sort} view={view} views={WIDE_VIEWS} setView={(v) => useLibraryView.getState().setWideView(v)} />
      </div>
      {chipView && options && <FacetChips view={chipView} options={options} chips={chips} onChange={setChips} />}
      {chipView && chipsOn(archive ? chips : active) && !model.loading && model.shown.length === 0 && !query ? (
        <FacetsEmpty view={chipView} onClear={() => setChips(NO_CHIPS)} />
      ) : (
      <LibraryCollection
        model={model}
        view={view}
        mode="wide"
        query={query}
        showArt={showArt}
        header={list && !model.loading && model.shown.length > 0 ? <LibraryTableHead compact={view === "compactList"} /> : null}
      />
      )}
    </div>
  );
}
