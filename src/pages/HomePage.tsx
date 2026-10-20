import { Disc3, WifiOff } from "lucide-react";
import { useEffect, useMemo, useRef, useState, type CSSProperties } from "react";
import { Button } from "../components/Button";
import { Shelf, ShelfItem } from "../components/Shelf";
import { FilterChips } from "../components/Tabs";
import { artUrl, backend } from "../lib/backend";
import { headerColorsFromImageUrl, headerColorsFromServer, lightHeader } from "../lib/color";
import { useThemeBase } from "../stores/theme";
import type { HomeItem, MixSummary } from "../lib/types";
import { useLoad } from "../lib/useLoad";
import { useApp } from "../stores/app";
import { useSettings } from "../stores/settings";
import { useLayout } from "../stores/theme";
import { nav } from "../stores/router";
import { HomeItemCard, homeItemInfo } from "./cards";
import {
  HOME_FILTERS,
  HOME_SHELVES,
  chipsAlpha,
  gridItems,
  greeting,
  offlineHome,
  offlineShelfAlbums,
  showsLikedTile,
  visibleShelves,
  type HomeFilter,
} from "./home";
import { useMixShelves } from "./mixShelves";
import { useCachedLibrary } from "./cachedLibrary";
import { LikedTile, ShortcutSkeleton, ShortcutTile } from "./ShortcutTile";
import { MixCard } from "./MixCard";
import { RadarShelf } from "./ReleaseRadar";
import { NewToYouShelf } from "./NewToYou";
import { OnThisDayShelf, TimeTravelShelf } from "./HistoryShelves";
import { TopSongsPromo } from "./TopSongsPromo";
import { useLikes } from "../stores/likes";
import "./home.css";
import { LoadError } from "../components/LoadError";
import { openSetting } from "./SoundMapPage";
import { HomeExplore } from "./HomeExplore";
import { CratesLink, RecordHero, RecordShelves, hasRecords, useRecordHome, useRecordsLead } from "./RecordHome";

/**
 * Colour for the wash at the top of Home: the artwork's most vivid tone
 * (the server's colour when the image can't be read). Keeps the previous
 * colour while the next one loads, so moving between tiles cross-fades.
 */
function useWashColour(item: HomeItem | undefined, enabled: boolean): string | null {
  const [colour, setColour] = useState<string | null>(null);
  const thumb = item ? homeItemInfo(item).thumb : null;
  const serverTint = item?.kind === "album" ? item.album.tint : null;
  const on = enabled && !!item;
  useEffect(() => {
    if (!on) return;
    let alive = true;
    const fallback = headerColorsFromServer(serverTint);
    const url = artUrl(thumb, 64);
    if (!url) {
      setColour(fallback?.vivid ?? null);
      return;
    }
    headerColorsFromImageUrl(url).then((c) => alive && setColour((c ?? fallback)?.vivid ?? null));
    return () => {
      alive = false;
    };
  }, [thumb, serverTint, on]);
  // Light themes wash with a pale tone of the same colour.
  const base = useThemeBase();
  if (!on || !colour) return null;
  return base === "light" ? lightHeader({ vivid: colour, shade: colour }).vivid : colour;
}

/** Chips row pinned under the main header; its backdrop fades in with the header's. */
function HomeChips({ value, onChange }: { value: HomeFilter; onChange: (f: HomeFilter) => void }) {
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const el = ref.current;
    const scroller = el?.closest<HTMLElement>(".main__scroll");
    if (!el || !scroller) return;
    let frame = 0;
    const sync = () => {
      frame = 0;
      el.style.setProperty("--chips-alpha", chipsAlpha(scroller.scrollTop).toFixed(3));
    };
    const onScroll = () => {
      if (!frame) frame = requestAnimationFrame(sync);
    };
    sync();
    scroller.addEventListener("scroll", onScroll, { passive: true });
    return () => {
      scroller.removeEventListener("scroll", onScroll);
      cancelAnimationFrame(frame);
    };
  }, []);
  return (
    <div className="home__chips" ref={ref}>
      <FilterChips label="Show on Home" items={HOME_FILTERS} value={value} onChange={onChange} />
    </div>
  );
}

/** Offline: whether any liked song is on this computer (true while that isn't known). */
function useLikedPlayable(cachedOnly: boolean): boolean {
  const ids = useLikes((s) => s.ids);
  const cached = useApp((s) => s.cached);
  return useMemo(() => {
    if (!cachedOnly || !cached) return true;
    for (const id of ids) if (cached.has(id)) return true;
    return false;
  }, [cachedOnly, cached, ids]);
}

