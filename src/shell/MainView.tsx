import { ChevronLeft, ChevronRight } from "lucide-react";
import { useEffect, useLayoutEffect, useRef, useState, type CSSProperties } from "react";
import { Banner } from "../components/Banner";
import { IconButton } from "../components/Button";
import { SearchField } from "../components/SearchField";
import { backend } from "../lib/backend";
import type { Health, SyncStatus } from "../lib/types";
import { AlbumPage } from "../pages/AlbumPage";
import { ArtistPage } from "../pages/ArtistPage";
import { HomePage } from "../pages/HomePage";
import { LibraryPage } from "../pages/LibraryPage";
import { PlaylistPage } from "../pages/PlaylistPage";
import { SearchPage } from "../pages/SearchPage";
import { useApp } from "../stores/app";
import { nav, useCanGoBack, useCanGoForward, useCurrentEntry, useRouter, type Route } from "../stores/router";
import { useUi } from "../stores/ui";

/** Scroll offset per history entry, so Back returns to where you were. */
const scrollMemory = new Map<number, number>();

function healthBanner(h: Health | null): { kind: "info" | "warning" | "error" | "offline" | "progress"; message: string } | null {
  if (!h) return null;
  if (h.status === "offline") return { kind: "offline", message: h.banner?.message ?? "Your server is offline." };
  if (h.status === "starting") return { kind: "progress", message: h.banner?.message ?? "Server is starting…" };
  if (h.status === "updating") return { kind: "progress", message: h.banner?.message ?? "Server is updating" };
  if (!h.filesAvailable) return { kind: "error", message: "Your server is up, but your music files aren't available" };
  if (h.banner) return { kind: h.status === "relay" ? "warning" : "info", message: h.banner.message };
  if (h.scan) return { kind: "progress", message: `Adding new music… ${h.scan.done.toLocaleString()} of ${h.scan.total.toLocaleString()}` };
  return null;
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
  }
}

function HeaderSearch({ query }: { query: string }) {
  const tick = useUi((s) => s.searchFocusTick);
  const ref = useRef<HTMLInputElement>(null);
  useEffect(() => {
    if (tick > 0) ref.current?.focus();
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
  const health = useApp((s) => s.health);
  const sync = useApp((s) => s.sync);
  const tint = useUi((s) => s.tint);
  const tick = useUi((s) => s.searchFocusTick);
  const scroller = useRef<HTMLDivElement>(null);
  const [scrolled, setScrolled] = useState(false);

  // Ctrl/⌘+L → go to Search (the header field focuses itself).
  useEffect(() => {
    if (tick > 0 && useRouter.getState().entries[useRouter.getState().index].route.name !== "search") nav.go({ name: "search" });
  }, [tick]);

  // Restore scroll for this history entry.
  useLayoutEffect(() => {
    const el = scroller.current;
    if (!el) return;
    const top = scrollMemory.get(entry.key) ?? 0;
    el.scrollTop = top;
    // Content may still be loading; try again next frame.
    const raf = requestAnimationFrame(() => {
      if (scroller.current) scroller.current.scrollTop = top;
    });
    setScrolled(top > 8);
    return () => cancelAnimationFrame(raf);
  }, [entry.key]);

  // Pages without artwork reset the tint.
  useEffect(() => {
    const n = entry.route.name;
    if (n !== "album" && n !== "artist" && n !== "playlist") useUi.getState().setTint(null);
  }, [entry.route.name]);

  const banner = healthBanner(health);
  const syncText = syncLabel(sync);
  const r = entry.route;

  return (
    <main className="main" style={{ "--tint": tint ?? "#2a2520" } as CSSProperties} aria-label="Main content">
      {banner && (
        <div className="main__banner">
          <Banner kind={banner.kind}>{banner.message}</Banner>
        </div>
      )}
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
          {r.name === "search" && <HeaderSearch query={r.query ?? ""} />}
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
            const top = e.currentTarget.scrollTop;
            scrollMemory.set(entry.key, top);
            const s = top > 8;
            if (s !== scrolled) setScrolled(s);
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
