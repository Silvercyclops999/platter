/**
 * A playlist (or Liked Songs) on the phone: a compact header so a long list
 * starts sooner, genre pills from the songs' own genres that narrow the
 * list in place, and the songs, each with the shared song sheet. Smart
 * playlists made here, playlists shared with you and Blends open the same
 * way, each from its own page data.
 */
import { MoreHorizontal, Shuffle } from "lucide-react";
import { Art } from "../../components/Art";
import { LikedCover } from "../../components/LikedCover";
import { phone } from "../phoneUi";
import { useEffect, useMemo, useState, type ReactNode } from "react";
import { PhoneDownload } from "../PhoneDownload";
import { backend } from "../../lib/backend";
import { formatLongDuration } from "../../lib/format";
import type { Playlist, Track } from "../../lib/types";
import { useLoad } from "../../lib/useLoad";
import { BlendCover } from "../../pages/BlendPage";
import { useContextState } from "../../pages/cards";
import { playBlend, playShared, useCircle } from "../../stores/circle";
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
  if (id.startsWith("smart:")) return <PhoneSmartPlaylist key={id} id={id} />;
  if (id.startsWith("shared:")) return <PhoneSharedPlaylist key={id} id={id} />;
  if (id.startsWith("blend:")) return <PhoneBlend key={id} id={id} />;
  return <PhoneOwnPlaylist id={id} />;
}

function PhoneOwnPlaylist({ id }: { id: string }) {
  const { data, error, retry } = useLoad(() => backend.playlistPage(id), [id]);
  const { isContext, playing } = useContextState("playlist", id);
  if (error && !data) return <PageFailed what="this playlist" retry={retry} framed />;
  if (!data) return <PageLoading />;
  const order = data.itemIds;
  return (
    <SongList
      title={data.playlist.title}
      cover={<Art path={data.playlist.thumb} fluid requestSize={400} kind="playlist" />}
      kindLine={data.playlist.smart ? "Smart playlist" : undefined}
      sheet={data.playlist}
      tracks={data.tracks}
      here={isContext}
      playing={playing}
      shuffle={<PhoneShuffleToggle id={data.playlist.id} kind="playlist" name={data.playlist.title} />}
      download={<PhoneDownload kind="playlist" id={data.playlist.id} title={data.playlist.title} trackIds={() => data.tracks.map((t) => t.id)} />}
      play={(start, shuffle, ids) => void player.playPlaylist(data.playlist.id, start, shuffle, ids)}
      inPlaylist={(i) => ({ playlist: data.playlist, itemId: data.itemIds[i], order })}
    />
  );
}

export function PhoneLiked() {
  const { data, error, retry } = useLoad(() => backend.likedPage(), []);
  const { isContext, playing } = useContextState("liked", "liked");
  if (error && !data) return <PageFailed what="Liked Songs" retry={retry} framed />;
  if (!data) return <PageLoading />;
  return (
    <SongList
      title="Liked Songs"
      cover={<LikedCover size={176} />}
      tracks={data.tracks}
      here={isContext}
      playing={playing}
      shuffle={<PhoneShuffleToggle id="liked" kind="liked" name="Liked Songs" />}
      download={<PhoneDownload kind="liked" id="" title="Liked Songs" />}
      play={(start, shuffle, ids) => void player.playLiked(start, ids, shuffle)}
    />
  );
}

/**
 * Whether a list played as plain songs (smart, shared, Blend) is the one
 * playing: the queue holds songs and the current one is in this list.
 */
export function useSongsHere(ids: string[]): { here: boolean; playing: boolean } {
  const here = usePlayer((s) => {
    const now = s.state?.track?.id;
    return s.state?.queue.context?.kind === "tracks" && !!now && ids.includes(now);
  });
  const playing = usePlayer((s) => here && (s.state?.status === "playing" || s.state?.status === "buffering" || s.state?.status === "loading"));
  return { here, playing };
}

