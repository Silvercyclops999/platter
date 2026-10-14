import { Ellipsis, ListMusic, Pause, Play, Shuffle } from "lucide-react";
import { openContextMenu } from "../components/ContextMenu";
import { usePins } from "../stores/pins";
import { useContextState } from "./cards";
import { usePageBar, useTitleStep } from "./detail";
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
  const shuffle = usePlayer((s) => s.state?.shuffledContexts?.includes(id) ?? false);
  const { isContext: isThis, playing: isThisPlaying } = useContextState("playlist", id);
  const pinned = usePins((s) => s.isPinned("playlist", id));
  const play = () => (isThis ? player.toggle() : player.playPlaylist(id));
  usePageBar(pl?.title, isThisPlaying, play);
  const { header, column, step, compact } = useTitleStep(pl?.title);
  const empty = !!data && data.tracks.length === 0;

  if (error && !data) return <p className="page-pad page-error">Couldn't load this playlist: {error.message}</p>;

  return (
    <div className="detail">
      <header ref={header} className={`detail-header ${compact ? "detail-header--compact" : ""}`}>
        <div className="detail-header__art">
          {pl ? <Art path={pl.thumb} size={232} kind="playlist" /> : <Skeleton width={232} height={232} />}
        </div>
        <div ref={column} className="detail-header__text">
          {pl ? (
            <>
              <span className="detail-header__type">{pl.smart ? "Smart playlist" : "Playlist"}</span>
              <h1 className="detail-header__title" data-step={step}>{pl.title}</h1>
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
            label={isThisPlaying ? `Pause ${pl?.title ?? "playlist"}` : `Play ${pl?.title ?? "playlist"}`}
            variant="accent"
            size="xl"
            disabled={!pl || empty}
            icon={isThisPlaying ? <Pause size={24} fill="currentColor" /> : <Play size={24} fill="currentColor" className="nudge-right" />}
            onClick={play}
          />
          <IconButton
            label={shuffle ? "Disable shuffle" : "Enable shuffle"}
            size="lg"
            pressed={shuffle}
            icon={<Shuffle size={22} />}
            onClick={() => player.setContextShuffle(id, !shuffle)}
          />
          <IconButton
            label={`More options for ${pl?.title ?? "playlist"}`}
            size="lg"
            icon={<Ellipsis size={22} />}
            disabled={!pl}
            onClick={(e) =>
              pl &&
              openContextMenu(
                `Options for ${pl.title}`,
                [
                  { label: "Add to queue", disabled: empty, onSelect: () => player.queueAdd(data!.tracks.map((t) => t.id), "end") },
                  {
                    label: pinned ? "Unpin from sidebar" : "Pin to sidebar",
                    separator: true,
                    onSelect: () =>
                      usePins.getState().toggle({
                        kind: "playlist",
                        id: pl.id,
                        title: pl.title,
                        subtitle: `Playlist · ${pl.trackCount} songs`,
                        thumb: pl.thumb,
                      }),
                  },
                ],
                e.currentTarget,
              )
            }
          />
        </div>
        {empty ? (
          <div className="empty-state">
            <ListMusic size={40} strokeWidth={1.5} aria-hidden="true" />
            <h2 className="empty-state__title">This playlist is empty</h2>
            <p className="empty-state__text">Songs added to it in Plex will show up here.</p>
          </div>
        ) : (
        <TrackList label={pl ? `${pl.title} tracks` : "Tracks"}>
          {data
            ? data.tracks.map((t, i) => (
                <TrackRow
                  key={`${t.id}-${i}`}
                  rowKey={`${t.id}-${i}`}
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
        )}
      </div>
    </div>
  );
}
