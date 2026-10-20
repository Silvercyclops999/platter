/**
 * Pieces the phone pages share: the back bar, song rows, tiles and the
 * sideways shelves.
 */
import { ChevronLeft, MoreHorizontal } from "lucide-react";
import { useEffect, useRef, useState, type ReactNode } from "react";
import { Art } from "../../components/Art";
import { RowGlyphs } from "../bits";

import { trackArtistLine } from "../../lib/format";
import type { HomeItem, Track } from "../../lib/types";
import { useCanGoBack, useRouter, type Route } from "../../stores/router";
import { player, usePlayer } from "../../stores/player";
import { isUnavailable, useApp, type AppStore } from "../../stores/app";
import { useIsDownloaded } from "../../stores/downloads";
import { phone, type InPlaylist } from "../phoneUi";
import { SwipeUnder, useSwipeToQueue } from "../rowSwipe";

/**
 * The bar over a page that was opened from another: Back, and once the page
 * has scrolled past its header (`after` px) a solid bar with the page's
 * title, so nothing scrolls under a floating button.
 */
export function BackBar({ title, after = 160, right }: { title?: string; after?: number; right?: ReactNode }) {
  const canBack = useCanGoBack();
  const bar = useRef<HTMLDivElement>(null);
  const [titled, setTitled] = useState(false);
  useEffect(() => {
    const page = bar.current?.closest(".ph-page");
    if (!page) return;
    let frame = 0;
    const check = () => {
      frame = 0;
      setTitled(page.scrollTop > after);
    };
    const onScroll = () => {
      if (!frame) frame = requestAnimationFrame(check);
    };
    page.addEventListener("scroll", onScroll, { passive: true });
    return () => {
      page.removeEventListener("scroll", onScroll);
      cancelAnimationFrame(frame);
    };
  }, [after]);
  return (
    <div ref={bar} className={`ph-backbar ${titled ? "is-titled" : ""}`}>
      {canBack ? (
        <button type="button" className="ph-icon-btn ph-backbar__back" aria-label="Back" onClick={() => useRouter.getState().back()}>
          <ChevronLeft size={26} aria-hidden="true" />
        </button>
      ) : (
        <span className="ph-icon-btn" aria-hidden="true" />
      )}
      {title && <span className="ph-backbar__title">{title}</span>}
      <span className="ph-backbar__right">{right}</span>
    </div>
  );
}

/**
 * Whether a song row is dimmed as unplayable. Only while just music on the
 * phone can play (health `cachedOnly`: server unreachable, files gone, Go
 * offline), and then only songs that aren't on the phone: downloaded or in
 * the audio cache. The song playing now is never dimmed (it plays), and while
 * the list of songs on the phone isn't known yet nothing is dimmed.
 */
export function songCantPlay(
  trackId: string,
  s: Pick<AppStore, "health" | "cached">,
  { playing, downloaded }: { playing: boolean; downloaded: boolean },
): boolean {
  return !playing && !downloaded && isUnavailable(trackId, s);
}

/**
 * A song: optional number or art, title, artist line with the downloaded
 * mark, and its sheet. The playing song wears the level bars: in place of
 * its number, or before its title on a row without one.
 */
export function SongRow({
  track,
  index,
  art = true,
  sub,
  onPlay,
  inPlaylist,
}: {
  track: Track;
  index?: number;
  art?: boolean;
  sub?: ReactNode;
  onPlay: () => void;
  inPlaylist?: InPlaylist;
}) {
  const playing = usePlayer((s) => s.state?.track?.id === track.id);
  const here = useIsDownloaded(track.id);
  // Dimmed only when it really can't play: see `songCantPlay`.
  const unplayable = useApp((s) => songCantPlay(track.id, s, { playing, downloaded: here }));
  // Swipe right: into the queue.
  const swipe = useSwipeToQueue(() => void player.addToQueue([track]), !unplayable);
  return (
    <li className={`ph-song ${swipe.className} ${unplayable ? "is-unplayable" : ""}`} aria-disabled={unplayable || undefined} {...swipe.props}>
      <SwipeUnder />
      <button type="button" className="ph-song__main" onClick={onPlay} aria-current={playing ? "true" : undefined}>
        {index !== undefined && (
          <span className={`ph-song__num tabular ${playing ? "is-playing" : ""}`}>{playing ? <PlayingBars /> : index}</span>
        )}
        {art && <Art path={track.thumb} size={48} sleeve={{ title: track.albumTitle, artist: track.artistTitle }} />}
        <span className="ph-row__text">
          <span className={`ph-row__title ${playing ? "is-playing" : ""}`}>
            {playing && index === undefined && <PlayingBars />}
            {track.title}
          </span>
          <span className="ph-row__sub">
            <RowGlyphs trackId={track.id} />
            {sub ?? trackArtistLine(track)}
          </span>
        </span>
      </button>
      <button type="button" className="ph-icon-btn" aria-label={`More for ${track.title}`} onClick={() => phone.push({ kind: "track", track, inPlaylist })}>
        <MoreHorizontal size={20} aria-hidden="true" />
      </button>
    </li>
  );
}

