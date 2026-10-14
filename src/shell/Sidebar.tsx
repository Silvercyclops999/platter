import { House, LibraryBig, Pin as PinIcon, Search } from "lucide-react";
import type { ReactNode } from "react";
import { Art } from "../components/Art";
import { openContextMenu } from "../components/ContextMenu";
import { Skeleton } from "../components/Skeleton";
import { Tooltip } from "../components/Tooltip";
import { plural } from "../lib/format";
import { useApp } from "../stores/app";
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
  route,
  title,
  subtitle,
  thumb,
  kind,
  active,
  collapsed,
  onPlay,
  pinned,
}: {
  route: Route;
  title: string;
  subtitle: string;
  thumb: string | null;
  kind: "album" | "artist" | "playlist";
  active: boolean;
  collapsed: boolean;
  onPlay: () => void;
  pinned?: boolean;
}) {
  const btn = (
    <button
      type="button"
      className={`lib-item ${active ? "is-active" : ""}`}
      aria-current={active ? "page" : undefined}
      aria-label={collapsed ? `${title}, ${subtitle}` : undefined}
      onClick={() => nav.go(route)}
      onContextMenu={(e) => {
        e.preventDefault();
        openContextMenu(`Options for ${title}`, [{ label: "Play", onSelect: onPlay }, { label: "Open", onSelect: () => nav.go(route) }], e);
      }}
    >
      <Art path={thumb} size={collapsed ? 40 : 40} round={kind === "artist"} kind={kind} />
      {!collapsed && (
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

export function Sidebar({ collapsed }: { collapsed: boolean }) {
  const entry = useCurrentEntry();
  const r = entry.route;
  const playlists = useApp((s) => s.playlists);
  const pins = usePins((s) => s.pins);

  const isActive = (route: Route) => JSON.stringify(route) === JSON.stringify(r);

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
        <div className="sidebar__lib-list" role="list" aria-label="Pinned and playlists">
          {pins.map((p) => (
            <div role="listitem" key={`${p.kind}-${p.id}`}>
              <LibraryEntry
                route={{ name: p.kind, id: p.id }}
                title={p.title}
                subtitle={p.subtitle}
                thumb={p.thumb}
                kind={p.kind}
                pinned
                collapsed={collapsed}
                active={isActive({ name: p.kind, id: p.id })}
                onPlay={() =>
                  p.kind === "album" ? player.playAlbum(p.id) : p.kind === "artist" ? player.playArtist(p.id) : player.playPlaylist(p.id)
                }
              />
            </div>
          ))}
          {playlists
            ? playlists
                .filter((pl) => !pins.some((p) => p.kind === "playlist" && p.id === pl.id))
                .map((pl) => (
                  <div role="listitem" key={pl.id}>
                    <LibraryEntry
                      route={{ name: "playlist", id: pl.id }}
                      title={pl.title}
                      subtitle={`${pl.smart ? "Smart playlist" : "Playlist"} · ${plural(pl.trackCount, "song")}`}
                      thumb={pl.thumb}
                      kind="playlist"
                      collapsed={collapsed}
                      active={isActive({ name: "playlist", id: pl.id })}
                      onPlay={() => player.playPlaylist(pl.id)}
                    />
                  </div>
                ))
            : Array.from({ length: 5 }, (_, i) => (
                <div key={i} className="lib-item lib-item--skeleton" aria-hidden="true">
                  <Skeleton width={40} height={40} />
                  {!collapsed && (
                    <span className="lib-item__text">
                      <Skeleton width="70%" height={13} />
                      <Skeleton width="45%" height={11} style={{ marginTop: 6 }} />
                    </span>
                  )}
                </div>
              ))}
        </div>
        <div className="sidebar__status">
          <StatusDot compact={collapsed} />
        </div>
      </div>
    </nav>
  );
}
