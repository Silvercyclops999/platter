import { Maximize2, PanelLeftClose, PanelLeftOpen } from "lucide-react";
import { useState } from "react";
import { IconButton } from "../components/Button";
import { useLibraryView } from "../stores/library";
import { nav } from "../stores/router";
import { useUi } from "../stores/ui";
import { LibraryCollection } from "./LibraryCollection";
import { LibraryChips, LibrarySearch, LibrarySortView } from "./LibraryControls";
import { StatusDot } from "./StatusDot";
import { useLibraryLoader, useLibraryModel, useLibraryRecents } from "./useLibrary";

/**
 * Left sidebar: the library (every playlist, album and artist) with its
 * chips, search, sort and views. Home and Search live in the top bar. `collapsed` shows the rail:
 * art tiles only. `auto` means the window is too narrow to open it, so the
 * rail's open button goes to the wide library instead.
 */
export function Sidebar({ collapsed, auto = false }: { collapsed: boolean; auto?: boolean }) {
  const view = useLibraryView((s) => s.view);
  const showArt = useUi((s) => s.sidebarArt);
  const [query, setQuery] = useState("");
  useLibraryLoader();
  useLibraryRecents();
  const model = useLibraryModel(collapsed ? "" : query);

  return (
    <nav className={`sidebar ${collapsed ? "is-collapsed" : ""}`} aria-label="Main">
      <section className="sidebar__block sidebar__library" aria-label="Your Library">
        {collapsed ? (
          <div className="lib-head lib-head--rail">
            <IconButton
              label="Open Your Library"
              tooltipPlacement="right"
              icon={<PanelLeftOpen size={20} />}
              onClick={() => (auto ? nav.go({ name: "library" }) : useUi.getState().setSidebar({ collapsed: false }))}
            />
          </div>
        ) : (
          <>
            <div className="lib-head">
              <button type="button" className="lib-head__title" aria-label="Collapse Your Library" onClick={() => useUi.getState().setSidebar({ collapsed: true })}>
                <span className="lib-head__icon" aria-hidden="true">
                  <PanelLeftClose size={18} />
                </span>
                <span className="lib-head__text">Your Library</span>
              </button>
              <IconButton size="sm" label="Expand Your Library" icon={<Maximize2 size={16} />} onClick={() => nav.go({ name: "library" })} />
            </div>
            <LibraryChips filter={model.filter} sub={model.sub} kinds={model.kinds} />
            <div className="lib-tools">
              <LibrarySearch query={query} setQuery={setQuery} filter={model.filter} />
              <LibrarySortView filter={model.filter} sort={model.sort} view={view} setView={(v) => useLibraryView.getState().setView(v)} />
            </div>
          </>
        )}
        <LibraryCollection model={model} view={view} mode={collapsed ? "rail" : "sidebar"} query={query} showArt={showArt} />
        <div className="sidebar__status">
          <StatusDot compact={collapsed} />
        </div>
      </section>
    </nav>
  );
}