function MixShelf({ title, mixes }: { title: string; mixes: MixSummary[] }) {
  if (!mixes.length) return null;
  return (
    <Shelf title={title} skeletonKind="album">
      {mixes.map((m) => (
        <ShelfItem key={m.id}>
          <MixCard mix={m} />
        </ShelfItem>
      ))}
    </Shelf>
  );
}

export function HomePage() {
  const { data: full, error, retry } = useLoad(() => backend.home(), []);
  const cachedOnly = useApp((s) => !!s.health?.cachedOnly);
  const { lib, failed: libFailed, retry: retryLib } = useCachedLibrary();
  // Offline: Home keeps to what can still play, and comes back whole on reconnect.
  const data = full && cachedOnly ? (lib ? offlineHome(full, lib) : undefined) : full;
  const offlineAlbums = cachedOnly && lib && data ? offlineShelfAlbums(lib, data) : [];
  const [filter, setFilter] = useState<HomeFilter>("all");
  const [hovered, setHovered] = useState<string | null>(null);
  const root = useRef<HTMLDivElement>(null);
  const showGrid = useSettings((s) => s.settings?.homeShortcuts !== false);
  const washOn = useSettings((s) => s.settings?.artworkColours !== false && s.settings?.homeColourWash !== false);
  const mixesOn = useSettings((s) => s.settings?.homeMixes !== false);
  // Offline, only mixes with downloaded songs.
  const { data: mixes, error: mixesError, retry: retryMixes } = useMixShelves(mixesOn);
  const radarOn = useSettings((s) => s.settings?.releaseRadar !== false);
  const showMixes = !!mixes && filter === "all";
  // Made for you holds its place while the mixes load (connected only).
  const mixesLoading = mixesOn && !mixes && !mixesError && !cachedOnly && filter === "all";
  // Platter's own looks lead with records (albums, so on All and Albums).
  const recordsOn = useRecordsLead();
  const records = useRecordHome(recordsOn);
  const recordsLead = recordsOn && hasRecords(records) && (filter === "all" || filter === "albums");

  // Offline, Liked Songs keeps its tile only while something liked can play
  // (and while that isn't known yet).
  const likedCount = useLikes((s) => s.ids.size);
  const likedPlayable = useLikedPlayable(cachedOnly);
  const liked = !!data && likedPlayable && showsLikedTile(filter, likedCount, showGrid);
  const look = useLayout();
  // Quick access grid size: Liked Songs takes one of the places.
  const tiles = data ? gridItems(data, filter, showGrid, liked).slice(0, look.quickAccess - (liked ? 1 : 0)) : [];
  const shelves = data ? visibleShelves(data, filter) : [];
  const washItem = tiles.find((t) => homeItemInfo(t).key === hovered) ?? tiles[0];
  // The hero carries its record's colour, so the wash stays off while records lead.
  const wash = useWashColour(washItem, washOn && !recordsLead);

  const pick = (f: HomeFilter) => {
    setFilter(f);
    setHovered(null);
    root.current?.closest(".main__scroll")?.scrollTo({ top: 0 });
  };

  const libraryEmpty =
    data && data.shortcuts.length === 0 && visibleShelves(data, "all").length === 0 && offlineAlbums.length === 0;
  const showOfflineShelf = offlineAlbums.length > 0 && (filter === "all" || filter === "albums");
  const filterEmpty = data && !libraryEmpty && tiles.length === 0 && !liked && shelves.length === 0;
  const kindLabel = HOME_FILTERS.find((f) => f.id === filter)?.label.toLowerCase();

  return (
    <div
      className={`page-pad home ${wash ? "has-wash" : ""}`}
      ref={root}
      style={wash ? ({ "--home-wash": wash } as CSSProperties) : undefined}
      data-testid="home"
    >
      <div className="home__wash" aria-hidden="true" />
      <h1 className={look.homeGreeting ? "home__greeting" : "sr-only"}>{greeting()}</h1>
      <HomeChips value={filter} onChange={pick} />
      {error && !data ? (
        <LoadError what="Home" error={error} onRetry={retry} inline />
      ) : libFailed && !data ? (
        <LoadError what="what's on this computer" onRetry={retryLib} inline />
      ) : libraryEmpty && cachedOnly ? (
        <div className="empty-state" role="status">
          <WifiOff size={40} strokeWidth={1.5} aria-hidden="true" />
          <h2 className="empty-state__title">Nothing on this computer yet</h2>
          <p className="empty-state__text">
            Platter can't reach your server right now. Songs you play are kept on this computer, so next time they can play here too.
          </p>
        </div>
      ) : libraryEmpty ? (
        <div className="empty-state" role="status">
          <Disc3 size={40} strokeWidth={1.5} aria-hidden="true" />
          <h2 className="empty-state__title">No music yet</h2>
          <p className="empty-state__text">Your library is empty or still syncing. Albums you add and play show up here.</p>
          <div className="empty-state__actions">
            <Button variant="secondary" size="sm" onClick={retry}>
              Try again
            </Button>
            <Button variant="ghost" size="sm" onClick={() => openSetting("servers")}>
              Check the server
            </Button>
          </div>
        </div>
      ) : (
        <>
          {recordsLead && records && (
            <div className="home__records" data-testid="home-records">
              {records.resume && <RecordHero progress={records.resume} />}
              <RecordShelves data={records} />
              <CratesLink />
            </div>
          )}
          {!data && showGrid ? (
            <div className="home-grid" aria-hidden="true">
              {Array.from({ length: look.quickAccess }, (_, i) => (
                <ShortcutSkeleton key={i} />
              ))}
            </div>
          ) : (
            (tiles.length > 0 || liked) && (
              <div className="home-grid" role="list" aria-label="Shortcuts" onPointerLeave={() => setHovered(null)}>
                {liked && <LikedTile onHover={() => setHovered(null)} />}
                {tiles.map((t) => {
                  const key = homeItemInfo(t).key;
                  return <ShortcutTile key={key} item={t} onHover={() => setHovered(key)} />;
                })}
              </div>
            )
          )}
          {filter === "all" && <TopSongsPromo />}
          {showOfflineShelf && (
            <Shelf title="On this computer" skeletonKind="album">
              {offlineAlbums.map((album) => (
                <ShelfItem key={`offline-${album.id}`}>
                  <HomeItemCard item={{ kind: "album", album }} />
                </ShelfItem>
              ))}
            </Shelf>
          )}
          {filterEmpty && !showOfflineShelf && (
            <p className="home__empty" role="status">
              No {kindLabel} to show yet.
            </p>
          )}
          {!data
            ? HOME_SHELVES.filter((s) => look.homeShelves.includes(s.id))
                .slice(0, 3)
                .map((s) => <Shelf key={s.id} title={s.title} loading />)
            : // The Layout option picks which shelves show, and their order.
              look.homeShelves.map((key) => {
                if (key === "madeForYou") {
                  if (mixesError && !mixes && filter === "all")
                    return <LoadError key={key} what="your mixes" error={mixesError} onRetry={retryMixes} inline />;
                  if (mixesLoading) return <Shelf key={key} title="Made for you" loading />;
                  return showMixes && <MixShelf key={key} title="Made for you" mixes={mixes.madeForYou} />;
                }
                if (key === "mixMoods") return showMixes && <MixShelf key={key} title="Your mixes by mood" mixes={mixes.moods} />;
                if (key === "mixGenres") return showMixes && <MixShelf key={key} title="Your genre mixes" mixes={mixes.genres} />;
                const s = shelves.find((x) => x.id === key);
                if (!s) return null;
                // New arrivals already lead the page.
                if (s.id === "recentlyAdded" && recordsLead && records?.newArrivals.length) return null;
                return (
                  <Shelf
                    key={s.id}
                    title={s.title}
                    skeletonKind={s.id === "topArtists" ? "artist" : "album"}
                    // Recently played's Show all is the Recents page.
                    onShowAll={() => nav.go(s.id === "recentlyPlayed" ? { name: "recents" } : { name: "homeShelf", shelf: s.id })}
                  >
                    {s.items.map((item) => (
                      <ShelfItem key={homeItemInfo(item).key}>
                        <HomeItemCard item={item} />
                      </ShelfItem>
                    ))}
                  </Shelf>
                );
              })}
          {data && radarOn && filter === "all" && <RadarShelf />}
          {data && filter === "all" && <NewToYouShelf />}
          {data && filter === "all" && <OnThisDayShelf />}
          {data && filter === "all" && <TimeTravelShelf />}
          {data && filter === "all" && <HomeExplore />}
        </>
      )}
    </div>
  );
}
