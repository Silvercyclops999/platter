import { Ellipsis, Pause, Play, Shuffle } from "lucide-react";
import { useRef, useState } from "react";
import { Art } from "../components/Art";
import { Badge, QualityBadge } from "../components/Badge";
import { IconButton } from "../components/Button";
import { openContextMenu } from "../components/ContextMenu";
import { addToPlaylistItem } from "../components/PlaylistPicker";
import { CoverPopup } from "../components/CoverPopup";
import { Shelf, ShelfItem } from "../components/Shelf";
import { Skeleton } from "../components/Skeleton";
import { TrackList, TrackRow, TrackRowSkeleton } from "../components/TrackRow";
import { backend } from "../lib/backend";
import { formatLongDuration, plural, releaseTypeLabel } from "../lib/format";
import { useLoad } from "../lib/useLoad";
import { useArtTint } from "../lib/useTint";
import { usePins } from "../stores/pins";
import { player, usePlayer } from "../stores/player";
import { nav } from "../stores/router";
import { copyText } from "../stores/toasts";
import { AlbumCard, useContextState } from "./cards";
import { usePageBar, useTitleStep } from "./detail";

export function AlbumPage({ id }: { id: string }) {
  const { data, error } = useLoad(() => backend.albumPage(id), [id]);
  const album = data?.album;
  useArtTint(album?.tint, album?.thumb);
  const nowTrackId = usePlayer((s) => s.state?.track?.id);
  const status = usePlayer((s) => s.state?.status);
  const shuffle = usePlayer((s) => s.state?.shuffledContexts?.includes(id) ?? false);
  const { isContext, playing: isThisPlaying } = useContextState("album", id);
  const pinned = usePins((s) => s.isPinned("album", id));
  const play = () => (isContext ? player.toggle() : player.playAlbum(id));
  usePageBar(album?.title, isThisPlaying, play);
  const { header, column, step, compact } = useTitleStep(album?.title);
  const [coverOpen, setCoverOpen] = useState(false);
  const coverBtn = useRef<HTMLButtonElement>(null);

  if (error && !data) return <p className="page-pad page-error">Couldn't load this album: {error.message}</p>;

  const firstTrack = data?.tracks[0];
  const untagged = album && album.format === null;

  return (
    <div className="detail">
      <header ref={header} className={`detail-header ${compact ? "detail-header--compact" : ""}`}>
        <div className="detail-header__art">
          {album ? (
            <button
              ref={coverBtn}
              type="button"
              className="cover-button"
              aria-label="View the cover art"
              aria-haspopup="dialog"
              onClick={() => setCoverOpen(true)}
            >
              <Art path={album.thumb} size={232} alt="" />
            </button>
          ) : (
            <Skeleton width={232} height={232} />
          )}
        </div>
        <div ref={column} className="detail-header__text">
          {album ? (
            <>
              <span className="detail-header__type">{releaseTypeLabel(album)}</span>
              <h1 className="detail-header__title" data-step={step}>{album.title}</h1>
              <div className="detail-header__meta">
                <button type="button" className="link detail-header__artist" onClick={() => nav.go({ name: "artist", id: album.artistId })}>
                  {album.artistTitle}
                </button>
                {album.year && <span>• {album.year}</span>}
                <span>• {plural(data!.tracks.length, "song")},</span>
                <span className="detail-header__muted">{formatLongDuration(data!.totalDurationMs)}</span>
              </div>
              <div className="detail-header__badges">
                {firstTrack && <QualityBadge track={firstTrack} />}
                {untagged && (
                  <Badge tone="version" title="No release type tag yet; listed under Albums">
                    Untagged
                  </Badge>
                )}
                {album.genres.slice(0, 2).map((g) => (
                  <Badge key={g}>{g}</Badge>
                ))}
              </div>
            </>
          ) : (
            <>
              <Skeleton width={60} height={12} />
              <Skeleton width="60%" height={48} style={{ margin: "12px 0" }} />
              <Skeleton width="40%" height={14} />
            </>
          )}
        </div>
      </header>

      {album && (
        <CoverPopup open={coverOpen} onClose={() => setCoverOpen(false)} path={album.thumb} title={album.title} returnFocus={coverBtn} />
      )}

      <div className="detail-body">
        <div className="action-row">
          <IconButton
            label={isThisPlaying ? `Pause ${album?.title ?? "album"}` : `Play ${album?.title ?? "album"}`}
            variant="accent"
            size="xl"
            disabled={!album}
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
            label={`More options for ${album?.title ?? "album"}`}
            size="lg"
            icon={<Ellipsis size={22} />}
            disabled={!album}
            onClick={(e) =>
              album &&
              openContextMenu(
                `Options for ${album.title}`,
                [
                  { label: "Add to queue", onSelect: () => player.queueAdd(data!.tracks.map((t) => t.id), "end") },
                  addToPlaylistItem(() => data?.tracks.map((t) => t.id) ?? []),
                  { label: "Go to artist", onSelect: () => nav.go({ name: "artist", id: album.artistId }) },
                  {
                    label: pinned ? "Unpin from sidebar" : "Pin to sidebar",
                    onSelect: () =>
                      usePins
                        .getState()
                        .toggle({ kind: "album", id: album.id, title: album.title, subtitle: `Album · ${album.artistTitle}`, thumb: album.thumb }),
                  },
                  {
                    label: "Copy album title",
                    onSelect: () => copyText(`${album.title} - ${album.artistTitle}`),
                  },
                ],
                e.currentTarget,
              )
            }
          />
        </div>

        <TrackList label={album ? `${album.title} tracks` : "Tracks"} showAlbum={false} showPlays>
          {data
            ? data.tracks.map((t) => (
                <TrackRow
                  key={t.id}
                  track={t}
                  index={t.trackNumber}
                  showAlbum={false}
                  showPlays
                  showArtist={!!t.trackArtist || t.artistId !== album!.artistId}
                  active={t.id === nowTrackId}
                  playing={t.id === nowTrackId && status === "playing"}
                  onPlay={() => player.playAlbum(id, t.id)}
                />
              ))
            : Array.from({ length: 8 }, (_, i) => <TrackRowSkeleton key={i} showAlbum={false} showPlays />)}
        </TrackList>

        {album && data!.moreByArtist.length > 0 && (
          <div className="detail-more">
            <Shelf title={`More by ${album.artistTitle}`} onShowAll={() => nav.go({ name: "artist", id: album.artistId })}>
              {data!.moreByArtist.map((a) => (
                <ShelfItem key={a.id}>
                  <AlbumCard album={a} subtitle="year" />
                </ShelfItem>
              ))}
            </Shelf>
          </div>
        )}
      </div>
    </div>
  );
}
