import { House, Maximize2, PanelLeftClose, PanelLeftOpen, Search } from "lucide-react";
import { useState, type ReactNode } from "react";
import { IconButton } from "../components/Button";
import { Tooltip } from "../components/Tooltip";
import { useLibraryView } from "../stores/library";
import { nav, useCurrentEntry } from "../stores/router";
import { useUi } from "../stores/ui";
import { LibraryCollection } from "./LibraryCollection";
import { LibraryChips, LibrarySearch, LibrarySortView } from "./LibraryControls";
import { Logo } from "./Logo";
import { StatusDot } from "./StatusDot";
import { useLibraryLoader, useLibraryModel, useLibraryRecents } from "./useLibrary";

function NavItem({ icon, label, active, onClick, collapsed }: { icon: ReactNode; label: string; active: boolean; onClick: () => void; collapsed: boolean }) {
  const btn = (
    <button
      type="button"
      className={`side-nav__item ${active ? "is-active" : ""}`}
      aria-current={active ? "page" : undefined}
      aria-label={collapsed ? label : undefined}
      onClick={onClick}
    >
      <span className="side-nav__icon" aria-hidden="true">
        {icon}
      </span>
      {!collapsed && <span>{label}</span>}
    </button>
  );
  return collapsed ? (
    <Tooltip label={label} placement="right">
      {btn}
    </Tooltip>
  ) : (
    btn
  );
}

/**
 * Left sidebar: Home, Search, then the library (every playlist, album and
 * artist) with its chips, search, sort and views. `collapsed` shows the rail:
 * art tiles only. `auto` means the window is too narrow to open it, so the
 * rail's open button goes to the wide library instead.
 */
export function Sidebar({ collapsed, auto = false }: { collapsed: boolean; auto?: boolean }) {
  const r = useCurrentEntry().route;
  const view = useLibraryView((s) => s.view);
  const showArt = useUi((s) => s.sidebarArt);
  const [query, setQuery] = useState("");
  useLibraryLoader();
  useLibraryRecents();
  const model = useLibraryModel(collapsed ? "" : query);

  return (
    <nav className={`sidebar ${collapsed ? "is-collapsed" : ""}`} aria-label="Main">
      <div className="sidebar__block">
        <div className="sidebar__brand">
          <Logo size={28} />
          {!collapsed && <span className="sidebar__wordmark">Platter</span>}
        </div>
        <NavItem collapsed={collapsed} icon={<House size={22} />} label="Home" active={r.name === "home"} onClick={() => nav.go({ name: "home" })} />
        <NavItem collapsed={collapsed} icon={<Search size={22} />} label="Search" active={r.name === "search"} onClick={() => nav.go({ name: "search" })} />
      </div>

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
