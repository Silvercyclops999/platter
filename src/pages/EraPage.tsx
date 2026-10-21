import { Ellipsis, Pause, Play, Shuffle } from "lucide-react";
import { useEffect } from "react";
import { Art } from "../components/Art";
import { Badge } from "../components/Badge";
import { IconButton } from "../components/Button";
import { openContextMenu } from "../components/ContextMenu";
import { Skeleton } from "../components/Skeleton";
import { tagEraItems } from "../components/TagEditMenu";
import { TrackList, TrackRow, TrackRowSkeleton } from "../components/TrackRow";
import { backend } from "../lib/backend";
import { plural, releaseTypeLabel } from "../lib/format";
import { useLoad, useReload } from "../lib/useLoad";
import { useArtTint } from "../lib/useTint";
import { player, usePlayer } from "../stores/player";
import { nav } from "../stores/router";
import { usePageBar, useTitleStep } from "./detail";
import { eraRange, formatArchiveDate } from "./era";
import { EraStory } from "./EraStory";
import { LoadError } from "../components/LoadError";

/** A copy in random order (Fisher-Yates). */
function shuffled<T>(list: T[]): T[] {
  const out = [...list];
  for (let i = out.length - 1; i > 0; i--) {
    const j = Math.floor(Math.random() * (i + 1));
    [out[i], out[j]] = [out[j], out[i]];
  }
  return out;
}

/**
 * One era of an artist (`/artist/:id/era/:name`): the era's dates, Play and
 * Shuffle, the era in a sentence with its releases on a timeline, then
 * every song of it by album or build, newest build first, released and
 * unreleased alike.
 */
export function EraPage({ artistId, name }: { artistId: string; name: string }) {
  const { data: loaded, error, retry } = useLoad(() => backend.eraPage(artistId, name), [artistId, name]);
  const { data, reload } = useReload(loaded, () => backend.eraPage(artistId, name));
  useEffect(() => {
    const uns = [backend.listen("tags://changed", reload), backend.listen("library://changed", reload)];
    return () => uns.forEach((un) => un.then((f) => f()));
  }, [artistId, name, reload]);
  const era = data?.era;
  useArtTint(null, era?.cover ?? undefined);
  const ids = data ? data.groups.flatMap((g) => g.tracks.map((t) => t.id)) : [];
  const nowId = usePlayer((s) => s.state?.track?.id);
  const status = usePlayer((s) => s.state?.status);
  const playingHere = usePlayer((s) => s.state?.queue.context?.kind === "tracks" && s.state.queue.context.id !== "albums" && !!nowId && ids.includes(nowId) && s.state?.status === "playing");
  const play = () => (playingHere ? player.toggle() : void player.playTracks(ids));
  usePageBar(era ? `${era.name} era` : undefined, playingHere, play);
  const { header, column, step, compact } = useTitleStep(era?.name);

  if (error && !data) return <LoadError what="this era" error={error} onRetry={retry} />;

  const range = era ? eraRange(era) : null;
  return (
    <div className="detail era-page">
      <header ref={header} className={`detail-header ${compact ? "detail-header--compact" : ""}`}>
        <div className="detail-header__art">{era ? <Art path={era.cover} size={232} alt="" /> : <Skeleton width={232} height={232} />}</div>
        <div ref={column} className="detail-header__text">
          {era && data ? (
            <>
              <span className="detail-header__type">Era</span>
              <h1 className="detail-header__title" data-step={step}>
                {era.name}
              </h1>
              <div className="detail-header__meta">
                <button type="button" className="link detail-header__artist" onClick={() => nav.go({ name: "artist", id: artistId })}>
                  {data.artist.title}
                </button>
                {range && <span>• {range}</span>}
                <span>
                  • {plural(era.trackCount, "song")}, {plural(era.albumCount, "release")}
                </span>
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
            label={playingHere ? `Pause ${name}` : `Play ${name}`}
            variant="accent"
            size="xl"
            text={playingHere ? "Pause" : "Play"}
            disabled={!data}
            icon={playingHere ? <Pause size={24} fill="currentColor" /> : <Play size={24} fill="currentColor" className="nudge-right" />}
            onClick={play}
          />
          <IconButton label={`Shuffle ${name}`} size="lg" disabled={!data} icon={<Shuffle size={22} />} onClick={() => void player.playTracks(shuffled(ids))} />
          <IconButton
            label={`More options for ${name}`}
            size="lg"
            icon={<Ellipsis size={22} />}
            onClick={(e) => openContextMenu(`Options for ${name}`, tagEraItems(artistId, name), e.currentTarget)}
          />
          <button type="button" className="link era-page__all" onClick={() => nav.go({ name: "artist", id: artistId, era: name })}>
            Show on the artist page
          </button>
        </div>

        {data && <EraStory data={data} />}

        {data
          ? data.groups.map((g) => (
              <section key={g.album.id} className="era-group" aria-label={g.album.title}>
                <header className="era-group__head">
                  <button type="button" className="era-group__art" tabIndex={-1} aria-hidden="true" onClick={() => nav.go({ name: "album", id: g.album.id })}>
                    <Art path={g.album.thumb} size={56} />
                  </button>
                  <div className="era-group__text">
                    <h2 className="era-group__title">
                      <button type="button" className="link" onClick={() => nav.go({ name: "album", id: g.album.id })}>
                        {g.album.title}
                      </button>
                    </h2>
                    <p className="era-group__meta">
                      {[releaseTypeLabel(g.album), g.date ? formatArchiveDate(g.date) : g.album.year ? String(g.album.year) : null, plural(g.tracks.length, "song")]
                        .filter(Boolean)
                        .join(" · ")}
                    </p>
                  </div>
                  {g.edition && <Badge tone="neutral">{g.edition}</Badge>}
                </header>
                <TrackList label={`${g.album.title} songs`} showAlbum={false}>
                  {g.tracks.map((t, i) => (
                    <TrackRow
                      key={t.id}
                      track={t}
                      index={i + 1}
                      showAlbum={false}
                      showArtist={!!t.trackArtist}
                      active={t.id === nowId}
                      playing={t.id === nowId && status === "playing"}
                      onPlay={() => void player.playTracks(ids, t.id)}
                    />
                  ))}
                </TrackList>
              </section>
            ))
          : Array.from({ length: 2 }, (_, i) => (
              <section key={i} className="era-group" aria-hidden="true">
                <header className="era-group__head">
                  <Skeleton width={56} height={56} />
                  <div className="era-group__text">
                    <Skeleton width={200} height={18} />
                    <Skeleton width={140} height={12} style={{ marginTop: 6 }} />
                  </div>
                </header>
                {Array.from({ length: 4 }, (_, j) => (
                  <TrackRowSkeleton key={j} showAlbum={false} />
                ))}
              </section>
            ))}
      </div>
    </div>
  );
}
