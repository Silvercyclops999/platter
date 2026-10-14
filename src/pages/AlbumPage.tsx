import { Disc3, Ellipsis, List, Pause, Play, Rows3, Shuffle } from "lucide-react";
import { Fragment, useRef, useState } from "react";
import { Art } from "../components/Art";
import { Badge, QualityBadge } from "../components/Badge";
import { IconButton } from "../components/Button";
import { openContextMenu } from "../components/ContextMenu";
import { shareItem } from "../components/ShareMenu";
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
import { transcribe, useTranscribe } from "../stores/transcribe";
import { AlbumCard, useContextState } from "./cards";
import { usePageBar, useTitleStep } from "./detail";
import { EditionPicker } from "./EditionPicker";
import { ALBUM_VIEW_LABEL, discGroups, formatReleaseDate, labelLine, loadAlbumView, saveAlbumView, type AlbumView } from "./album";
import "./album.css";

export function AlbumPage({ id }: { id: string }) {
  const { data, error } = useLoad(() => backend.albumPage(id), [id]);
  const album = data?.album;
  useArtTint(album?.tint, album?.thumb);
  const nowTrackId = usePlayer((s) => s.state?.track?.id);
  const status = usePlayer((s) => s.state?.status);
  const shuffle = usePlayer((s) => s.state?.shuffledContexts?.includes(id) ?? false);
  const { isContext, playing: isThisPlaying } = useContextState("album", id);
  const pinned = usePins((s) => s.isPinned("album", id));
  const transcribeOn = useTranscribe((s) => !!s.status?.enabled && s.status.available);
  const play = () => (isContext ? player.toggle() : player.playAlbum(id));
  usePageBar(album?.title, isThisPlaying, play);
  const { header, column, step, compact } = useTitleStep(album?.title);
  const [coverOpen, setCoverOpen] = useState(false);
  const coverBtn = useRef<HTMLButtonElement>(null);
  const [view, setView] = useState<AlbumView>(loadAlbumView);
  const chooseView = (v: AlbumView) => {
    setView(v);
    saveAlbumView(v);
  };
  const viewMenu = (el: HTMLElement) =>
    openContextMenu(
      "View as",
      (["compact", "list"] as AlbumView[]).map((v, i) => ({
        heading: i === 0 ? "View as" : undefined,
        label: ALBUM_VIEW_LABEL[v],
        checked: view === v,
        onSelect: () => chooseView(v),
      })),
      el,
    );

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
                {data!.artistThumb && <Art path={data!.artistThumb} size={24} round kind="artist" className="detail-header__artist-photo" />}
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
              <EditionPicker albumId={album.id} />
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
                  { label: "Add to queue", onSelect: () => void player.addToQueue(data!.tracks, album.thumb) },
                  addToPlaylistItem(() => data?.tracks.map((t) => t.id) ?? []),
                  { label: "Go to artist", onSelect: () => nav.go({ name: "artist", id: album.artistId }) },
                  shareItem("album", album.id, album.thumb),
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
                  ...(transcribeOn
                    ? [{ label: "Transcribe songs without lyrics", onSelect: () => void transcribe.album(album.id) }]
                    : []),
                ],
                e.currentTarget,
              )
            }
          />
          <button
            type="button"
            className="pl-sort"
            aria-haspopup="menu"
            aria-label={`View as: ${ALBUM_VIEW_LABEL[view]}`}
            onClick={(e) => viewMenu(e.currentTarget)}
          >
            <span>{ALBUM_VIEW_LABEL[view]}</span>
            {view === "list" ? <List size={16} aria-hidden="true" /> : <Rows3 size={16} aria-hidden="true" />}
          </button>
        </div>

        <div className={`album-tracks album-tracks--${view}`}>
          <TrackList label={album ? `${album.title} tracks` : "Tracks"} showAlbum={false} showPlays>
            {data
              ? discGroups(data.tracks).map((g) => (
                  <Fragment key={g.disc ?? "all"}>
                    {g.disc !== null && (
                      <div className="album-disc" role="listitem" aria-label={`Disc ${g.disc}`}>
                        <span className="album-disc__icon">
                          <Disc3 size={16} aria-hidden="true" />
                        </span>
                        <span className="album-disc__label">Disc {g.disc}</span>
                      </div>
                    )}
                    {g.tracks.map((t) => (
                      <TrackRow
                        key={t.id}
                        track={t}
                        index={t.trackNumber}
                        showAlbum={false}
                        showPlays
                        showArtist={view === "list" && (!!t.trackArtist || t.artistId !== album!.artistId)}
                        active={t.id === nowTrackId}
                        playing={t.id === nowTrackId && status === "playing"}
                        onPlay={() => player.playAlbum(id, t.id)}
                      />
                    ))}
                  </Fragment>
                ))
              : Array.from({ length: 8 }, (_, i) => <TrackRowSkeleton key={i} showAlbum={false} showPlays />)}
          </TrackList>
        </div>

        {album && <AlbumFooter released={data!.released} studio={data!.studio} year={album.year} />}

        {album && data!.moreByArtist.length > 0 && (
          <div className="detail-more">
            <Shelf
              title={`More by ${album.artistTitle}`}
              showAllLabel="See discography"
              onShowAll={() => nav.go({ name: "discography", id: album.artistId })}
            >
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

/** Under the tracklist: the full release date, then the label line. Hidden when Plex has neither. */
function AlbumFooter({ released, studio, year }: { released?: string | null; studio?: string | null; year: number | null }) {
  const date = formatReleaseDate(released);
  const label = labelLine(studio, year);
  if (!date && !label) return null;
  return (
    <footer className="album-footer">
      {date && <p className="album-footer__date">{date}</p>}
      {label && <p className="album-footer__label">{label}</p>}
    </footer>
  );
}
