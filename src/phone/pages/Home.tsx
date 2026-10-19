/**
 * Home on the phone: a type row, the 2 by 4 grid of recent and pinned
 * shortcuts, then the shelves as sideways carousels. Records first.
 */
import { UserRound } from "lucide-react";
import { useState } from "react";
import { Art } from "../../components/Art";
import { backend } from "../../lib/backend";
import { useLoad } from "../../lib/useLoad";
import { GRID_TILES, HOME_FILTERS, filterItems, visibleShelves, type HomeFilter } from "../../pages/home";
import { useApp } from "../../stores/app";
import { usePlayer } from "../../stores/player";
import { tap } from "../bits";
import { nav } from "../../stores/router";
import { phone } from "../phoneUi";
import { Carousel, PageFailed, PageLoading, PlayingBars, ShelfCard, itemInfo } from "./common";
import { HomeExplore, HomeMixes } from "./HomeExtras";

export function PhoneHome() {
  const { data, error, retry } = useLoad(() => backend.home(), []);
  const [filter, setFilter] = useState<HomeFilter>("all");
  const avatar = useApp((s) => s.auth?.avatar ?? null);
  const playingCtx = usePlayer((s) => s.state?.queue.context ?? null);
  if (error && !data) return <PageFailed what="Home" retry={retry} framed />;
  if (!data) return <PageLoading />;
  const grid = filterItems(data.shortcuts, filter).slice(0, GRID_TILES);
  const shelves = visibleShelves(data, filter);
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
      {grid.length > 0 && (
        <ul className="ph-shortcuts" aria-label="Recent and pinned">
          {grid.map((i) => {
            const info = itemInfo(i);
            const id = info.route.name === "album" || info.route.name === "artist" || info.route.name === "playlist" ? info.route.id : "";
            const live = playingCtx?.id === id && playingCtx?.kind === info.kind;
            return (
              <li key={info.key}>
                <button type="button" className={`ph-shortcut ${live ? "is-playing" : ""}`} onClick={() => nav.go(info.route)}>
                  <Art path={info.thumb} size={56} round={info.round} kind={info.kind} tint={info.tint} sleeve={info.kind === "album" ? { title: info.title, artist: info.sub } : undefined} />
                  <span className="ph-shortcut__title">{info.title}</span>
                  {live && <PlayingBars />}
                </button>
              </li>
            );
          })}
        </ul>
      )}
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
      {!grid.length && !shelves.length && (
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

