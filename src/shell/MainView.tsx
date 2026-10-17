import { Pause, Play } from "lucide-react";
import { lazy, Suspense, useEffect, useLayoutEffect, useRef, useState, type CSSProperties } from "react";
import { IconButton } from "../components/Button";
import { NEUTRAL_HEADER, THEMED_NEUTRAL_HEADER, themedHeader } from "../lib/color";
import { useTheme } from "../stores/theme";
import type { SyncStatus } from "../lib/types";
import { AlbumPage } from "../pages/AlbumPage";
import { ArtistPage } from "../pages/ArtistPage";
import { DiscographyPage } from "../pages/DiscographyPage";
import { HomePage } from "../pages/HomePage";
import { ArtistShelfPage, HomeShelfPage } from "../pages/HomeShelfPage";
import { LibraryPage } from "../pages/LibraryPage";
import { LikedPage } from "../pages/LikedPage";
import { MixPage } from "../pages/MixPage";
import { PlaylistPage } from "../pages/PlaylistPage";
import { SharedPlaylistPage } from "../pages/SharedPlaylistPage";
import { BlendPage } from "../pages/BlendPage";
import { BlendCreatePage } from "../pages/BlendCreatePage";
import { RecentsPage } from "../pages/RecentsPage";
import { SearchPage } from "../pages/SearchPage";
import { NewToYouPage } from "../pages/NewToYou";
import { ShelfPage } from "../pages/ShelfPage";
import { useApp } from "../stores/app";
import { useCurrentEntry, type Route } from "../stores/router";
import { useUi } from "../stores/ui";
import { HealthBanner } from "./HealthBanner";
import { DownloadsBanner } from "../components/DownloadButton";

// Rarely visited pages load on first visit, so their code isn't parsed at startup.
const BookmarksPage = lazy(() => import("../pages/BookmarksPage").then((m) => ({ default: m.BookmarksPage })));
const ImportPage = lazy(() => import("../pages/ImportPage").then((m) => ({ default: m.ImportPage })));
const SmartPlaylistPage = lazy(() => import("../pages/SmartPlaylistPage").then((m) => ({ default: m.SmartPlaylistPage })));
const SettingsPage = lazy(() => import("../pages/SettingsPage").then((m) => ({ default: m.SettingsPage })));
const StatsPage = lazy(() => import("../pages/StatsPage").then((m) => ({ default: m.StatsPage })));
const ProfilePage = lazy(() => import("../pages/ProfilePage").then((m) => ({ default: m.ProfilePage })));
const TopSongsPage = lazy(() => import("../pages/TopSongsPage").then((m) => ({ default: m.TopSongsPage })));
const WhatsNewPage = lazy(() => import("../pages/WhatsNewPage").then((m) => ({ default: m.WhatsNewPage })));
const ReleaseRadarPage = lazy(() => import("../pages/ReleaseRadar").then((m) => ({ default: m.ReleaseRadarPage })));
const LineagePage = lazy(() => import("../pages/LineagePage").then((m) => ({ default: m.LineagePage })));
const EraPage = lazy(() => import("../pages/EraPage").then((m) => ({ default: m.EraPage })));
const DoctorPage = lazy(() => import("../pages/DoctorPage").then((m) => ({ default: m.DoctorPage })));
const DevicesPage = lazy(() => import("../pages/DevicesPage").then((m) => ({ default: m.DevicesPage })));
const SoundMapPage = lazy(() => import("../pages/SoundMapPage").then((m) => ({ default: m.SoundMapPage })));

/** Scroll offset per history entry, so Back returns to where you were. */
const scrollMemory = new Map<number, number>();

/** Height of the floating main header. */
const HEADER_H = 64;

/** The bar's background starts fading in after this much scroll… */
const BAR_FADE_START = 16;
/** …and is fully opaque this much further down. */
const BAR_FADE_RANGE = 100;

/** Opacity of the main header's background for a scroll offset (0 to 1). */
export function barAlpha(scrollTop: number): number {
  return Math.min(1, Math.max(0, (scrollTop - BAR_FADE_START) / BAR_FADE_RANGE));
}

/**
 * Scroll offset past which a detail page's own play button has gone under
 * the main header, so the sticky title + play button take over.
 */
export function stickyThreshold(scroller: HTMLElement): number {
  const row = scroller.querySelector<HTMLElement>(".action-row");
  const t = row ? row.offsetTop + row.offsetHeight - HEADER_H : 0;
  return t > 0 ? t : 280;
}

/**
 * Restore a remembered scroll offset. Pages load their content
 * asynchronously, so keep trying each frame until the page is tall enough,
 * the user scrolls, or `maxMs` passes. Returns a cancel function.
 */
