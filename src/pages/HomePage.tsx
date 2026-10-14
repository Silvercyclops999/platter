import { Disc3 } from "lucide-react";
import { useEffect, useRef, useState, type CSSProperties } from "react";
import { Button } from "../components/Button";
import { Shelf, ShelfItem } from "../components/Shelf";
import { FilterChips } from "../components/Tabs";
import { artUrl, backend } from "../lib/backend";
import { headerColorsFromImageUrl, headerColorsFromServer } from "../lib/color";
import type { HomeItem } from "../lib/types";
import { useLoad } from "../lib/useLoad";
import { useSettings } from "../stores/settings";
import { nav } from "../stores/router";
import { HomeItemCard, homeItemInfo } from "./cards";
import { HOME_FILTERS, HOME_SHELVES, chipsAlpha, gridItems, greeting, visibleShelves, type HomeFilter } from "./home";
import { ShortcutSkeleton, ShortcutTile } from "./ShortcutTile";
import "./home.css";

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
  return on ? colour : null;
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

export function HomePage() {
  const { data, error } = useLoad(() => backend.home(), []);
  const [filter, setFilter] = useState<HomeFilter>("all");
  const [hovered, setHovered] = useState<string | null>(null);
  const root = useRef<HTMLDivElement>(null);
  const showGrid = useSettings((s) => s.settings?.homeShortcuts !== false);
  const washOn = useSettings((s) => s.settings?.artworkColours !== false && s.settings?.homeColourWash !== false);

  const tiles = data ? gridItems(data, filter, showGrid) : [];
  const shelves = data ? visibleShelves(data, filter) : [];
  const washItem = tiles.find((t) => homeItemInfo(t).key === hovered) ?? tiles[0];
  const wash = useWashColour(washItem, washOn);

  const pick = (f: HomeFilter) => {
    setFilter(f);
    setHovered(null);
    root.current?.closest(".main__scroll")?.scrollTo({ top: 0 });
  };

  const libraryEmpty = data && data.shortcuts.length === 0 && visibleShelves(data, "all").length === 0;
  const filterEmpty = data && !libraryEmpty && tiles.length === 0 && shelves.length === 0;
  const kindLabel = HOME_FILTERS.find((f) => f.id === filter)?.label.toLowerCase();

  return (
    <div
      className={`page-pad home ${wash ? "has-wash" : ""}`}
      ref={root}
      style={wash ? ({ "--home-wash": wash } as CSSProperties) : undefined}
      data-testid="home"
    >
      <div className="home__wash" aria-hidden="true" />
      <h1 className="sr-only">{greeting()}</h1>
      <HomeChips value={filter} onChange={pick} />
      {error && !data ? (
        <p className="page-error">Couldn't load Home: {error.message}</p>
      ) : libraryEmpty ? (
        <div className="empty-state" role="status">
          <Disc3 size={40} strokeWidth={1.5} aria-hidden="true" />
          <h2 className="empty-state__title">Nothing here yet</h2>
          <p className="empty-state__text">Once your library has synced, albums you add and play show up here.</p>
          <Button variant="secondary" size="sm" onClick={() => nav.go({ name: "library", tab: "albums" })}>
            Browse your library
          </Button>
        </div>
      ) : (
        <>
          {!data && showGrid ? (
            <div className="home-grid" aria-hidden="true">
              {Array.from({ length: 8 }, (_, i) => (
                <ShortcutSkeleton key={i} />
              ))}
            </div>
          ) : (
            tiles.length > 0 && (
              <div className="home-grid" role="list" aria-label="Shortcuts" onPointerLeave={() => setHovered(null)}>
                {tiles.map((t) => {
                  const key = homeItemInfo(t).key;
                  return <ShortcutTile key={key} item={t} onHover={() => setHovered(key)} />;
                })}
              </div>
            )
          )}
          {filterEmpty && (
            <p className="home__empty" role="status">
              No {kindLabel} to show yet.
            </p>
          )}
          {!data
            ? HOME_SHELVES.slice(0, 3).map((s) => <Shelf key={s.id} title={s.title} loading />)
            : shelves.map((s) => (
                <Shelf
                  key={s.id}
                  title={s.title}
                  skeletonKind={s.id === "topArtists" ? "artist" : "album"}
                  onShowAll={() => nav.go({ name: "homeShelf", shelf: s.id })}
                >
                  {s.items.map((item) => (
                    <ShelfItem key={homeItemInfo(item).key}>
                      <HomeItemCard item={item} />
                    </ShelfItem>
                  ))}
                </Shelf>
              ))}
        </>
      )}
    </div>
  );
}
