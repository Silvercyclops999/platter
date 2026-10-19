/**
 * The phone layout: one page at a time, the floating mini player and four
 * tabs along the bottom, now playing over everything, and the bottom sheets.
 * It is a different arrangement of the same stores and core calls the
 * desktop uses; nothing here talks to a server.
 */
import { CircleUserRound, Home, Library, Search } from "lucide-react";
import { Suspense, useEffect, useRef } from "react";
import { backend } from "../lib/backend";
import { afterFirstPaint } from "../lib/idle";
import { SettingsPage } from "../shell/lazyPages";
import { ShellHosts } from "../shell/ShellHosts";
import { startLyricsTrust } from "../shell/lyrics";
import { useLibraryRecents } from "../shell/useLibrary";
import { startDownloads } from "../stores/downloads";
import { loadOwnLyricsIds } from "../stores/lyricsEditor";
import { useCurrentEntry, type Route } from "../stores/router";
import { usePlayer } from "../stores/player";
import { tap } from "./bits";
import { LyricsScreen } from "./LyricsScreen";
import { MiniPlayer } from "./MiniPlayer";
import { NowPlaying } from "./NowPlaying";
import { PhoneAlbum } from "./pages/Album";
import { PhoneArtist } from "./pages/Artist";
import { BackBar, PageLoading } from "./pages/common";
import { PhoneCrate, PhoneCrates } from "./pages/Crates";
import { PhoneHome } from "./pages/Home";
import { PhoneLibrary } from "./pages/Library";
import { PhoneLiked, PhonePlaylist } from "./pages/Playlist";
import { PhoneSearch } from "./pages/Search";
import { PhoneYou } from "./pages/You";
import { notchFromUrl } from "./form";
import { followDock } from "./dockInset";
import { installTapGuard } from "./gestures";
import { TABS, phone, tabOf, usePhoneUi, type PhoneTab } from "./phoneUi";
import { PageStack } from "./PageStack";
import { PhoneRoutePage } from "./routes";
import { SheetHost } from "./Sheets";
import "./phone.css";


const TAB_ICON: Record<PhoneTab, typeof Home> = { home: Home, search: Search, library: Library, you: CircleUserRound };

/** The browser mock's stand-in for a notch and home indicator (`?notch=1`). */
export const NOTCH = typeof window !== "undefined" && notchFromUrl(window.location.search);

export function PhoneShell() {
  const entry = useCurrentEntry();
  const tab = usePhoneUi((s) => s.tab);
  const playerOpen = usePhoneUi((s) => s.nowPlaying);
  const hasTrack = usePlayer((s) => !!s.state?.track);
  const root = useRef<HTMLDivElement>(null);
  useEffect(() => (root.current ? installTapGuard(root.current) : undefined), []);
  // Pages pad their foot by what the dock really covers (mini player or not).
  const dock = useRef<HTMLDivElement>(null);
  useEffect(() => (root.current && dock.current ? followDock(root.current, dock.current) : undefined), [hasTrack]);
  useLibraryRecents();
  useEffect(() => void loadOwnLyricsIds(), []);
  useEffect(() => void startDownloads(), []);
  useEffect(() => afterFirstPaint(() => void startLyricsTrust()), []);
  useEffect(() => followStatusBar(), []);
  // A page reached from elsewhere (a link in a sheet) lights its own tab.
  useEffect(() => {
    const t = tabOf(entry.route, usePhoneUi.getState().tab);
    if (t !== usePhoneUi.getState().tab) usePhoneUi.setState({ tab: t });
  }, [entry.route]);
  return (
    <div ref={root} className={`phone ${hasTrack ? "has-mini" : ""} ${NOTCH ? "ph-notch" : ""}`} data-testid="phone-shell" data-tab={tab}>
      <main className="ph-main" aria-hidden={playerOpen || undefined} inert={playerOpen || undefined}>
        <PageStack render={(e) => <PhonePage route={e.route} />} />
      </main>
      <div ref={dock} className="ph-dock" aria-hidden={playerOpen || undefined} inert={playerOpen || undefined}>
        <MiniPlayer />
        <nav className="ph-tabs" aria-label="Tabs">
          {TABS.map((t) => {
            const Icon = TAB_ICON[t.id];
            const on = tab === t.id;
            return (
              <button
                key={t.id}
                type="button"
                className={`ph-tab ${on ? "is-on" : ""}`}
                aria-current={on ? "page" : undefined}
                onClick={() => {
                  tap("selection");
                  phone.tab(t.id);
                }}
              >
                <Icon size={24} aria-hidden="true" strokeWidth={on ? 2.25 : 1.75} />
                <span className="ph-tab__label">{t.label}</span>
              </button>
            );
          })}
        </nav>
      </div>
      <NowPlaying />
      <LyricsScreen />
      <SheetHost />
      <ShellHosts />
    </div>
  );
}

/** The page for a route. Pages that have no phone version yet say so plainly. */
export function PhonePage({ route }: { route: Route }) {
  switch (route.name) {
    case "home":
      return <PhoneHome />;
    case "search":
      return <PhoneSearch />;
    case "library":
      return <PhoneLibrary />;
    case "crates":
      return <PhoneCrates kind={route.kind} />;
    case "crate":
      return <PhoneCrate kind={route.kind} crateKey={route.key} />;
    case "album":
      return <PhoneAlbum id={route.id} />;
    case "artist":
      return <PhoneArtist id={route.id} />;
    case "playlist":
      return <PhonePlaylist id={route.id} />;
    case "liked":
      return <PhoneLiked />;
    case "profile":
      return <PhoneYou />;
    case "settings":
      return (
        <div className="ph-page ph-settings">
          <BackBar />
          <Suspense fallback={<PageLoading />}>
            <SettingsPage page={route.page} />
          </Suspense>
        </div>
      );
    default:
      return <PhoneRoutePage route={route} />;
  }
}

/** The status bar's text for a look: dark on a light look, light on a dark one. */
export function statusBarDark(base: string | undefined): boolean {
  return base === "light";
}

/**
 * Keeps the phone's status bar readable: told once at the start and again
 * whenever the look turns light or dark. Off the phone it quietly does nothing.
 */
export function followStatusBar(root: HTMLElement = document.documentElement): () => void {
  let last: boolean | null = null;
  const tell = () => {
    const dark = statusBarDark(root.dataset.themeBase);
    if (dark === last) return;
    last = dark;
    try {
      void backend.setStatusBarStyle(dark).catch(() => undefined);
    } catch {
      // No status bar here.
    }
  };
  tell();
  if (typeof MutationObserver === "undefined") return () => {};
  const mo = new MutationObserver(tell);
  mo.observe(root, { attributes: true, attributeFilter: ["data-theme-base"] });
  return () => mo.disconnect();
}

