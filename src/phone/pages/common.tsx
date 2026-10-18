/**
 * Pieces the phone pages share: the back bar, song rows, tiles and the
 * sideways shelves.
 */
import { ChevronLeft, MoreHorizontal } from "lucide-react";
import type { ReactNode } from "react";
import { Art } from "../../components/Art";
import { DownloadedMark } from "../../components/DownloadButton";
import { trackArtistLine } from "../../lib/format";
import type { HomeItem, Track } from "../../lib/types";
import { useCanGoBack, useRouter, type Route } from "../../stores/router";
import { usePlayer } from "../../stores/player";
import { phone, type InPlaylist } from "../phoneUi";

/** The bar over a page that was opened from another: Back and a title that appears once scrolled. */
export function BackBar({ title, shown = false, right }: { title?: string; shown?: boolean; right?: ReactNode }) {
  const canBack = useCanGoBack();
  return (
    <div className={`ph-backbar ${shown ? "is-titled" : ""}`}>
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

/** A song: optional number or art, title, artist line with the downloaded mark, and its sheet. */
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
  return (
    <li className="ph-song">
      <button type="button" className="ph-song__main" onClick={onPlay} aria-current={playing ? "true" : undefined}>
        {index !== undefined && <span className={`ph-song__num tabular ${playing ? "is-playing" : ""}`}>{index}</span>}
        {art && <Art path={track.thumb} size={48} sleeve={{ title: track.albumTitle, artist: track.artistTitle }} />}
        <span className="ph-row__text">
          <span className={`ph-row__title ${playing ? "is-playing" : ""}`}>{track.title}</span>
          <span className="ph-row__sub">
            <DownloadedMark trackId={track.id} />
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

/** What a Home item shows and where it goes. */
export function itemInfo(i: HomeItem): { key: string; title: string; sub: string; thumb: string | null; round: boolean; route: Route; kind: "album" | "artist" | "playlist" } {
  if (i.kind === "album")
    return { key: `album:${i.album.id}`, title: i.album.title, sub: i.album.artistTitle, thumb: i.album.thumb, round: false, route: { name: "album", id: i.album.id }, kind: "album" };
  if (i.kind === "artist")
    return { key: `artist:${i.artist.id}`, title: i.artist.title, sub: "Artist", thumb: i.artist.thumb, round: true, route: { name: "artist", id: i.artist.id }, kind: "artist" };
  return {
    key: `playlist:${i.playlist.id}`,
    title: i.playlist.title,
    sub: `${i.playlist.trackCount} songs`,
    thumb: i.playlist.thumb,
    round: false,
    route: { name: "playlist", id: i.playlist.id },
    kind: "playlist",
  };
}

/** A titled shelf that scrolls sideways. */
export function Carousel({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section className="ph-shelf" aria-label={title}>
      <h2 className="ph-shelf__title">{title}</h2>
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
  onOpen,
}: {
  title: string;
  sub?: string;
  /** Printed on the sleeve when a record has no cover (defaults to `sub`). */
  artist?: string;
  thumb: string | null;
  round?: boolean;
  kind?: "album" | "artist" | "playlist";
  onOpen: () => void;
}) {
  return (
    <button type="button" className={`ph-card-tile ${round ? "is-round" : ""}`} onClick={onOpen}>
      <Art path={thumb} size={136} round={round} kind={kind} sleeve={kind === "album" ? { title, artist: artist ?? sub } : undefined} monogram={kind === "artist" ? title : undefined} />
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

export function PageFailed({ what, retry }: { what: string; retry: () => void }) {
  return (
    <div className="ph-empty">
      <p className="ph-empty__title">Couldn't load {what}</p>
      <button type="button" className="ph-pill" onClick={retry}>
        Try again
      </button>
    </div>
  );
}
