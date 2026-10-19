/**
 * The phone layout: one page at a time, the floating mini player and four
 * tabs along the bottom, now playing over everything, and the bottom sheets.
 * It is a different arrangement of the same stores and core calls the
 * desktop uses; nothing here talks to a server.
 */
import { CircleUserRound, Home, Library, Search } from "lucide-react";
import { lazy, Suspense, useEffect } from "react";
import { afterFirstPaint } from "../lib/idle";
import { ShellHosts } from "../shell/AppShell";
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
import { TABS, phone, tabOf, usePhoneUi, type PhoneTab } from "./phoneUi";
import { SheetHost } from "./Sheets";
import "./phone.css";

const SettingsPage = lazy(() => import("../pages/SettingsPage").then((m) => ({ default: m.SettingsPage })));

const TAB_ICON: Record<PhoneTab, typeof Home> = { home: Home, search: Search, library: Library, you: CircleUserRound };

/** The browser mock's stand-in for a notch and home indicator (`?notch=1`). */
export const NOTCH = typeof window !== "undefined" && notchFromUrl(window.location.search);

export function PhoneShell() {
  const entry = useCurrentEntry();
  const tab = usePhoneUi((s) => s.tab);
  const playerOpen = usePhoneUi((s) => s.nowPlaying);
  const hasTrack = usePlayer((s) => !!s.state?.track);
  // On Home, the big now-playing card stands in for the mini player.
  const cardShown = usePhoneUi((s) => s.homeCard) && entry.route.name === "home";
  useLibraryRecents();
  useEffect(() => void loadOwnLyricsIds(), []);
  useEffect(() => void startDownloads(), []);
  useEffect(() => afterFirstPaint(() => void startLyricsTrust()), []);
  // A page reached from elsewhere (a link in a sheet) lights its own tab.
  useEffect(() => {
    const t = tabOf(entry.route, usePhoneUi.getState().tab);
    if (t !== usePhoneUi.getState().tab) usePhoneUi.setState({ tab: t });
  }, [entry.route]);
  return (
    <div className={`phone ${hasTrack ? "has-mini" : ""} ${NOTCH ? "ph-notch" : ""}`} data-testid="phone-shell" data-tab={tab}>
      <main className="ph-main" aria-hidden={playerOpen || undefined} inert={playerOpen || undefined}>
        <PhonePage key={entry.key} route={entry.route} />
      </main>
      <div className={`ph-dock ${cardShown ? "is-mini-away" : ""}`} aria-hidden={playerOpen || undefined} inert={playerOpen || undefined}>
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
                  tap();
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
      <ShellHosts phone />
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
      return (
        <div className="ph-page">
          <BackBar />
          <div className="ph-empty">
            <p className="ph-empty__title">Not on the phone yet</p>
            <p className="ph-empty__text">This page opens in Platter on your computer for now.</p>
          </div>
        </div>
      );
  }
}
