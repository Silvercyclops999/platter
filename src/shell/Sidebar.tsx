import { AlignJustify, House, LibraryBig, List, Pin as PinIcon, Search } from "lucide-react";
import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { IconButton } from "../components/Button";
import { Art } from "../components/Art";
import { openContextMenu } from "../components/ContextMenu";
import { Skeleton } from "../components/Skeleton";
import { Tooltip } from "../components/Tooltip";
import { plural } from "../lib/format";
import { useApp } from "../stores/app";
import { libraryEntries, useLibraryView, type LibEntry, type LibKind, type LibSort } from "../stores/library";
import { usePins } from "../stores/pins";
import { player } from "../stores/player";
import { nav, useCurrentEntry, type Route } from "../stores/router";
import { Logo } from "./Logo";
import { StatusDot } from "./StatusDot";

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

function LibraryEntry({
  id,
  route,
  title,
  subtitle,
  thumb,
  kind,
  active,
  collapsed,
  onPlay,
  pinned,
  compact,
}: {
  id: string;
  route: Route;
  title: string;
  subtitle: string;
  thumb: string | null;
  kind: "album" | "artist" | "playlist";
  active: boolean;
  collapsed: boolean;
  onPlay: () => void;
  pinned?: boolean;
  compact?: boolean;
}) {
  const kindLabel = kind === "album" ? "Album" : kind === "artist" ? "Artist" : "Playlist";
  const btn = (
    <button
      type="button"
      className={`lib-item ${active ? "is-active" : ""} ${compact ? "lib-item--compact" : ""}`}
      aria-current={active ? "page" : undefined}
      aria-label={collapsed ? `${title}, ${subtitle}` : undefined}
      onClick={() => nav.go(route)}
      onContextMenu={(e) => {
        e.preventDefault();
        openContextMenu(
          `Options for ${title}`,
          [
            { label: "Play", onSelect: onPlay },
            { label: "Open", onSelect: () => nav.go(route) },
            ...(pinned
              ? [{ label: "Unpin from sidebar", separator: true, onSelect: () => usePins.getState().toggle({ kind, id, title, subtitle, thumb }) }]
              : []),
          ],
          e,
        );
      }}
    >
      {compact ? (
        <span className="lib-item__compact truncate">
          {pinned && <PinIcon size={11} className="lib-item__pin" aria-label="Pinned" />}
          <span className={`lib-item__title ${active ? "is-active" : ""}`}>{title}</span>
          <span className="lib-item__sub"> · {kindLabel}</span>
        </span>
      ) : (
        <Art path={thumb} size={40} round={kind === "artist"} kind={kind} />
      )}
      {!collapsed && !compact && (
        <span className="lib-item__text">
          <span className={`lib-item__title truncate ${active ? "is-active" : ""}`}>{title}</span>
          <span className="lib-item__sub truncate">
            {pinned && <PinIcon size={11} className="lib-item__pin" aria-label="Pinned" />}
            {subtitle}
          </span>
        </span>
      )}
    </button>
  );
  return collapsed ? (
    <Tooltip label={title} placement="right">
      {btn}
    </Tooltip>
  ) : (
    btn
  );
}

const KIND_LABEL: Record<LibKind, string> = { playlist: "Playlists", album: "Albums", artist: "Artists" };
const SORT_LABEL: Record<LibSort, string> = { recents: "Recents", alpha: "Alphabetical" };

/** Search, sort and view controls above the sidebar library list. */
function LibraryTools({ query, setQuery }: { query: string; setQuery: (q: string) => void }) {
  const { sort, view, setSort, setView } = useLibraryView();
  const [searching, setSearching] = useState(false);
  const input = useRef<HTMLInputElement>(null);
  useEffect(() => {
    if (searching) input.current?.focus();
  }, [searching]);
  const close = () => {
    setQuery("");
    setSearching(false);
  };
  return (
    <div className="lib-tools">
      {searching ? (
        <label className="lib-search">
          <Search size={14} aria-hidden="true" />
          <input
            ref={input}
            type="search"
            value={query}
            placeholder="Search in Your Library"
            aria-label="Search in Your Library"
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Escape") {
                e.preventDefault();
                close();
              }
            }}
            onBlur={() => !query && setSearching(false)}
          />
        </label>
      ) : (
        <IconButton size="sm" label="Search in Your Library" icon={<Search size={16} />} onClick={() => setSearching(true)} />
      )}
      <button
        type="button"
        className="lib-sort"
        aria-label={`Sort and view: ${SORT_LABEL[sort]}, ${view === "list" ? "list" : "compact"}`}
        onClick={(e) =>
          openContextMenu(
            "Sort and view",
            [
              { heading: "Sort by", label: "Recents", checked: sort === "recents", onSelect: () => setSort("recents") },
              { label: "Alphabetical", checked: sort === "alpha", onSelect: () => setSort("alpha") },
              { separator: true, heading: "View as", label: "List", checked: view === "list", onSelect: () => setView("list") },
              { label: "Compact", checked: view === "compact", onSelect: () => setView("compact") },
            ],
            e.currentTarget,
          )
        }
      >
        <span>{SORT_LABEL[sort]}</span>
        {view === "list" ? <List size={16} aria-hidden="true" /> : <AlignJustify size={16} aria-hidden="true" />}
      </button>
    </div>
  );
}

