/**
 * The region host: draws every frame but Classic from its arrangement
 * (frames.ts). Regions are top, left, main, right, bottom and a drawer;
 * each component goes where the arrangement puts it. Index is one such
 * arrangement and a custom frame is another, so both run this code.
 */
import {
  Archive,
  ChevronLeft,
  ChevronRight,
  Disc3,
  House,
  Library,


  Search,
  Settings as SettingsIcon,
  Sparkles,
  UsersRound,
  Heart,
  Boxes,
} from "lucide-react";
import { useEffect, useMemo, useRef, useState, type CSSProperties, type ReactNode } from "react";
import { IconButton } from "../components/Button";
import { openContextMenu } from "../components/ContextMenu";
import { Tooltip } from "../components/Tooltip";
import { chrome } from "../lib/platform";
import type { FrameComponent, RegionName, Regions } from "../design/themes/validate";
import { useLibraryView, toEntries } from "../stores/library";
import { nav, useCanGoBack, useCanGoForward, useCurrentEntry, type Route } from "../stores/router";
import { useSettings } from "../stores/settings";
import { useLayout } from "../stores/theme";
import { rightView as slotView, useUi, type RightView } from "../stores/ui";
import { useApp } from "../stores/app";
import { FriendActivityPanel } from "./FriendActivityPanel";
import { frameRegions, regionOf, solveFrame, type FrameSolve } from "./frames";
import { LazyHelpPanel } from "./lazyHosts";
import { Logo } from "./Logo";
import { MainView } from "./MainView";
import { NowPlayingPanel } from "./NowPlayingPanel";
import { OutputPanel } from "./OutputPanel";
import { QueuePanel } from "./QueuePanel";
import { RemoteStrip } from "./RemoteStrip";
import { ListenStrip } from "./ListenStrip";
import { SearchBox } from "./SearchBox";
import { StatusDot } from "./StatusDot";
import { AppMenu, FriendActivityButton, OfflineIndicator, WhatsNewButton, WindowControls } from "./TopBar";
import { TransportStrip } from "./TransportStrip";
import { entryMenu, openEntry, useLibraryLoader } from "./useLibrary";
import { skipToMain, useShellTabOrder } from "./focusOrder";
import "./frames.css";

function useWindowWidth() {
  const [w, setW] = useState(() => (typeof window !== "undefined" ? window.innerWidth : 1280));
  useEffect(() => {
    const on = () => setW(window.innerWidth);
    window.addEventListener("resize", on);
    return () => window.removeEventListener("resize", on);
  }, []);
  return w;
}

// ---------------------------------------------------------------- sections

interface Section {
  key: string;
  label: string;
  icon: ReactNode;
  route: Route;
  /** Route names that light this section up. */
  on: (r: Route) => boolean;
}

function useSections(): { main: Section[]; library: Section[] } {
  const crates = useSettings((s) => s.settings?.showCrates !== false);
  const shelf = useSettings((s) => s.settings?.showPhysicalCollection !== false);
  return useMemo(() => {
    const main: Section[] = [
      { key: "home", label: "Home", icon: <House size={18} />, route: { name: "home" }, on: (r) => r.name === "home" },
      { key: "search", label: "Search", icon: <Search size={18} />, route: { name: "search" }, on: (r) => r.name === "search" },
      { key: "new", label: "What's new", icon: <Sparkles size={18} />, route: { name: "whatsNew" }, on: (r) => r.name === "whatsNew" },
    ];
    const library: Section[] = [
      { key: "recent", label: "Recently added", icon: <Disc3 size={18} />, route: { name: "homeShelf", shelf: "recentlyAdded" }, on: (r) => r.name === "homeShelf" && r.shelf === "recentlyAdded" },
      { key: "artists", label: "Artists", icon: <UsersRound size={18} />, route: { name: "library", tab: "artists" }, on: (r) => r.name === "library" && r.tab === "artists" },
      { key: "albums", label: "Albums", icon: <Library size={18} />, route: { name: "library", tab: "albums" }, on: (r) => r.name === "library" && r.tab === "albums" },
      { key: "songs", label: "Liked songs", icon: <Heart size={18} />, route: { name: "liked" }, on: (r) => r.name === "liked" },
      ...(crates ? [{ key: "crates", label: "Crates", icon: <Boxes size={18} />, route: { name: "crates" } as Route, on: (r: Route) => r.name === "crates" || r.name === "crate" }] : []),
      ...(shelf ? [{ key: "shelf", label: "On the shelf", icon: <Archive size={18} />, route: { name: "shelf" } as Route, on: (r: Route) => r.name === "shelf" }] : []),
    ];
    return { main, library };
  }, [crates, shelf]);
}

