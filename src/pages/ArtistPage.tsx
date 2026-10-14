import { Ellipsis, Pause, Play, Shuffle } from "lucide-react";
import { openContextMenu } from "../components/ContextMenu";
import { usePins } from "../stores/pins";
import { useState } from "react";
import { Art } from "../components/Art";
import { IconButton } from "../components/Button";
import { Shelf, ShelfItem } from "../components/Shelf";
import { Skeleton } from "../components/Skeleton";
import { TrackList, TrackRow, TrackRowSkeleton } from "../components/TrackRow";
import { backend } from "../lib/backend";
import { SECTION_LABELS, plural } from "../lib/format";
import { useLoad } from "../lib/useLoad";
import { useArtTint } from "../lib/useTint";
import { player, usePlayer } from "../stores/player";
import { nav } from "../stores/router";
import { AlbumCard, useContextState } from "./cards";
import { usePageBar, useTitleStep } from "./detail";

export function ArtistPage({ id }: { id: string }) {
  const { data, error } = useLoad(() => backend.artistPage(id), [id]);
  const artist = data?.artist;
  useArtTint(null, artist?.thumb);
  const [showMore, setShowMore] = useState(false);
  const nowTrackId = usePlayer((s) => s.state?.track?.id);
  const status = usePlayer((s) => s.state?.status);
  const shuffle = usePlayer((s) => s.state?.shuffledContexts?.includes(id) ?? false);
  const { isContext: isThis, playing: isThisPlaying } = useContextState("artist", id);
  const pinned = usePins((s) => s.isPinned("artist", id));
  const play = () => (isThis ? player.toggle() : player.playArtist(id));
  usePageBar(artist?.title, isThisPlaying, play);
  const { header, column, step, compact } = useTitleStep(artist?.title);

  if (error && !data) return <p className="page-pad page-error">Couldn't load this artist: {error.message}</p>;

  const popular = data ? data.popular.slice(0, showMore ? 10 : 5) : [];
  const releaseCount = data?.sections.reduce((n, s) => n + s.albums.length, 0) ?? 0;

  return (
    <div className="detail">
      <header ref={header} className={`detail-header detail-header--artist ${compact ? "detail-header--compact" : ""}`}>
        <div className="detail-header__art">
          {artist ? <Art path={artist.thumb} size={232} round kind="artist" /> : <Skeleton width={232} height={232} radius="circle" />}
        </div>
        <div ref={column} className="detail-header__text">
          {artist ? (
            <>
              <span className="detail-header__type">Artist</span>
              <h1 className="detail-header__title" data-step={step}>{artist.title}</h1>
              <div className="detail-header__meta">
                <span>{plural(releaseCount, "release")}</span>
                <span className="detail-header__muted">• {plural(artist.viewCount, "play")}</span>
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
            label={isThisPlaying ? `Pause ${artist?.title ?? "artist"}` : `Play ${artist?.title ?? "artist"}`}
            variant="accent"
            size="xl"
            disabled={!artist}
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
            label={`More options for ${artist?.title ?? "artist"}`}
            size="lg"
            icon={<Ellipsis size={22} />}
            disabled={!artist}
            onClick={(e) =>
              artist &&
              openContextMenu(
                `Options for ${artist.title}`,
                [
                  { label: "Add popular songs to queue", onSelect: () => player.queueAdd(data!.popular.map((t) => t.id), "end") },
                  {
                    label: pinned ? "Unpin from sidebar" : "Pin to sidebar",
                    onSelect: () =>
                      usePins.getState().toggle({ kind: "artist", id: artist.id, title: artist.title, subtitle: "Artist", thumb: artist.thumb }),
                  },
                ],
                e.currentTarget,
              )
            }
          />
        </div>

        <section className="detail-section" aria-labelledby="popular-h">
          <h2 id="popular-h" className="section-title">
            Popular
          </h2>
          <TrackList label="Popular tracks" header={false}>
            {data
              ? popular.map((t, i) => (
                  <TrackRow
                    key={t.id}
                    track={t}
                    index={i + 1}
                    showArt
                    showArtist={!!t.trackArtist || t.artistId !== id}
                    active={t.id === nowTrackId}
                    playing={t.id === nowTrackId && status === "playing"}
                    onPlay={() => player.playArtist(id, t.id)}
                  />
                ))
              : Array.from({ length: 5 }, (_, i) => <TrackRowSkeleton key={i} showArt />)}
          </TrackList>
          {data && data.popular.length > 5 && (
            <button type="button" className="see-more" onClick={() => setShowMore((v) => !v)} aria-expanded={showMore}>
              {showMore ? "Show less" : "See more"}
            </button>
          )}
        </section>

        {data
          ? data.sections.map((s) => (
              <Shelf key={s.section} title={SECTION_LABELS[s.section]}>
                {s.albums.map((a) => (
                  <ShelfItem key={a.id}>
                    <AlbumCard album={a} subtitle="year" />
                  </ShelfItem>
                ))}
              </Shelf>
            ))
          : <Shelf title="Albums" loading />}
        {artist && (
          <button type="button" className="see-more" onClick={() => nav.go({ name: "library", tab: "albums" })}>
            Browse all albums
          </button>
        )}
      </div>
    </div>
  );
}
