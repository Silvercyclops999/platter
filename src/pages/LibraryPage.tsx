import { Minimize2 } from "lucide-react";
import { useLayoutEffect, useState } from "react";
import { IconButton } from "../components/Button";
import { LibraryCollection, LibraryTableHead } from "../shell/LibraryCollection";
import { LibraryChips, LibrarySearch, LibrarySortView } from "../shell/LibraryControls";
import { useLibraryLoader, useLibraryModel, useLibraryRecents } from "../shell/useLibrary";
import { useLibraryView, type LibKind } from "../stores/library";
import { nav, useRouter, type LibraryTab } from "../stores/router";
import { useUi } from "../stores/ui";

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
 * sidebar; its own view setting, and a table in the list views.
 */
export function LibraryPage({ tab }: { tab?: LibraryTab }) {
  const entryKey = useRouter((s) => s.entries[s.index].key);
  const view = useLibraryView((s) => s.wideView);
  const showArt = useUi((s) => s.sidebarArt);
  const [query, setQuery] = useState("");
  useLibraryLoader();
  useLibraryRecents();
  const model = useLibraryModel(query);

  // Arriving with a type (e.g. "Show all" albums) filters to it.
  useLayoutEffect(() => {
    if (tab) useLibraryView.getState().setFilter(TAB_KIND[tab]);
    // Once per history entry.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [entryKey]);

  const list = view === "list" || view === "compactList";
  return (
    <div className="page-pad library-wide">
      <div className="library-wide__head">
        <h1 className="library-wide__title">Your Library</h1>
        <IconButton label="Minimize Your Library" icon={<Minimize2 size={18} />} onClick={minimizeLibrary} />
      </div>
      <LibraryChips filter={model.filter} sub={model.sub} kinds={model.kinds} />
      <div className="lib-tools lib-tools--wide">
        <LibrarySearch query={query} setQuery={setQuery} filter={model.filter} alwaysOpen />
        <LibrarySortView filter={model.filter} sort={model.sort} view={view} setView={(v) => useLibraryView.getState().setWideView(v)} />
      </div>
      <LibraryCollection
        model={model}
        view={view}
        mode="wide"
        query={query}
        showArt={showArt}
        header={list && !model.loading && model.shown.length > 0 ? <LibraryTableHead compact={view === "compactList"} /> : null}
      />
    </div>
  );
}