/** Shuffle for a list played as plain songs: plays it once in a new order. */
function ShuffleOnce({ ids, play }: { ids: string[]; play: (ids: string[]) => void }) {
  return (
    <button
      type="button"
      className="ph-icon-btn ph-pill-btn"
      aria-label="Shuffle play"
      disabled={!ids.length}
      onClick={() => {
        tap("selection");
        play(shuffled(ids));
      }}
    >
      <Shuffle size={16} aria-hidden="true" />
      <span aria-hidden="true">Shuffle</span>
    </button>
  );
}

/** A new order for `ids` (Fisher-Yates). */
export function shuffled<T>(ids: T[], random: () => number = Math.random): T[] {
  const out = [...ids];
  for (let i = out.length - 1; i > 0; i--) {
    const j = Math.floor(random() * (i + 1));
    [out[i], out[j]] = [out[j], out[i]];
  }
  return out;
}

/** A smart playlist made on this device: the songs its rules find, kept fresh when they change. */
function PhoneSmartPlaylist({ id }: { id: string }) {
  const [version, setVersion] = useState(0);
  const { data, error, retry } = useLoad(() => backend.smartPage(id), [id, version]);
  useEffect(() => {
    let un: (() => void) | undefined;
    let alive = true;
    void backend
      .listen("smart://changed", () => setVersion((v) => v + 1))
      .then((u) => (alive ? (un = u) : u()))
      .catch(() => undefined);
    return () => {
      alive = false;
      un?.();
    };
  }, []);
  const ids = useMemo(() => data?.tracks.map((t) => t.id) ?? [], [data]);
  const { here, playing } = useSongsHere(ids);
  if (error && !data) return <PageFailed what="this smart playlist" retry={retry} framed />;
  if (!data) return <PageLoading />;
  const pl = data.playlist;
  return (
    <SongList
      title={pl.title}
      cover={<Art path={null} fluid requestSize={400} kind="playlist" />}
      kindLine="Smart playlist"
      sheet={{ id: pl.id, title: pl.title, smart: true, trackCount: pl.trackCount, durationMs: pl.durationMs, thumb: null }}
      tracks={data.tracks}
      here={here}
      playing={playing}
      shuffle={<ShuffleOnce ids={ids} play={(order) => void player.playTracks(order)} />}
      play={(start, _shuffle, only) => void player.playTracks(only ?? ids, start)}
      empty="No songs match these rules right now."
    />
  );
}

/** A playlist someone shared with you: the songs you have, and a line for the ones you don't. */
function PhoneSharedPlaylist({ id }: { id: string }) {
  const version = useCircle((s) => s.version);
  const { data, error, retry } = useLoad(() => backend.collabPage(id), [id, version]);
  const tracks = useMemo(() => data?.rows.flatMap((r) => (r.track ? [r.track] : [])) ?? [], [data]);
  const ids = useMemo(() => tracks.map((t) => t.id), [tracks]);
  const { here, playing } = useSongsHere(ids);
  if (error && !data) return <PageFailed what="this playlist" retry={retry} framed />;
  if (!data) return <PageLoading />;
  const s = data.summary;
  const missing = data.rows.length - tracks.length;
  const thumb = tracks.find((t) => t.thumb)?.thumb ?? null;
  return (
    <SongList
      title={s.title}
      cover={<Art path={thumb} fluid requestSize={400} kind="playlist" />}
      kindLine={s.role === "owner" ? "Shared playlist" : `Shared by ${s.owner.name}`}
      sheet={{ id: s.id, title: s.title, smart: false, trackCount: data.rows.length, durationMs: data.durationMs, thumb }}
      tracks={tracks}
      here={here}
      playing={playing}
      shuffle={<ShuffleOnce ids={ids} play={(order) => void player.playTracks(order)} />}
      play={(start, _shuffle, only) => void (only ? player.playTracks(only, start) : playShared(id, start))}
      note={missing > 0 ? `${countLine(missing, "song")} ${missing === 1 ? "isn't" : "aren't"} in your library.` : undefined}
      empty={s.gone ? "This playlist isn't shared with you any more." : "No songs here yet."}
    />
  );
}

