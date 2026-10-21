/**
 * A playlist (or Liked Songs) on the phone: a compact header so a long list
 * starts sooner, genre pills from the songs' own genres that narrow the
 * list in place, and the songs, each with the shared song sheet.
 */
import { MoreHorizontal } from "lucide-react";
import { Art } from "../../components/Art";
import { LikedCover } from "../../components/LikedCover";
import { phone } from "../phoneUi";
import { useMemo, useState, type ReactNode } from "react";
import { PhoneDownload } from "../PhoneDownload";
import { backend } from "../../lib/backend";
import { formatLongDuration } from "../../lib/format";
import type { Playlist, Track } from "../../lib/types";
import { useLoad } from "../../lib/useLoad";
import { player, usePlayer } from "../../stores/player";
import { BackBar, PageFailed, PageLoading, SongRow, countLine } from "./common";
import { tap } from "../bits";
import { PhoneShuffleToggle, PlayPill } from "./kit";

/** Genre pills shown. */
export const GENRE_PILLS = 6;

/** The genres most songs carry, most first. */
export function topGenres(tracks: Track[], n = GENRE_PILLS): string[] {
  const count = new Map<string, number>();
  for (const t of tracks) for (const g of t.genres ?? []) count.set(g, (count.get(g) ?? 0) + 1);
  return [...count.entries()]
    .sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]))
    .slice(0, n)
    .map(([g]) => g);
}

export function PhonePlaylist({ id }: { id: string }) {
  const { data, error, retry } = useLoad(() => backend.playlistPage(id), [id]);
  if (error && !data) return <PageFailed what="this playlist" retry={retry} framed />;
  if (!data) return <PageLoading />;
  const order = data.itemIds;
  return (
    <SongList
      title={data.playlist.title}
      playlist={data.playlist}
      tracks={data.tracks}
      contextKind="playlist"
      contextId={data.playlist.id}
      download={<PhoneDownload kind="playlist" id={data.playlist.id} title={data.playlist.title} trackIds={() => data.tracks.map((t) => t.id)} />}
      play={(start, shuffle, ids) => void player.playPlaylist(data.playlist.id, start, shuffle, ids)}
      inPlaylist={(i) => ({ playlist: data.playlist, itemId: data.itemIds[i], order })}
    />
  );
}

export function PhoneLiked() {
  const { data, error, retry } = useLoad(() => backend.likedPage(), []);
  if (error && !data) return <PageFailed what="Liked Songs" retry={retry} framed />;
  if (!data) return <PageLoading />;
  return (
    <SongList
      title="Liked Songs"
      tracks={data.tracks}
      contextKind="liked"
      contextId="liked"
      download={<PhoneDownload kind="liked" id="" title="Liked Songs" />}
      play={(start, shuffle, ids) => void player.playLiked(start, ids, shuffle)}
    />
  );
}

function SongList({
  title,
  playlist,
  tracks,
  contextKind,
  contextId,
  download,
  play,
  inPlaylist,
}: {
  title: string;
  playlist?: Playlist;
  tracks: Track[];
  contextKind: "playlist" | "liked";
  contextId: string;
  download: ReactNode;
  play: (startTrackId: string | undefined, shuffle: boolean | undefined, trackIds?: string[]) => void;
  inPlaylist?: (index: number) => import("../phoneUi").InPlaylist;
}) {
  const ctx = usePlayer((s) => s.state?.queue.context ?? null);
  const status = usePlayer((s) => s.state?.status ?? "idle");
  const [genre, setGenre] = useState<string | null>(null);
  const genres = useMemo(() => topGenres(tracks), [tracks]);
  const rows = useMemo(() => tracks.map((t, i) => ({ t, i })).filter(({ t }) => !genre || (t.genres ?? []).includes(genre)), [tracks, genre]);
  const here = ctx?.kind === contextKind && ctx.id === contextId;
  const playing = here && (status === "playing" || status === "loading" || status === "buffering");
  const total = tracks.reduce((n, t) => n + t.durationMs, 0);
  const ids = genre ? rows.map((r) => r.t.id) : undefined;
  return (
    <div className="ph-page ph-playlist">
      <BackBar title={title} after={200} />
      <header className="ph-playlist__head">
        <div className="ph-playlist__cover">
          {playlist ? <Art path={playlist.thumb} fluid requestSize={400} kind="playlist" /> : <LikedCover size={176} />}
        </div>
        <h1 className="ph-playlist__title">{title}</h1>
        <p className="ph-playlist__count tabular">
          {playlist?.smart ? "Smart playlist, " : ""}
          {countLine(tracks.length, "song")}, {formatLongDuration(total)}
        </p>
      </header>
      <div className="ph-actions">
        <div className="ph-actions__left">
          <PlayPill playing={playing} label={playing ? "Pause" : `Play ${title}`} onClick={() => (playing || (here && !genre) ? void player.toggle() : play(undefined, undefined, ids))} />
          <PhoneShuffleToggle id={contextId} kind={contextKind} name={title} />
        </div>
        <div className="ph-actions__right">
          {download}
          {playlist && (
            <button type="button" className="ph-icon-btn" aria-label={`More for ${title}`} onClick={() => phone.push({ kind: "playlist", playlist })}>
              <MoreHorizontal size={24} aria-hidden="true" />
            </button>
          )}
        </div>
      </div>
      {genres.length > 1 && (
        <div className="ph-pills ph-pills--scroll" role="group" aria-label="Genres">
          {genres.map((g) => (
            <button key={g} type="button" className={`ph-chip ${genre === g ? "is-on" : ""}`} aria-pressed={genre === g} onClick={() => (tap("selection"), setGenre(genre === g ? null : g))}>
              {g}
            </button>
          ))}
        </div>
      )}
      <ul className="ph-songs" aria-label="Songs">
        {rows.map(({ t, i }) => (
          <SongRow key={`${t.id}:${i}`} track={t} onPlay={() => play(t.id, undefined, ids)} inPlaylist={inPlaylist?.(i)} />
        ))}
      </ul>
    </div>
  );
}