function SectionLink({ s, rail, current }: { s: Section; rail: boolean; current: Route }) {
  const on = s.on(current);
  const link = (
    <button
      type="button"
      className={`frame-nav__link ${on ? "is-on" : ""}`}
      aria-current={on ? "page" : undefined}
      aria-label={rail ? s.label : undefined}
      onClick={() => nav.go(s.route)}
    >
      <span className="frame-nav__icon" aria-hidden="true">
        {s.icon}
      </span>
      {!rail && <span className="frame-nav__label truncate">{s.label}</span>}
    </button>
  );
  return <li>{rail ? <Tooltip label={s.label} placement="right">{link}</Tooltip> : link}</li>;
}

/** Your playlists, most recently touched first, as plain rows. */
function NavPlaylists() {
  useLibraryLoader();
  const items = useLibraryView((s) => s.items);
  const recents = useLibraryView((s) => s.recents);
  const owner = useApp((s) => s.auth?.username ?? null);
  const current = useCurrentEntry().route;
  const rows = useMemo(() => {
    const all = toEntries(items ?? [], owner).filter((e) => e.kind === "playlist");
    const at = (id: string) => recents[`playlist:${id}`] ?? 0;
    return all.sort((a, b) => at(b.id) - at(a.id) || a.sortTitle.localeCompare(b.sortTitle));
  }, [items, recents, owner]);
  if (items === null) {
    return (
      <ul className="frame-nav__list frame-nav__list--playlists" aria-busy="true" aria-label="Playlists">
        {[0, 1, 2, 3].map((i) => (
          <li key={i} className="frame-nav__skeleton" />
        ))}
      </ul>
    );
  }
  if (!rows.length) return <p className="frame-nav__empty">Playlists you make show here.</p>;
  return (
    <ul className="frame-nav__list frame-nav__list--playlists" aria-label="Playlists">
      {rows.map((e) => {
        const on = current.name === "playlist" && current.id === e.id;
        return (
          <li key={e.id}>
            <button
              type="button"
              className={`frame-nav__link frame-nav__link--playlist ${on ? "is-on" : ""}`}
              aria-current={on ? "page" : undefined}
              onClick={() => openEntry(e)}
              onContextMenu={(ev) => {
                ev.preventDefault();
                openContextMenu(`Options for ${e.title}`, entryMenu(e, { playing: false }), ev);
              }}
            >
              <span className="frame-nav__label truncate">{e.title}</span>
            </button>
          </li>
        );
      })}
    </ul>
  );
}

/** The navigation component: sections, then your library, then playlists, with Settings at the foot. */
function FrameNav({ rail, withSearch, place }: { rail: boolean; withSearch: boolean; place: "left" | "top" }) {
  const current = useCurrentEntry().route;
  const { main, library } = useSections();
  if (place === "top") {
    return (
      <nav className="frame-keys" aria-label="Main">
        <ul className="frame-keys__list">
          {[...main, ...library.slice(1, 3)].map((s) => (
            <SectionLink key={s.key} s={s} rail={false} current={current} />
          ))}
          <SectionLink
            s={{ key: "settings", label: "Settings", icon: <SettingsIcon size={18} />, route: { name: "settings" }, on: (r) => r.name === "settings" }}
            rail={false}
            current={current}
          />
        </ul>
      </nav>
    );
  }
  return (
    <nav className={`frame-nav ${rail ? "is-rail" : ""}`} aria-label="Main">
      <div className="frame-nav__head" data-tauri-drag-region>
        {chrome !== "mac" && (
          <span className="frame-nav__logo" data-tauri-drag-region>
            <Logo size={22} />
          </span>
        )}
      </div>
      {/* On the rail, the Search section is the way to search. */}
      {withSearch && !rail && (
        <div className="frame-nav__search">
          <SearchBox />
        </div>
      )}
      <div className="frame-nav__scroll">
        <ul className="frame-nav__list" aria-label="Sections">
          {main.map((s) => (
            <SectionLink key={s.key} s={s} rail={rail} current={current} />
          ))}
        </ul>
        {!rail && <h2 className="frame-nav__heading">Your library</h2>}
        <ul className="frame-nav__list" aria-label="Your library">
          {library.map((s) => (
            <SectionLink key={s.key} s={s} rail={rail} current={current} />
          ))}
        </ul>
        {!rail && (
          <>
            <h2 className="frame-nav__heading">Playlists</h2>
            <NavPlaylists />
          </>
        )}
      </div>
      <div className="frame-nav__foot">
        <ul className="frame-nav__list">
          <SectionLink
            s={{ key: "settings", label: "Settings", icon: <SettingsIcon size={18} />, route: { name: "settings" }, on: (r) => r.name === "settings" }}
            rail={rail}
            current={current}
          />
        </ul>
        <StatusDot compact={rail} />
      </div>
    </nav>
  );
}