/** Small moving bars for the playing song (still while paused). */
export function PlayingBars() {
  const paused = usePlayer((s) => s.state?.status !== "playing");
  return (
    <span className={`ph-eq ${paused ? "is-paused" : ""}`} role="img" aria-label="Playing">
      <span />
      <span />
      <span />
    </span>
  );
}

/** What a Home item shows and where it goes. */
export function itemInfo(i: HomeItem): { key: string; title: string; sub: string; thumb: string | null; tint?: string | null; round: boolean; route: Route; kind: "album" | "artist" | "playlist" } {
  if (i.kind === "album")
    return { key: `album:${i.album.id}`, title: i.album.title, sub: i.album.artistTitle, thumb: i.album.thumb, tint: i.album.tint, round: false, route: { name: "album", id: i.album.id }, kind: "album" };
  if (i.kind === "artist")
    return { key: `artist:${i.artist.id}`, title: i.artist.title, sub: "", thumb: i.artist.thumb, round: true, route: { name: "artist", id: i.artist.id }, kind: "artist" };
  return {
    key: `playlist:${i.playlist.id}`,
    title: i.playlist.title,
    sub: countLine(i.playlist.trackCount, "song"),
    thumb: i.playlist.thumb,
    round: false,
    route: { name: "playlist", id: i.playlist.id },
    kind: "playlist",
  };
}

/** A titled shelf that scrolls sideways. */
export function Carousel({ title, children, onAll }: { title: string; children: ReactNode; onAll?: () => void }) {
  return (
    <section className="ph-shelf" aria-label={title}>
      {onAll ? (
        <div className="ph-shelf__head">
          <h2 className="ph-shelf__title">{title}</h2>
          <button type="button" className="ph-shelf__all" onClick={onAll} aria-label={`Show all: ${title}`}>
            Show all
          </button>
        </div>
      ) : (
        <h2 className="ph-shelf__title">{title}</h2>
      )}
      <div className="ph-shelf__row">{children}</div>
    </section>
  );
}

/** A record (or artist, or playlist) in a shelf. */
export function ShelfCard({
  title,
  sub,
  artist,
  thumb,
  round,
  kind,
  tint,
  onOpen,
}: {
  title: string;
  sub?: string;
  /** The cover's colour, shown until it loads. */
  tint?: string | null;
  /** Printed on the sleeve when a record has no cover (defaults to `sub`). */
  artist?: string;
  thumb: string | null;
  round?: boolean;
  kind?: "album" | "artist" | "playlist";
  onOpen: () => void;
}) {
  return (
    <button type="button" className={`ph-card-tile ${round ? "is-round" : ""}`} onClick={onOpen}>
      <Art path={thumb} size={round ? 112 : 136} round={round} kind={kind} tint={tint} sleeve={kind === "album" ? { title, artist: artist ?? sub } : undefined} monogram={kind === "artist" ? title : undefined} />
      <span className="ph-card-tile__title">{title}</span>
      {sub && <span className="ph-card-tile__sub">{sub}</span>}
    </button>
  );
}

/** "12 songs, 48 min" style counts. */
export function countLine(n: number, noun: string): string {
  return `${n.toLocaleString()} ${n === 1 ? noun : `${noun}s`}`;
}

/** A page's loading state: quiet blocks where the content goes. */
export function PageLoading() {
  return (
    <div className="ph-loading" aria-busy="true" aria-label="Loading">
      <span className="ph-loading__block ph-loading__block--hero" />
      <span className="ph-loading__block" />
      <span className="ph-loading__block" />
    </div>
  );
}

/** A page that couldn't load. `framed`: it is the whole page, so it brings its own margins and Back. */
export function PageFailed({ what, retry, framed }: { what: string; retry: () => void; framed?: boolean }) {
  if (framed)
    return (
      <div className="ph-page" data-phone-failed={what}>
        <BackBar />
        <PageFailed what={what} retry={retry} />
      </div>
    );
  return (
    <div className="ph-empty">
      <p className="ph-empty__title">Couldn't load {what}</p>
      <button type="button" className="ph-pill" onClick={retry}>
        Try again
      </button>
    </div>
  );
}
