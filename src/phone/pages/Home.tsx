/**
 * Home on the phone: a type row, the 2 by 4 grid of recent and pinned
 * shortcuts, then at most four more sections: Made for you, what's new in
 * the library, your top artists and Explore. Everything else has its own
 * tab (playlists in Library, what you played in Stats). A type picked shows
 * every shelf of that type. While only music on the phone can play, an "On
 * this phone" shelf of what's downloaded follows the grid.
 */
import { UserRound } from "lucide-react";
import { useMemo, useState } from "react";
import { Art } from "../../components/Art";
import { backend } from "../../lib/backend";
import { useLoad } from "../../lib/useLoad";
import { HOME_FILTERS, gridItems, showsLikedTile, visibleShelves, type HomeFilter } from "../../pages/home";
import { LikedCover } from "../../components/LikedCover";
import { useLikes } from "../../stores/likes";
import { useSettings } from "../../stores/settings";
import type { HomeShelfId } from "../../lib/types";
import { useApp } from "../../stores/app";
import { downloadRoute, useDownloads } from "../../stores/downloads";
import { toEntries, useLibraryView } from "../../stores/library";
import { usePlayer } from "../../stores/player";
import { RowGlyphs, tap } from "../bits";
import { MixRowSleeve } from "./Library";
import { useLibraryLoader } from "../../shell/useLibrary";
import { nav } from "../../stores/router";
import { phone } from "../phoneUi";
import { Carousel, PageFailed, PageLoading, ShelfCard, itemInfo } from "./common";
import { HomeExplore, HomeMixes } from "./HomeExtras";

/** The shelves All shows, besides Made for you and Explore. */
export const PHONE_HOME_SHELVES: readonly HomeShelfId[] = ["recentlyAdded", "topArtists"];

export function PhoneHome() {
  const { data, error, retry } = useLoad(() => backend.home(), []);
  const [filter, setFilter] = useState<HomeFilter>("all");
  const avatar = useApp((s) => s.auth?.avatar ?? null);
  const playingCtx = usePlayer((s) => s.state?.queue.context ?? null);
  const likedCount = useLikes((s) => s.ids.size);
  const showGrid = useSettings((s) => s.settings?.homeShortcuts !== false);
  if (error && !data) return <PageFailed what="Home" retry={retry} framed />;
  if (!data) return <PageLoading />;
  // Liked Songs leads the grid, as on the computer, and takes one of its places.
  const liked = showsLikedTile(filter, likedCount, showGrid);
  const grid = gridItems(data, filter, showGrid, liked);
  const shelves = visibleShelves(data, filter).filter((s) => filter !== "all" || PHONE_HOME_SHELVES.includes(s.id));
  return (
    <div className="ph-page ph-home">
      <div className="ph-home__top">
        <button type="button" className="ph-avatar" aria-label="You" onClick={() => phone.tab("you")}>
          {avatar ? <Art path={avatar} size={32} round kind="artist" /> : <UserRound size={20} aria-hidden="true" />}
        </button>
        <div className="ph-pills" role="group" aria-label="Show">
          {HOME_FILTERS.map((f) => (
            <button key={f.id} type="button" className={`ph-chip ${filter === f.id ? "is-on" : ""}`} aria-pressed={filter === f.id} onClick={() => (tap("selection"), setFilter(f.id))}>
              {f.label}
            </button>
          ))}
        </div>
      </div>
      {(grid.length > 0 || liked) && (
        <ul className="ph-shortcuts" aria-label="Recent and pinned">
          {liked && (
            <li key="liked">
              <button type="button" className={`ph-shortcut ${playingCtx?.kind === "liked" ? "is-playing" : ""}`} onClick={() => nav.go({ name: "liked" })}>
                <LikedCover size={56} />
                <span className="ph-shortcut__title">Liked Songs</span>
              </button>
            </li>
          )}
          {grid.map((i) => {
            const info = itemInfo(i);
            const id = info.route.name === "album" || info.route.name === "artist" || info.route.name === "playlist" ? info.route.id : "";
            // What's playing is marked on its song's row; the tile only takes the accent.
            const live = playingCtx?.id === id && playingCtx?.kind === info.kind;
            return (
              <li key={info.key}>
                <button type="button" className={`ph-shortcut ${live ? "is-playing" : ""}`} onClick={() => nav.go(info.route)}>
                  <Art path={info.thumb} size={56} round={info.round} kind={info.kind} tint={info.tint} sleeve={info.kind === "album" ? { title: info.title, artist: info.sub } : undefined} />
                  <span className="ph-shortcut__title">{info.title}</span>
                </button>
              </li>
            );
          })}
        </ul>
      )}
      {filter === "all" && <OnThisPhone />}
      {filter === "all" && <HomeMixes />}
      {shelves.map((s) => (
        <Carousel key={s.id} title={s.title} onAll={() => nav.go(s.id === "recentlyPlayed" ? { name: "recents" } : { name: "homeShelf", shelf: s.id })}>
          {s.items.slice(0, 12).map((i) => {
            const info = itemInfo(i);
            return <ShelfCard key={info.key} title={info.title} sub={info.sub} thumb={info.thumb} round={info.round} kind={info.kind} onOpen={() => nav.go(info.route)} />;
          })}
        </Carousel>
      ))}
      {filter === "all" && <HomeExplore />}
      {!grid.length && !liked && !shelves.length && (
        <div className="ph-empty">
          <p className="ph-empty__title">Nothing played yet</p>
          <p className="ph-empty__text">Play a record from your library and it shows up here.</p>
          <button type="button" className="ph-pill" onClick={() => phone.tab("library")}>
            Open your library
          </button>
        </div>
      )}
    </div>
  );
}


/** What's downloaded, as a shelf, while only music on the phone can play: albums, playlists and mixes with something saved. */
export function OnThisPhone() {
  useLibraryLoader();
  const cachedOnly = useApp((s) => !!s.health?.cachedOnly);
  const dl = useDownloads((s) => s.status?.items);
  const items = useLibraryView((s) => s.items);
  const owner = useApp((s) => s.auth?.username ?? null);
  const cards = useMemo(() => {
    const byKey = new Map(toEntries(items ?? [], owner).map((e) => [`${e.kind}:${e.id}`, e]));
    return (dl ?? [])
      .filter((i) => i.done > 0 && i.kind !== "liked")
      .map((i) => ({ i, e: i.kind === "mix" ? undefined : byKey.get(`${i.kind}:${i.id}`) }))
      .filter(({ i, e }) => i.kind === "mix" || e);
  }, [dl, items, owner]);
  if (!cachedOnly || !cards.length) return null;
  return (
    <Carousel title="On this phone">
      {cards.slice(0, 12).map(({ i, e }) => (
        <ShelfCard
          key={`${i.kind}:${i.id}`}
          title={e?.title ?? i.title}
          sub={i.kind === "album" ? (e?.creator ?? undefined) : i.kind === "mix" ? "Mix" : "Playlist"}
          thumb={e?.thumb ?? null}
          tint={e?.tint}
          kind={i.kind === "album" ? "album" : "playlist"}
          marks={<RowGlyphs item={{ kind: i.kind, id: i.id }} />}
          art={i.kind === "mix" ? <MixRowSleeve e={{ id: i.id, title: i.title, thumb: null }} card /> : undefined}
          onOpen={() => nav.go(downloadRoute(i))}
        />
      ))}
    </Carousel>
  );
}
