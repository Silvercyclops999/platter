/**
 * Home on the phone: the record you were in the middle of, on the platter,
 * then what you spun this week as three columns of covers (Liked Songs
 * first), your mixes as printed sleeves, at most two more shelves and
 * Explore. Everything else has its own tab (playlists in Library, what you
 * played in Stats).
 */
import { Pause, Play, UserRound } from "lucide-react";
import type { CSSProperties } from "react";
import { Art } from "../../components/Art";
import { backend } from "../../lib/backend";
import { headerColorsFromServer } from "../../lib/color";
import { useLoad } from "../../lib/useLoad";
import { gridItems, showsLikedTile, visibleShelves } from "../../pages/home";
import { useContextState } from "../../pages/cards";
import { useRecordHome, useRecordsLead } from "../../pages/RecordHome";
import { LikedCover } from "../../components/LikedCover";
import { useLikes } from "../../stores/likes";
import { useSettings } from "../../stores/settings";
import type { HomeItem, HomeShelfId, RecordProgress } from "../../lib/types";
import { useApp } from "../../stores/app";
import { player, usePlayer } from "../../stores/player";
import { nav } from "../../stores/router";
import { phone } from "../phoneUi";
import { Carousel, PageFailed, PageLoading, ShelfCard, countLine, itemInfo } from "./common";
import { HomeExplore, HomeMixes } from "./HomeExtras";

/** Places in Spun this week: two rows of three. */
export const SPUN_TILES = 6;

/** The record to resume: its sleeve with the record peeking out, and Resume. */
function OnThePlatter({ p }: { p: RecordProgress }) {
  const { isContext, playing } = useContextState("album", p.album.id);
  const resume = () => void (isContext ? player.toggle() : player.playAlbum(p.album.id, p.nextTrackId));
  const open = () => nav.go({ name: "album", id: p.album.id });
  const label = headerColorsFromServer(p.album.tint)?.vivid ?? "#8a7a62";
  return (
    <section className="ph-platter" aria-labelledby="ph-platter-title" data-testid="on-the-platter">
      <button type="button" className="ph-platter__sleeve" onClick={open} aria-label={`Open ${p.album.title}`} style={{ "--record-label": label } as CSSProperties}>
        <span className="ph-platter__record record-disc" aria-hidden="true" />
        <span className="ph-platter__cover">
          <Art path={p.album.thumb} fluid requestSize={224} kind="album" sleeve={{ title: p.album.title, artist: p.album.artistTitle }} />
        </span>
      </button>
      <div className="ph-platter__text">
        <span className="ph-platter__eyebrow">{isContext ? "Now spinning" : "On the platter"}</span>
        <h2 className="ph-platter__title" id="ph-platter-title">
          <button type="button" onClick={open}>
            <span className="ph-platter__name">{p.album.title}</span>
          </button>
        </h2>
        <span className="ph-platter__artist">
          {p.album.artistTitle}
          {p.album.year ? `, ${p.album.year}` : ""}
        </span>
        <span className="ph-platter__actions">
          <button type="button" className="ph-platter__resume" onClick={resume}>
            {playing ? <Pause size={14} fill="currentColor" aria-hidden="true" /> : <Play size={14} fill="currentColor" aria-hidden="true" />}
            {playing ? "Pause" : isContext ? "Play" : "Resume"}
          </button>
          <span className="ph-platter__where">
            song {p.nextNumber} of {p.total}
          </span>
        </span>
      </div>
    </section>
  );
}

/** The shelves All shows, besides Made for you and Explore. */
export const PHONE_HOME_SHELVES: readonly HomeShelfId[] = ["recentlyAdded", "topArtists"];

export function PhoneHome() {
  const { data, error, retry } = useLoad(() => backend.home(), []);
  const records = useRecordHome(useRecordsLead());
  const avatar = useApp((s) => s.auth?.avatar ?? null);
  const playingCtx = usePlayer((s) => s.state?.queue.context ?? null);
  const likedCount = useLikes((s) => s.ids.size);
  const showGrid = useSettings((s) => s.settings?.homeShortcuts !== false);
  if (error && !data) return <PageFailed what="Home" retry={retry} framed />;
  if (!data) return <PageLoading />;
  // Liked Songs leads, as on the computer, and takes one of the places.
  const liked = showsLikedTile("all", likedCount, showGrid);
  const room = SPUN_TILES - (liked ? 1 : 0);
  // The records played most this week; before any, what's recent and pinned.
  const spun = showGrid ? (records?.spunThisWeek ?? []).map((r): HomeItem => ({ kind: "album", album: r.album })) : [];
  const tiles = (spun.length ? spun : gridItems(data, "all", showGrid, liked)).slice(0, room);
  const tilesTitle = spun.length ? "Spun this week" : "Recent and pinned";
  const shelves = visibleShelves(data, "all").filter((s) => PHONE_HOME_SHELVES.includes(s.id));
  return (
    <div className="ph-page ph-home">
      <div className="ph-home__top">
        <h1 className="ph-home__title">Home</h1>
        <button type="button" className="ph-avatar" aria-label="You" onClick={() => phone.tab("you")}>
          {avatar ? <Art path={avatar} size={32} round kind="artist" /> : <UserRound size={20} aria-hidden="true" />}
        </button>
      </div>
      {records?.resume && <OnThePlatter p={records.resume} />}
      {(tiles.length > 0 || liked) && (
        <section className="ph-spun" aria-labelledby="ph-spun-title">
          <h2 className="ph-spun__title" id="ph-spun-title">
            {tilesTitle}
          </h2>
          <ul className="ph-spun__grid" aria-label={tilesTitle}>
            {liked && (
              <li key="liked">
                <button type="button" className={`ph-spun__tile ${playingCtx?.kind === "liked" ? "is-playing" : ""}`} onClick={() => nav.go({ name: "liked" })}>
                  <LikedCover fluid />
                  <span className="ph-spun__name">Liked Songs</span>
                  <span className="ph-spun__sub">{countLine(likedCount, "song")}</span>
                </button>
              </li>
            )}
            {tiles.map((i) => {
              const info = itemInfo(i);
              const id = info.route.name === "album" || info.route.name === "artist" || info.route.name === "playlist" ? info.route.id : "";
              // What's playing is marked on its song's row; the tile only takes the accent.
              const live = playingCtx?.id === id && playingCtx?.kind === info.kind;
              return (
                <li key={info.key}>
                  <button type="button" className={`ph-spun__tile ${live ? "is-playing" : ""} ${info.round ? "is-round" : ""}`} onClick={() => nav.go(info.route)}>
                    <Art path={info.thumb} fluid requestSize={240} round={info.round} kind={info.kind} tint={info.tint} sleeve={info.kind === "album" ? { title: info.title, artist: info.sub } : undefined} />
                    <span className="ph-spun__name">{info.title}</span>
                    {info.sub && <span className="ph-spun__sub">{info.sub}</span>}
                  </button>
                </li>
              );
            })}
          </ul>
        </section>
      )}
      <HomeMixes />
      {shelves.map((s) => (
        <Carousel key={s.id} title={s.title} onAll={() => nav.go(s.id === "recentlyPlayed" ? { name: "recents" } : { name: "homeShelf", shelf: s.id })}>
          {s.items.slice(0, 12).map((i) => {
            const info = itemInfo(i);
            return <ShelfCard key={info.key} title={info.title} sub={info.sub} thumb={info.thumb} round={info.round} kind={info.kind} onOpen={() => nav.go(info.route)} />;
          })}
        </Carousel>
      ))}
      <HomeExplore />
      {!tiles.length && !liked && !shelves.length && !records?.resume && (
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
