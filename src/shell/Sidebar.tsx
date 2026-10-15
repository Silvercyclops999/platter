import { ChevronLeft, Folder, ListMusic, Maximize2, MoreHorizontal, PanelLeftClose, PanelLeftOpen, Plus } from "lucide-react";
import { useState, type MouseEvent } from "react";
import { IconButton } from "../components/Button";
import { openContextMenu } from "../components/ContextMenu";
import { useLibraryView } from "../stores/library";
import { folders } from "../stores/folders";
import { nav } from "../stores/router";
import { smartCreateItems } from "../stores/curate";
import { useUi, type MenuItem } from "../stores/ui";
import { createPlaylistIn, folderMenu } from "./FolderMenus";
import { LibraryCollection } from "./LibraryCollection";
import { LibraryChips, LibrarySearch, LibrarySortView } from "./LibraryControls";
import { ServerSwitcher } from "./ServerSwitcher";
import { StatusDot } from "./StatusDot";
import "./library-create.css";
import { useLibraryLoader, useLibraryModel, useLibraryRecents } from "./useLibrary";

/** The Create menu: a playlist or a folder (inside the open folder, if any). */
export function createMenu(folderId: string | null): MenuItem[] {
  return [
    { label: "Playlist", icon: ListMusic, onSelect: () => void createPlaylistIn(folderId) },
    ...smartCreateItems(),
    { label: "Folder", icon: Folder, onSelect: () => void folders.create(folderId) },
  ];
}

const openCreate = (folderId: string | null) => (e: MouseEvent<HTMLButtonElement>) =>
  openContextMenu("Create", createMenu(folderId), e.currentTarget);

/**
 * Left sidebar: the library (every playlist, album and artist) with its
 * chips, search, sort and views. Home and Search live in the top bar. `collapsed` shows the rail:
 * art tiles only. `auto` means the window is too narrow to open it, so the
 * rail's open button goes to the wide library instead. Opening a folder
 * drills the list into it, with a back arrow and the folder's name on top.
 */
export function Sidebar({ collapsed, auto = false }: { collapsed: boolean; auto?: boolean }) {
  const view = useLibraryView((s) => s.view);
  const showArt = useUi((s) => s.sidebarArt);
  const [query, setQuery] = useState("");
  useLibraryLoader();
  useLibraryRecents();
  const model = useLibraryModel(collapsed ? "" : query, !collapsed);
  const drill = collapsed ? null : model.drill;

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
            <IconButton
              label="Create playlist or folder"
              tooltipPlacement="right"
              className="lib-create lib-create--rail"
              icon={<Plus size={20} />}
              onClick={openCreate(null)}
            />
          </div>
        ) : (
          <>
            <div className="lib-head">
              {drill ? (
                <button type="button" className="lib-head__back" aria-label={`Back from ${drill.name}`} onClick={() => folders.back()}>
                  <ChevronLeft size={20} aria-hidden="true" />
                  <span className="lib-head__back-name">{drill.name}</span>
                </button>
              ) : (
                <button type="button" className="lib-head__title" aria-label="Collapse Your Library" onClick={() => useUi.getState().setSidebar({ collapsed: true })}>
                  <span className="lib-head__icon" aria-hidden="true">
                    <PanelLeftClose size={20} />
                  </span>
                  <span className="lib-head__text">Your Library</span>
                </button>
              )}
              <span className="lib-head__actions">
                <IconButton
                  size="sm"
                  label={drill ? `Create in ${drill.name}` : "Create playlist or folder"}
                  className="lib-create"
                  icon={<Plus size={20} />}
                  onClick={openCreate(drill?.id ?? null)}
                />
                {drill && (
                  <IconButton
                    size="sm"
                    label={`More options for ${drill.name}`}
                    icon={<MoreHorizontal size={20} />}
                    onClick={(e) => openContextMenu(`Options for ${drill.name}`, folderMenu(drill.id), e.currentTarget)}
                  />
                )}
                <IconButton size="sm" label="Expand Your Library" icon={<Maximize2 size={16} />} onClick={() => nav.go({ name: "library" })} />
              </span>
            </div>
            {!drill && <ServerSwitcher />}
            <LibraryChips filter={model.filter} sub={model.sub} kinds={model.kinds} downloaded={model.downloaded} />
            <div className="lib-tools">
              <LibrarySearch query={query} setQuery={setQuery} filter={model.filter} />
              <LibrarySortView filter={model.filter} sort={model.sort} view={view} setView={(v) => useLibraryView.getState().setView(v)} />
            </div>
          </>
        )}
        {drill && model.shown.length === 0 && !model.loading ? (
          <p className="lib-drill-empty" role="status">
            This folder is empty. Drag playlists here, or use Move to folder in a playlist's menu.
          </p>
        ) : (
          <LibraryCollection model={model} view={view} mode={collapsed ? "rail" : "sidebar"} query={query} showArt={showArt} label={drill ? `${drill.name} items` : undefined} />
        )}
        <div className="sidebar__status">
          <StatusDot compact={collapsed} />
        </div>
      </section>
    </nav>
  );
}