/** A Blend: everyone's taste taking turns, made again each day. */
function PhoneBlend({ id }: { id: string }) {
  const version = useCircle((s) => s.version);
  const { data, error, retry } = useLoad(() => backend.blendPage(id), [id, version]);
  const live = useCircle((s) => s.view.blends.find((b) => b.id === id) ?? null);
  const tracks = useMemo(() => data?.rows.map((r) => r.track) ?? [], [data]);
  const ids = useMemo(() => tracks.map((t) => t.id), [tracks]);
  const { here, playing } = useSongsHere(ids);
  if (error && !data) return <PageFailed what="this Blend" retry={retry} framed />;
  if (!data) return <PageLoading />;
  const s = live ?? data.summary;
  return (
    <SongList
      title={s.title}
      cover={<BlendCover people={s.people} size={176} />}
      kindLine="Blend"
      sheet={{ id: s.id, title: s.title, smart: false, trackCount: tracks.length, durationMs: data.durationMs, thumb: null }}
      tracks={tracks}
      here={here}
      playing={playing}
      shuffle={<ShuffleOnce ids={ids} play={(order) => void player.playTracks(order)} />}
      play={(start, _shuffle, only) => void (only ? player.playTracks(only, start) : playBlend(id, start))}
      empty="Nothing in this Blend yet. It fills in as everyone listens."
    />
  );
}

function SongList({
  title,
  cover,
  kindLine,
  sheet,
  tracks,
  here,
  playing,
  shuffle,
  download,
  play,
  inPlaylist,
  note,
  empty,
}: {
  title: string;
  cover: ReactNode;
  /** What kind of list this is, ahead of the count ("Smart playlist"). */
  kindLine?: string;
  /** The playlist the More sheet is for. */
  sheet?: Playlist;
  tracks: Track[];
  /** This list is what's playing. */
  here: boolean;
  playing: boolean;
  shuffle: ReactNode;
  download?: ReactNode;
  play: (startTrackId: string | undefined, shuffle: boolean | undefined, trackIds?: string[]) => void;
  inPlaylist?: (index: number) => import("../phoneUi").InPlaylist;
  note?: string;
  /** Said in place of the songs when there are none. */
  empty?: string;
}) {
  const [genre, setGenre] = useState<string | null>(null);
  const genres = useMemo(() => topGenres(tracks), [tracks]);
  const rows = useMemo(() => tracks.map((t, i) => ({ t, i })).filter(({ t }) => !genre || (t.genres ?? []).includes(genre)), [tracks, genre]);
  const total = tracks.reduce((n, t) => n + t.durationMs, 0);
  const ids = genre ? rows.map((r) => r.t.id) : undefined;
  return (
    <div className="ph-page ph-playlist">
      <BackBar title={title} after={200} />
      <header className="ph-playlist__head">
        <div className="ph-playlist__cover">{cover}</div>
        <h1 className="ph-playlist__title">{title}</h1>
        <p className="ph-playlist__count tabular">
          {kindLine ? `${kindLine}, ` : ""}
          {countLine(tracks.length, "song")}, {formatLongDuration(total)}
        </p>
        {note && <p className="ph-playlist__count">{note}</p>}
      </header>
      <div className="ph-actions">
        <div className="ph-actions__left">
          <PlayPill
            playing={playing}
            label={playing ? "Pause" : `Play ${title}`}
            disabled={!tracks.length}
            onClick={() => (here && !genre ? void player.toggle() : play(undefined, undefined, ids))}
          />
          {shuffle}
        </div>
        <div className="ph-actions__right">
          {download}
          {sheet && (
            <button type="button" className="ph-icon-btn" aria-label={`More for ${title}`} onClick={() => phone.push({ kind: "playlist", playlist: sheet })}>
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
      {!tracks.length && empty ? (
        <div className="ph-empty" role="status">
          <p className="ph-empty__title">{empty}</p>
        </div>
      ) : (
        <ul className="ph-songs" aria-label="Songs">
          {rows.map(({ t, i }) => (
            <SongRow key={`${t.id}:${i}`} track={t} onPlay={() => play(t.id, undefined, ids)} inPlaylist={inPlaylist?.(i)} />
          ))}
        </ul>
      )}
    </div>
  );
}