export function restoreScroll(el: HTMLElement, top: number, onDone: () => void, maxMs = 2000): () => void {
  el.scrollTop = top;
  if (top <= 0 || Math.abs(el.scrollTop - top) < 2) {
    onDone();
    return () => {};
  }
  const started = performance.now();
  let raf = 0;
  let live = true;
  const stop = () => {
    if (!live) return;
    live = false;
    cancelAnimationFrame(raf);
    el.removeEventListener("wheel", stop);
    el.removeEventListener("pointerdown", stop);
    el.removeEventListener("keydown", stop);
    onDone();
  };
  const step = () => {
    if (!live) return;
    el.scrollTop = top;
    if (Math.abs(el.scrollTop - top) < 2 || performance.now() - started > maxMs) stop();
    else raf = requestAnimationFrame(step);
  };
  // The user taking over cancels the restore.
  el.addEventListener("wheel", stop, { passive: true });
  el.addEventListener("pointerdown", stop);
  el.addEventListener("keydown", stop);
  raf = requestAnimationFrame(step);
  return stop;
}

/** The main region's name on pages with a fixed title; others use their heading. */
const PAGE_LABELS: Partial<Record<Route["name"], string>> = {
  home: "Home",
  search: "Search",
  library: "Your Library",
  liked: "Liked Songs",
  settings: "Settings",
  recents: "Recents",
  whatsNew: "What's New",
  releaseRadar: "Release radar",
  newToYou: "New to you",
  shelf: "Your shelf",
  stats: "Stats",
  profile: "Your profile",
  devices: "Devices",
  soundMap: "Sound map",
};

/**
 * The main region is named after the page ("Home", an album's title), so a
 * screen reader says where you are after moving. Pages that load their
 * title use their level 1 heading once it shows.
 */
function useMainLabel(route: Route, entryKey: number, page: React.RefObject<HTMLDivElement | null>): string {
  const [heading, setHeading] = useState<string | null>(null);
  const fixed = PAGE_LABELS[route.name];
  useEffect(() => {
    const el = page.current;
    setHeading(null);
    if (!el || fixed) return;
    const read = () => setHeading(el.querySelector("h1")?.textContent?.trim() || null);
    read();
    if (typeof MutationObserver === "undefined") return;
    const mo = new MutationObserver(read);
    mo.observe(el, { subtree: true, childList: true, characterData: true });
    return () => mo.disconnect();
  }, [entryKey, fixed, page]);
  return fixed ?? heading ?? "Main content";
}

function syncLabel(s: SyncStatus | null): string | null {
  if (!s || s.phase === "idle") return null;
  const what = s.phase === "incremental" ? "changes" : s.phase;
  return s.total > 0
    ? `Syncing library… ${s.done.toLocaleString()} of ${s.total.toLocaleString()} ${what}`
    : "Syncing library…";
}

function Page({ route }: { route: Route }) {
  switch (route.name) {
    case "home":
      return <HomePage />;
    case "search":
      return <SearchPage query={route.query ?? ""} />;
    case "library":
      return <LibraryPage tab={route.tab} />;
    case "artist":
      return <ArtistPage id={route.id} era={route.era} />;
    case "lineage":
      return <LineagePage key={route.id} id={route.id} />;
    case "era":
      return <EraPage key={`${route.id}:${route.era}`} artistId={route.id} name={route.era} />;
    case "discography":
      return <DiscographyPage id={route.id} type={route.type} />;
    case "album":
      return <AlbumPage id={route.id} />;
    case "catalogueArtist":
      return <ArtistPage id={route.mbid} catalogue />;
    case "catalogueAlbum":
      return <AlbumPage id={route.id} catalogue />;
    case "newToYou":
      return <NewToYouPage />;
    case "shelf":
      return <ShelfPage />;
    case "playlist":
      // A Platter smart playlist has its own page.
      if (route.id.startsWith("blend:")) return <BlendPage key={route.id} id={route.id} />;
      if (route.id.startsWith("shared:")) return <SharedPlaylistPage key={route.id} id={route.id} />;
      return route.id.startsWith("smart:") ? <SmartPlaylistPage id={route.id} /> : <PlaylistPage id={route.id} />;
    case "blendNew":
      return <BlendCreatePage />;
    case "bookmarks":
      return <BookmarksPage />;
    case "stats":
      return <StatsPage tab={route.tab} month={route.month} friend={route.friend} />;
    case "profile":
      return <ProfilePage />;
    case "topSongs":
      return <TopSongsPage key={route.year ?? "all"} year={route.year ?? null} />;
    case "doctor":
      return <DoctorPage />;
    case "devices":
      return <DevicesPage />;
    case "soundMap":
      return <SoundMapPage />;
    case "import":
      return <ImportPage key={route.view ?? "home"} view={route.view} />;
    case "liked":
      return <LikedPage />;
    case "settings":
      return <SettingsPage />;
    case "recents":
      return <RecentsPage />;
    case "whatsNew":
      return <WhatsNewPage />;
    case "releaseRadar":
      return <ReleaseRadarPage />;
    case "homeShelf":
      return <HomeShelfPage shelf={route.shelf} />;
    case "artistShelf":
      return <ArtistShelfPage id={route.id} shelf={route.shelf} />;
    case "mix":
      // Saved links to the old All-Time mix open the All-Time list.
      if (route.id === "topSongs") return <TopSongsPage key="all" year={null} />;
      return <MixPage key={`mix-${route.id}`} id={route.id} />;
    case "radio":
      return <MixPage key={`radio-${route.seed}`} seed={route.seed} />;
    case "lyrics":
      // Never pushed: the router opens the expanded view in lyrics mode instead.
      return null;
  }
}

