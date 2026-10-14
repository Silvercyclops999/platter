import { ChevronLeft, ChevronRight, Pause, Play, Settings as SettingsIcon } from "lucide-react";
import { useEffect, useLayoutEffect, useRef, useState, type CSSProperties } from "react";
import { IconButton } from "../components/Button";
import { SearchField } from "../components/SearchField";
import { backend } from "../lib/backend";
import { modKey } from "../lib/platform";
import type { SyncStatus } from "../lib/types";
import { AlbumPage } from "../pages/AlbumPage";
import { ArtistPage } from "../pages/ArtistPage";
import { HomePage } from "../pages/HomePage";
import { LibraryPage } from "../pages/LibraryPage";
import { PlaylistPage } from "../pages/PlaylistPage";
import { SearchPage } from "../pages/SearchPage";
import { SettingsPage } from "../pages/SettingsPage";
import { useApp } from "../stores/app";
import { nav, useCanGoBack, useCanGoForward, useCurrentEntry, useRouter, type Route } from "../stores/router";
import { useUi } from "../stores/ui";
import { HealthBanner } from "./HealthBanner";

/** Scroll offset per history entry, so Back returns to where you were. */
const scrollMemory = new Map<number, number>();

/** Height of the floating main header. */
const HEADER_H = 64;

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
      return <ArtistPage id={route.id} />;
    case "album":
      return <AlbumPage id={route.id} />;
    case "playlist":
      return <PlaylistPage id={route.id} />;
    case "settings":
      return <SettingsPage />;
  }
}

function HeaderSearch({ query }: { query: string }) {
  const tick = useUi((s) => s.searchFocusTick);
  const ref = useRef<HTMLInputElement>(null);
  useEffect(() => {
    if (tick > 0) {
      ref.current?.focus();
      ref.current?.select();
    }
  }, [tick]);
  useEffect(() => {
    // Focus on arrival at Search.
    ref.current?.focus();
  }, []);
  return (
    <SearchField
      ref={ref}
      value={query}
      onChange={(v) => useRouter.getState().replace({ name: "search", query: v })}
      onSubmit={(v) => v.trim() && backend.recentSearchAdd(v.trim())}
      className="main-header__search"
    />
  );
}

export function MainView() {
  const entry = useCurrentEntry();
  const canBack = useCanGoBack();
  const canForward = useCanGoForward();
  const sync = useApp((s) => s.sync);
  const tint = useUi((s) => s.tint);
  const tick = useUi((s) => s.searchFocusTick);
  const scroller = useRef<HTMLDivElement>(null);
  const [scrolled, setScrolled] = useState(false);
  const [pastHeader, setPastHeader] = useState(false);
  const pageBar = useUi((s) => s.pageBar);
  const restoring = useRef(false);

  // Ctrl/⌘+L → go to Search (the header field focuses itself).
  useEffect(() => {
    if (tick > 0 && useRouter.getState().entries[useRouter.getState().index].route.name !== "search") nav.go({ name: "search" });
  }, [tick]);

  // Restore scroll for this history entry.
  useLayoutEffect(() => {
    const el = scroller.current;
    if (!el) return;
    const top = scrollMemory.get(entry.key) ?? 0;
    restoring.current = true;
    setScrolled(top > 8);
    setPastHeader(false);
    const cancel = restoreScroll(el, top, () => {
      restoring.current = false;
      if (scroller.current) {
        const now = scroller.current.scrollTop;
        scrollMemory.set(entry.key, now);
        setScrolled(now > 8);
        setPastHeader(now > stickyThreshold(scroller.current));
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

  return (
    <main className="main" style={{ "--tint": tint ?? "#2a2520" } as CSSProperties} aria-label="Main content">
      <HealthBanner />
      <div className="main__viewport">
        <div className={`main-header ${scrolled ? "is-scrolled" : ""} ${tint ? "is-tinted" : ""}`}>
          <div className="main-header__nav">
            <IconButton label="Go back" tooltip="Go back (Alt+←)" tooltipPlacement="bottom" variant="filled" icon={<ChevronLeft size={20} />} disabled={!canBack} onClick={() => nav.back()} />
            <IconButton
              label="Go forward"
              tooltip="Go forward (Alt+→)"
              tooltipPlacement="bottom"
              variant="filled"
              icon={<ChevronRight size={20} />}
              disabled={!canForward}
              onClick={() => nav.forward()}
            />
          </div>
          {pageBar && pastHeader && r.name !== "search" && (
            <div className="main-header__bar" data-testid="sticky-bar">
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
          {r.name === "search" && <HeaderSearch query={r.query ?? ""} />}
          <div className="main-header__end">
            {syncText && (
              <span className="sync-pill" role="status">
                <span className="sync-pill__dot" aria-hidden="true" />
                {syncText}
              </span>
            )}
            <IconButton
              label="Settings"
              tooltip={`Settings (${modKey}+,)`}
              tooltipPlacement="bottom"
              variant="filled"
              aria-current={r.name === "settings" ? "page" : undefined}
              icon={<SettingsIcon size={18} />}
              onClick={() => nav.go({ name: "settings" })}
            />
          </div>
        </div>
        <div
          className="main__scroll"
          ref={scroller}
          onScroll={(e) => {
            const top = e.currentTarget.scrollTop;
            if (restoring.current) return;
            scrollMemory.set(entry.key, top);
            const s = top > 8;
            if (s !== scrolled) setScrolled(s);
            const past = top > stickyThreshold(e.currentTarget);
            if (past !== pastHeader) setPastHeader(past);
          }}
        >
          <div className="page" key={entry.key}>
            <Page route={r} />
          </div>
        </div>
      </div>
    </main>
  );
}
