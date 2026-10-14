import { Pause, Play, Shuffle } from "lucide-react";
import { Art } from "../components/Art";
import { IconButton } from "../components/Button";
import { Skeleton } from "../components/Skeleton";
import { TrackList, TrackRow, TrackRowSkeleton } from "../components/TrackRow";
import { backend } from "../lib/backend";
import { formatLongDuration, plural } from "../lib/format";
import { useLoad } from "../lib/useLoad";
import { useArtTint } from "../lib/useTint";
import { player, usePlayer } from "../stores/player";

export function PlaylistPage({ id }: { id: string }) {
  const { data, error } = useLoad(() => backend.playlistPage(id), [id]);
  const pl = data?.playlist;
  useArtTint(null, pl?.thumb);
  const nowTrackId = usePlayer((s) => s.state?.track?.id);
  const status = usePlayer((s) => s.state?.status);
  const ctx = usePlayer((s) => s.state?.queue.context);
  const isThis = ctx?.kind === "playlist" && ctx.id === id;

  if (error && !data) return <p className="page-pad page-error">Couldn't load this playlist: {error.message}</p>;

  return (
    <div className="detail">
      <header className="detail-header">
        <div className="detail-header__art">
          {pl ? <Art path={pl.thumb} size={232} kind="playlist" /> : <Skeleton width={232} height={232} />}
        </div>
        <div className="detail-header__text">
          {pl ? (
            <>
              <span className="detail-header__type">{pl.smart ? "Smart playlist" : "Playlist"}</span>
              <h1 className={`detail-header__title ${pl.title.length > 24 ? "is-long" : ""}`}>{pl.title}</h1>
              <div className="detail-header__meta">
                <span>{plural(pl.trackCount, "song")},</span>
                <span className="detail-header__muted">{formatLongDuration(pl.durationMs)}</span>
              </div>
            </>
          ) : (
            <>
              <Skeleton width={60} height={12} />
              <Skeleton width="50%" height={48} style={{ margin: "12px 0" }} />
              <Skeleton width="30%" height={14} />
            </>
          )}
        </div>
      </header>
      <div className="detail-body">
        <div className="action-row">
          <IconButton
            label={isThis && status === "playing" ? "Pause playlist" : "Play playlist"}
            variant="accent"
            size="xl"
            disabled={!pl}
            icon={isThis && status === "playing" ? <Pause size={24} fill="currentColor" /> : <Play size={24} fill="currentColor" className="nudge-right" />}
            onClick={() => (isThis ? player.toggle() : player.playPlaylist(id))}
          />
          <IconButton label="Shuffle play" size="lg" icon={<Shuffle size={22} />} disabled={!pl} onClick={() => player.playPlaylist(id, undefined, true)} />
        </div>
        <TrackList label={pl ? `${pl.title} tracks` : "Tracks"}>
          {data
            ? data.tracks.map((t, i) => (
                <TrackRow
                  key={`${t.id}-${i}`}
                  track={t}
                  index={i + 1}
                  showArt
                  active={t.id === nowTrackId}
                  playing={t.id === nowTrackId && status === "playing"}
                  onPlay={() => player.playPlaylist(id, t.id)}
                />
              ))
            : Array.from({ length: 8 }, (_, i) => <TrackRowSkeleton key={i} showArt />)}
        </TrackList>
      </div>
    </div>
  );
}