export function MainView() {
  const entry = useCurrentEntry();
  const sync = useApp((s) => s.sync);
  const tint = useUi((s) => s.tint);
  const scroller = useRef<HTMLDivElement>(null);
  const topBar = useRef<HTMLDivElement>(null);
  const [pastHeader, setPastHeader] = useState(false);
  const pageBar = useUi((s) => s.pageBar);
  const restoring = useRef(false);
  const frame = useRef(0);
  const page = useRef<HTMLDivElement>(null);
  const mainLabel = useMainLabel(entry.route, entry.key, page);

  // Scroll-linked header state, at most once per frame: the bar background
  // goes straight to a CSS variable (no re-render); the sticky title only
  // re-renders when it crosses its threshold.
  const syncScroll = () => {
    const el = scroller.current;
    if (!el) return;
    const top = el.scrollTop;
    topBar.current?.style.setProperty("--bar-alpha", barAlpha(top).toFixed(3));
    setPastHeader(top > stickyThreshold(el));
  };
  const scheduleSync = () => {
    if (frame.current) return;
    frame.current = requestAnimationFrame(() => {
      frame.current = 0;
      syncScroll();
    });
  };
  useEffect(() => () => cancelAnimationFrame(frame.current), []);

  // Restore scroll for this history entry.
  useLayoutEffect(() => {
    const el = scroller.current;
    if (!el) return;
    const top = scrollMemory.get(entry.key) ?? 0;
    restoring.current = true;
    topBar.current?.style.setProperty("--bar-alpha", barAlpha(top).toFixed(3));
    setPastHeader(false);
    const cancel = restoreScroll(el, top, () => {
      restoring.current = false;
      if (scroller.current) {
        scrollMemory.set(entry.key, scroller.current.scrollTop);
        syncScroll();
      }
    });
    return cancel;
  }, [entry.key]);

  // Pages without artwork reset the tint.
  useEffect(() => {
    const n = entry.route.name;
    if (n !== "album" && n !== "artist" && n !== "playlist") useUi.getState().setTint(null);
  }, [entry.route.name]);

  const syncText = syncLabel(sync);
  const r = entry.route;
  // Light themes get pale header colours; pages without art use the theme's panels.
  const theme = useTheme((s) => s.resolved);
  const colors =
    !tint || tint === NEUTRAL_HEADER
      ? theme.id === "dark"
        ? NEUTRAL_HEADER
        : THEMED_NEUTRAL_HEADER[theme.base]
      : themedHeader(tint, theme.base);
  const barShown = !!pageBar && pastHeader;

  return (
    <main id="main" className="main" style={{ "--tint": colors.vivid, "--tint-shade": colors.shade } as CSSProperties} aria-label={mainLabel}>
      <HealthBanner />
      <DownloadsBanner />
      <div className="main__viewport">
        <div className="main-header" ref={topBar}>
          {pageBar && (
            <div
              className={`main-header__bar ${barShown ? "is-shown" : ""}`}
              data-testid="sticky-bar"
              aria-hidden={!barShown || undefined}
              inert={!barShown}
            >
              <IconButton
                label={`${pageBar.playing ? "Pause" : "Play"} ${pageBar.title}`}
                tooltip={false}
                variant="accent"
                size="lg"
                icon={pageBar.playing ? <Pause size={18} fill="currentColor" /> : <Play size={18} fill="currentColor" className="nudge-right" />}
                onClick={pageBar.onPlay}
              />
              <span className="main-header__title truncate">{pageBar.title}</span>
            </div>
          )}
          <div className="main-header__end">
            {syncText && (
              <span className="sync-pill" role="status">
                <span className="sync-pill__dot" aria-hidden="true" />
                {syncText}
              </span>
            )}
          </div>
        </div>
        <div
          className="main__scroll"
          ref={scroller}
          onScroll={(e) => {
            if (restoring.current) return;
            scrollMemory.set(entry.key, e.currentTarget.scrollTop);
            scheduleSync();
          }}
        >
          <div className="page" key={entry.key} ref={page}>
            <Suspense fallback={null}>
              <Page route={r} />
            </Suspense>
          </div>
        </div>
      </div>
    </main>
  );
}
