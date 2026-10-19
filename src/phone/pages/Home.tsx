/**
 * Home on the phone: a type row, the 2 by 4 grid of recent and pinned
 * shortcuts, then the shelves as sideways carousels. Records first.
 */
import { Pause, Play, UserRound } from "lucide-react";
import { useEffect, useRef, useState, type CSSProperties } from "react";
import { Art } from "../../components/Art";
import { backend } from "../../lib/backend";
import { useLoad } from "../../lib/useLoad";
import { GRID_TILES, HOME_FILTERS, filterItems, visibleShelves, type HomeFilter } from "../../pages/home";
import { trackArtistLine } from "../../lib/format";
import { useArtColours } from "../../lib/useTint";
import { useApp } from "../../stores/app";
import { player, usePlayer } from "../../stores/player";
import { ProgressLine, tap } from "../bits";
import { nav } from "../../stores/router";
import { phone, usePhoneUi } from "../phoneUi";
import { Carousel, PageFailed, PageLoading, PlayingBars, ShelfCard, itemInfo } from "./common";

export function PhoneHome() {
  const { data, error, retry } = useLoad(() => backend.home(), []);
  const [filter, setFilter] = useState<HomeFilter>("all");
  const avatar = useApp((s) => s.auth?.avatar ?? null);
  const playingCtx = usePlayer((s) => s.state?.queue.context ?? null);
  if (error && !data) return <PageFailed what="Home" retry={retry} />;
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
      <NowCard />
      {grid.length > 0 && (
        <ul className="ph-shortcuts" aria-label="Recent and pinned">
          {grid.map((i) => {
            const info = itemInfo(i);
            const id = info.route.name === "album" || info.route.name === "artist" || info.route.name === "playlist" ? info.route.id : "";
            const live = playingCtx?.id === id && playingCtx?.kind === info.kind;
            return (
              <li key={info.key}>
                <button type="button" className={`ph-shortcut ${live ? "is-playing" : ""}`} onClick={() => nav.go(info.route)}>
                  <Art path={info.thumb} size={56} round={info.round} kind={info.kind} sleeve={info.kind === "album" ? { title: info.title, artist: info.sub } : undefined} />
                  <span className="ph-shortcut__title">{info.title}</span>
                  {live && <PlayingBars />}
                </button>
              </li>
            );
          })}
        </ul>
      )}
      {shelves.map((s) => (
        <Carousel key={s.id} title={s.title}>
          {s.items.slice(0, 12).map((i) => {
            const info = itemInfo(i);
            return <ShelfCard key={info.key} title={info.title} sub={info.sub} thumb={info.thumb} round={info.round} kind={info.kind} onOpen={() => nav.go(info.route)} />;
          })}
        </Carousel>
      ))}
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

/**
 * The record on the player, at the front of Home: art, the song, play or
 * pause and how far in. Tapping it opens now playing. While it is on screen
 * the mini player steps aside, so the song isn't shown twice.
 */
export function NowCard() {
  const track = usePlayer((s) => s.state?.track ?? null);
  const status = usePlayer((s) => s.state?.status ?? "idle");
  const colours = useArtColours(null, track?.thumb);
  const box = useRef<HTMLDivElement>(null);
  const has = !!track;
  useEffect(() => {
    const el = box.current;
    if (!el || typeof IntersectionObserver === "undefined") return;
    const io = new IntersectionObserver(([e]) => usePhoneUi.setState({ homeCard: e.isIntersecting }), { threshold: 0.25 });
    io.observe(el);
    return () => {
      io.disconnect();
      usePhoneUi.setState({ homeCard: false });
    };
  }, [has]);
  if (!track) return null;
  const playing = status === "playing" || status === "loading" || status === "buffering";
  const style = { "--ph-tint": colours?.vivid ?? "var(--color-bg-overlay)" } as CSSProperties;
  return (
    <div ref={box} className="ph-now" style={style} data-testid="home-now">
      <button type="button" className="ph-now__open" aria-label={`${track.title} by ${trackArtistLine(track)}. Open the player`} onClick={phone.openPlayer}>
        <span className="ph-now__art">
          <Art path={track.thumb} fluid requestSize={240} sleeve={{ title: track.albumTitle, artist: track.artistTitle }} />
        </span>
        <span className="ph-now__text">
          <span className="ph-now__eyebrow">{playing ? "Now playing" : "Paused"}</span>
          <span className="ph-now__title">{track.title}</span>
          <span className="ph-now__artist">{trackArtistLine(track)}</span>
        </span>
      </button>
      <button
        type="button"
        className="ph-play ph-now__play"
        aria-label={playing ? "Pause" : "Play"}
        onClick={() => {
          tap();
          void player.toggle();
        }}
      >
        {playing ? <Pause size={26} fill="currentColor" aria-hidden="true" /> : <Play size={26} fill="currentColor" aria-hidden="true" />}
      </button>
      <ProgressLine className="ph-now__progress" />
    </div>
  );
}
