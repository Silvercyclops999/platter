import { Pause, Play, Shuffle } from "lucide-react";
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
import { AlbumCard } from "./cards";

export function ArtistPage({ id }: { id: string }) {
  const { data, error } = useLoad(() => backend.artistPage(id), [id]);
  const artist = data?.artist;
  useArtTint(null, artist?.thumb);
  const [showMore, setShowMore] = useState(false);
  const nowTrackId = usePlayer((s) => s.state?.track?.id);
  const status = usePlayer((s) => s.state?.status);
  const ctx = usePlayer((s) => s.state?.queue.context);
  const isThis = ctx?.kind === "artist" && ctx.id === id;

  if (error && !data) return <p className="page-pad page-error">Couldn't load this artist: {error.message}</p>;

  const popular = data ? data.popular.slice(0, showMore ? 10 : 5) : [];
  const releaseCount = data?.sections.reduce((n, s) => n + s.albums.length, 0) ?? 0;

  return (
    <div className="detail">
      <header className="detail-header detail-header--artist">
        <div className="detail-header__art">
          {artist ? <Art path={artist.thumb} size={232} round kind="artist" /> : <Skeleton width={232} height={232} radius="circle" />}
        </div>
        <div className="detail-header__text">
          {artist ? (
            <>
              <span className="detail-header__type">Artist</span>
              <h1 className={`detail-header__title ${artist.title.length > 24 ? "is-long" : ""}`}>{artist.title}</h1>
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
            label={isThis && status === "playing" ? `Pause ${artist?.title ?? "artist"}` : `Play ${artist?.title ?? "artist"}`}
            variant="accent"
            size="xl"
            disabled={!artist}
            icon={isThis && status === "playing" ? <Pause size={24} fill="currentColor" /> : <Play size={24} fill="currentColor" className="nudge-right" />}
            onClick={() => (isThis ? player.toggle() : player.playArtist(id))}
          />
          <IconButton label="Shuffle play" size="lg" icon={<Shuffle size={22} />} disabled={!artist} onClick={() => player.playArtist(id, undefined, true)} />
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
                    showArtist={false}
                    active={t.id === nowTrackId}
                    playing={t.id === nowTrackId && status === "playing"}
                    onPlay={() => player.playTracks(data.popular.map((p) => p.id), t.id)}
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