/** Now playing on its own (a frame that put it away from the transport). */
function NowPlayingSpot() {
  return <TransportStrip place="left" />;
}

// ---------------------------------------------------------------- bars

/**
 * The window's own row over the page: back and forward, then the account,
 * What's New and, on Windows, the window buttons. It's the drag area.
 */
function FrameBar() {
  const canBack = useCanGoBack();
  const canForward = useCanGoForward();
  return (
    <header className={`frame-bar frame-bar--${chrome}`} data-tauri-drag-region>
      <div className="frame-bar__start" data-tauri-drag-region>
        <IconButton className="topbar__nav" label="Go back" tooltip="Go back (Alt+←)" tooltipPlacement="bottom" icon={<ChevronLeft size={20} />} disabled={!canBack} onClick={() => nav.back()} />
        <IconButton
          className="topbar__nav"
          label="Go forward"
          tooltip="Go forward (Alt+→)"
          tooltipPlacement="bottom"
          icon={<ChevronRight size={20} />}
          disabled={!canForward}
          onClick={() => nav.forward()}
        />
      </div>
      <div className="frame-bar__end" data-tauri-drag-region>
        <OfflineIndicator />
        <WhatsNewButton />
        <FriendActivityButton />
        <AppMenu />
        {chrome === "windows" && <WindowControls />}
      </div>
    </header>
  );
}

/** A top or bottom region: navigation keys and search on one row, the strip on another. */
function EdgeRegion({ place, items }: { place: "top" | "bottom"; items: FrameComponent[] }) {
  const nav = items.includes("navigation");
  const search = items.includes("search");
  const transport = items.includes("transport");
  return (
    <div className={`frame-edge frame-edge--${place}`}>
      {(nav || search) && (
        <div className="frame-edge__row">
          {nav && <FrameNav rail={false} withSearch={false} place="top" />}
          {search && (
            <div className="frame-edge__search">
              <SearchBox />
            </div>
          )}
        </div>
      )}
      {transport && <TransportStrip place={place} showNow={items.includes("nowPlaying")} />}
    </div>
  );
}

/** The left region: navigation (with search at its top), or the player as a column. */
function LeftRegion({ items, solve }: { items: FrameComponent[]; solve: FrameSolve }) {
  const rail = solve.leftMode === "rail";
  const hasNav = items.includes("navigation");
  return (
    <div className={`frame-left ${rail ? "is-rail" : ""}`}>
      {hasNav && <FrameNav rail={rail} withSearch={items.includes("search")} place="left" />}
      {!hasNav && items.includes("search") && !rail && (
        <div className="frame-nav__search">
          <SearchBox />
        </div>
      )}
      {items.includes("transport") ? <TransportStrip place="left" showNow={items.includes("nowPlaying")} /> : items.includes("nowPlaying") && <NowPlayingSpot />}
    </div>
  );
}

// ---------------------------------------------------------------- panels

const CLOSED = { queueOpen: false, nowPlayingOpen: false, outputOpen: false, activityOpen: false, helpOpen: false, outputReturn: null };

function Panel({ view }: { view: RightView }) {
  return view === "help" ? <LazyHelpPanel /> : view === "output" ? <OutputPanel /> : view === "activity" ? <FriendActivityPanel /> : view === "queue" ? <QueuePanel /> : <NowPlayingPanel />;
}