export function Sidebar({ collapsed }: { collapsed: boolean }) {
  const entry = useCurrentEntry();
  const r = entry.route;
  const playlists = useApp((s) => s.playlists);
  const pins = usePins((s) => s.pins);
  const { filter, sort, view, recents, setFilter } = useLibraryView();
  const [query, setQuery] = useState("");

  // Opening an album, artist or playlist moves it up the Recents sort.
  useEffect(() => {
    if (r.name === "album" || r.name === "artist" || r.name === "playlist") useLibraryView.getState().touch(r.name, r.id);
  }, [r]);

  const isActive = (route: Route) => JSON.stringify(route) === JSON.stringify(r);

  const all: LibEntry[] = useMemo(
    () => [
      ...pins.map((p) => ({ kind: p.kind, id: p.id, title: p.title, subtitle: p.subtitle, thumb: p.thumb, pinned: true })),
      ...(playlists ?? [])
        .filter((pl) => !pins.some((p) => p.kind === "playlist" && p.id === pl.id))
        .map((pl) => ({
          kind: "playlist" as const,
          id: pl.id,
          title: pl.title,
          subtitle: `${pl.smart ? "Smart playlist" : "Playlist"} · ${plural(pl.trackCount, "song")}`,
          thumb: pl.thumb,
          pinned: false,
        })),
    ],
    [pins, playlists],
  );
  const kinds = (["playlist", "album", "artist"] as LibKind[]).filter((k) => all.some((e) => e.kind === k));
  // A saved filter for a type that's no longer there shows everything.
  const activeFilter = filter !== "all" && !kinds.includes(filter) ? "all" : filter;
  const shown = libraryEntries(all, { filter: activeFilter, sort, query: collapsed ? "" : query, recents });
  const compact = view === "compact" && !collapsed;
  const loading = playlists === null;

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

      <div className="sidebar__block sidebar__library">
        <div className="sidebar__lib-head">
          <NavItem
            collapsed={collapsed}
            icon={<LibraryBig size={22} />}
            label="Your Library"
            active={r.name === "library"}
            onClick={() => nav.go({ name: "library", tab: "albums" })}
          />
        </div>
        {!collapsed && kinds.length > 0 && (
          <div role="group" aria-label="Show in library" className="chips lib-chips">
            {kinds.map((k) => (
              <button
                key={k}
                type="button"
                aria-pressed={activeFilter === k}
                className={`chip chip--sm ${activeFilter === k ? "is-selected" : ""}`}
                onClick={() => setFilter(activeFilter === k ? "all" : k)}
              >
                {KIND_LABEL[k]}
              </button>
            ))}
          </div>
        )}
        {!collapsed && <LibraryTools query={query} setQuery={setQuery} />}
        <div className={`sidebar__lib-list ${compact ? "is-compact" : ""}`} role="list" aria-label="Your Library items">
          {loading
            ? Array.from({ length: 5 }, (_, i) => (
                <div key={i} className="lib-item lib-item--skeleton" aria-hidden="true">
                  <Skeleton width={40} height={40} />
                  {!collapsed && (
                    <span className="lib-item__text">
                      <Skeleton width="70%" height={13} />
                      <Skeleton width="45%" height={11} style={{ marginTop: 6 }} />
                    </span>
                  )}
                </div>
              ))
            : shown.map((e) => (
                <div role="listitem" key={`${e.kind}-${e.id}`}>
                  <LibraryEntry
                    id={e.id}
                    route={{ name: e.kind, id: e.id }}
                    title={e.title}
                    subtitle={e.subtitle}
                    thumb={e.thumb}
                    kind={e.kind}
                    pinned={e.pinned}
                    compact={compact}
                    collapsed={collapsed}
                    active={isActive({ name: e.kind, id: e.id })}
                    onPlay={() =>
                      e.kind === "album" ? player.playAlbum(e.id) : e.kind === "artist" ? player.playArtist(e.id) : player.playPlaylist(e.id)
                    }
                  />
                </div>
              ))}
          {!loading && !collapsed && shown.length === 0 && (
            <p className="lib-empty" role="status">
              {query.trim()
                ? `Nothing in your library matches "${query.trim()}".`
                : all.length === 0
                  ? "Pin albums, artists and playlists from their menus to keep them here."
                  : `No ${KIND_LABEL[activeFilter as LibKind]?.toLowerCase() ?? "items"} here yet.`}
            </p>
          )}
        </div>
        <div className="sidebar__status">
          <StatusDot compact={collapsed} />
        </div>
      </div>
    </nav>
  );
}