/**
 * Where an open panel goes: the region the arrangement gives it (the queue
 * and now playing by name; the device picker, friend activity and help go
 * with the queue), shown as a column, a drawer, or in place of the page.
 */
function panelPlace(r: Regions, view: RightView, solve: FrameSolve): "column" | "drawer" | "main" {
  const comp: FrameComponent = view === "nowPlaying" ? "nowPlaying" : "queue";
  let region: RegionName | null = regionOf(r, comp);
  if (view === "nowPlaying" && region !== "right" && region !== "overlay") region = regionOf(r, "queue");
  if (region === "main") return "main";
  if (region === "right") return solve.rightMode;
  return "drawer";
}

/** A panel over the page from the right edge. Escape or the backdrop closes it. */
function Drawer({ view, swap }: { view: RightView; swap: boolean }) {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape" || e.defaultPrevented) return;
      // A menu, dialog or the search list inside handles its own Escape first.
      if (document.querySelector("[role='dialog'][aria-modal='true'], [role='menu']")) return;
      useUi.setState(CLOSED);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);
  return (
    <div className={`frame-drawer ${swap ? "is-swap" : ""}`} data-testid="frame-drawer">
      <div className="frame-drawer__scrim" aria-hidden="true" onClick={() => useUi.setState(CLOSED)} />
      <div className="frame-drawer__panel">
        <Panel view={view} />
      </div>
    </div>
  );
}

// ---------------------------------------------------------------- the host

/** Every frame but Classic. */
export function FrameShell({ children }: { children: ReactNode }) {
  const layout = useLayout();
  const regions = frameRegions(layout) as Regions;
  const width = useWindowWidth();
  const solve = solveFrame({ windowWidth: width, regions, leftSize: layout.frame === "custom" ? layout.leftSize : "medium", rightSize: layout.rightSize });
  const shell = useRef<HTMLDivElement>(null);
  useShellTabOrder(shell);

  const queueOpen = useUi((s) => s.queueOpen);
  const nowPlayingOpen = useUi((s) => s.nowPlayingOpen);
  const outputOpen = useUi((s) => s.outputOpen);
  const activityOpen = useUi((s) => s.activityOpen);
  const helpOpen = useUi((s) => s.helpOpen);
  const view = slotView({ queueOpen, nowPlayingOpen, outputOpen, activityOpen, helpOpen });
  const [slot, setSlot] = useState<{ view: RightView | null; swap: boolean }>({ view, swap: false });
  if (slot.view !== view) setSlot({ view, swap: slot.view !== null && view !== null });
  const place = view ? panelPlace(regions, view, solve) : null;

  const style = { "--frame-left": `${solve.left}px`, "--frame-right": `${solve.right}px` } as CSSProperties;
  const top = regions.top;
  const bottom = regions.bottom;
  const left = regions.left;
  const mainSearch = regions.main.includes("search");

  return (
    <div
      ref={shell}
      className={`app app--chrome-${chrome} frame frame--${layout.frame} ${left.length ? "has-left" : ""} ${place === "column" ? "has-right" : ""}`}
      style={style}
      data-frame={layout.frame}
    >
      <a
        href="#main"
        className="skip-link"
        onClick={(e) => {
          e.preventDefault();
          skipToMain();
        }}
      >
        Skip to main content
      </a>
      {left.length > 0 && <LeftRegion items={left} solve={solve} />}
      <div className="frame-center">
        <FrameBar />
        {top.length > 0 && <EdgeRegion place="top" items={top} />}
        <div className="frame-page">
          {mainSearch && (
            <div className="frame-page__search">
              <SearchBox />
            </div>
          )}
          {place === "main" && view ? (
            <div className="frame-page__panel">
              <Panel view={view} />
            </div>
          ) : (
            <MainView />
          )}
          {/* The drawer opens over the page only, so the strip and sections stay in reach. */}
          {place === "drawer" && view && <Drawer view={view} swap={slot.swap} />}
        </div>
        <RemoteStrip />
        <ListenStrip />
        {bottom.length > 0 && <EdgeRegion place="bottom" items={bottom} />}
      </div>
      {place === "column" && view && (
        <div className={`frame-right ${slot.swap ? "right-swap" : ""}`}>
          <Panel view={view} />
        </div>
      )}
      {children}
    </div>
  );
}

