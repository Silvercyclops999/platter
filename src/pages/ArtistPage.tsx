import { Ellipsis, Pause, Pin, PinOff, Play, Shuffle } from "lucide-react";
import { useEffect, useId, useState, type RefObject } from "react";
import { Art } from "../components/Art";
import { Button, IconButton } from "../components/Button";
import { openContextMenu } from "../components/ContextMenu";
import { shareItem } from "../components/ShareMenu";
import { Shelf, ShelfItem } from "../components/Shelf";
import { Skeleton } from "../components/Skeleton";
import { FilterChips } from "../components/Tabs";
import { TrackList, TrackRow, TrackRowSkeleton } from "../components/TrackRow";
import { artUrl, backend } from "../lib/backend";
import { useLoad } from "../lib/useLoad";
import { useArtTint } from "../lib/useTint";
import type { Artist, ArtistPage as ArtistPageData, ArtistPageSection } from "../lib/types";
import { usePins } from "../stores/pins";
import { player, usePlayer } from "../stores/player";
import { AlbumCard, ArtistCard, useContextState } from "./cards";
import { usePageBar, useTitleStep } from "./detail";
import { AboutCard, useArtistInfo } from "./ArtistAbout";
import {
  POPULAR_FIRST,
  POPULAR_MORE,
  aboutText,
  artistSections,
  bannerPath,
  discographyChips,
  discographyItems,
  headerVariant,
  listenersLine,
  playsLine,
  type DiscographyFilter,
} from "./artist";

/** Page state (see more, chip, expanded rows) starts over for each artist. */
export function ArtistPage({ id }: { id: string }) {
  return <ArtistView key={id} id={id} />;
}

function ArtistView({ id }: { id: string }) {
  const { data: loaded, error } = useLoad(() => backend.artistPage(id), [id]);
  // Similar artists fetched in the background after the page opened.
  const [similar, setSimilar] = useState<Artist[] | null>(null);
  useEffect(() => {
    let alive = true;
    const un = backend.listen("artist://changed", (e) => {
      if (e.id !== id) return;
      backend.artistPage(id).then(
        (p) => alive && setSimilar(p.similar),
        () => {},
      );
    });
    return () => {
      alive = false;
      un.then((f) => f());
    };
  }, [id]);
  const data = loaded && similar ? { ...loaded, similar } : loaded;
  const artist = data?.artist;
  const info = useArtistInfo(id);
  const banner = artist ? bannerPath(artist, info) : null;
  const variant = artist ? headerVariant(artist, banner) : "photo";
  const about = data ? aboutText(data.about, info) : null;
  useArtTint(null, artist ? (artist.art ?? artist.thumb) : undefined);
  const { isContext: isThis, playing: isThisPlaying } = useContextState("artist", id);
  const play = () => (isThis ? player.toggle() : player.playArtist(id));
  usePageBar(artist?.title, isThisPlaying, play);
  const { header, column, step, compact } = useTitleStep(artist?.title);

  if (error && !data) return <p className="page-pad page-error">Couldn't load this artist: {error.message}</p>;

  const sections = data ? artistSections({ ...data, about, listeners: info?.listeners }) : [];
  const text = artist ? (
    <>
      <span className="detail-header__type">Artist</span>
      <h1 className="detail-header__title" data-step={step}>
        {artist.title}
      </h1>
      <div className="detail-header__meta">
        <span>{playsLine(data!.plays)}</span>
        {info?.listeners != null && <span className="artist-listeners">{listenersLine(info.listeners)}</span>}
      </div>
    </>
  ) : (
    <>
      <Skeleton width={60} height={12} />
      <Skeleton width="50%" height={48} style={{ margin: "12px 0" }} />
      <Skeleton width="30%" height={14} />
    </>
  );

  return (
    <div className="detail artist-page">
      {variant === "banner" && artist ? (
        <header
          ref={header}
          className={`artist-banner ${compact ? "artist-banner--compact" : ""}`}
          data-variant="banner"
        >
          <BannerImage path={banner!} box={header} />
          <div ref={column} className="detail-header__text artist-banner__text">
            {text}
          </div>
        </header>
      ) : (
        <header
          ref={header}
          className={`detail-header detail-header--artist ${compact ? "detail-header--compact" : ""}`}
          data-variant="photo"
        >
          <div className="detail-header__art">
            {artist ? <Art path={artist.thumb} size={232} round kind="artist" /> : <Skeleton width={232} height={232} radius="circle" />}
          </div>
          <div ref={column} className="detail-header__text">
            {text}
          </div>
        </header>
      )}

      <div className="detail-body">
        <ActionRow id={id} data={data} playing={isThisPlaying} onPlay={play} />

        {!data && (
          <>
            <section className="detail-section">
              <h2 className="section-title">Popular</h2>
              <TrackList label="Popular tracks" header={false}>
                {Array.from({ length: POPULAR_FIRST }, (_, i) => (
                  <TrackRowSkeleton key={i} showArt />
                ))}
              </TrackList>
            </section>
            <Shelf title="Discography" loading />
          </>
        )}

        {data &&
          sections.map((s) => {
            switch (s) {
              case "popular":
                return <PopularSection key={s} id={id} data={data} />;
              case "discography":
                return <Discography key={s} sections={data.sections} />;
              case "about":
                return <AboutCard key={s} artist={data.artist} about={about} plays={data.plays} info={info} />;
              case "fansAlsoLike":
                return <FansAlsoLike key={s} artists={data.similar} />;
            }
          })}
      </div>
    </div>
  );
}

// ---------------------------------------------------------------- header

/** Largest banner width requested, in CSS px. */
const BANNER_MAX_W = 1920;

/**
 * The banner's background art, requested at the banner's rendered size
 * (rounded up to the shared art steps, so resizing only refetches when it
 * crosses a step).
 */
function BannerImage({ path, box }: { path: string; box: RefObject<HTMLElement | null> }) {
  const [size, setSize] = useState({ w: 1280, h: 400 });
  const [loaded, setLoaded] = useState<string | null>(null);

  useEffect(() => {
    const el = box.current;
    if (!el) return;
    const read = (w: number, h: number) => {
      if (w > 0 && h > 0) setSize((s) => (s.w === Math.round(w) && s.h === Math.round(h) ? s : { w: Math.round(w), h: Math.round(h) }));
    };
    read(el.clientWidth, el.clientHeight);
    if (typeof ResizeObserver === "undefined") return;
    const ro = new ResizeObserver((entries) => {
      for (const e of entries) read(e.contentRect.width, e.contentRect.height);
    });
    ro.observe(el);
    return () => ro.disconnect();
  }, [box]);

  const w = Math.min(size.w, BANNER_MAX_W);
  const h = Math.round(size.h * (w / size.w));
  const src = artUrl(path, w, h);
  return (
    <div className="artist-banner__art" aria-hidden="true">
      {src && (
        <img
          src={src}
          alt=""
          draggable={false}
          decoding="async"
          className={loaded ? "is-loaded" : ""}
          onLoad={() => setLoaded(src)}
        />
      )}
    </div>
  );
}

// ---------------------------------------------------------------- action row

function ActionRow({ id, data, playing, onPlay }: { id: string; data: ArtistPageData | undefined; playing: boolean; onPlay: () => void }) {
  const artist = data?.artist;
  const shuffle = usePlayer((s) => s.state?.shuffledContexts?.includes(id) ?? false);
  const pinned = usePins((s) => s.isPinned("artist", id));
  const togglePin = () =>
    artist && usePins.getState().toggle({ kind: "artist", id: artist.id, title: artist.title, subtitle: "Artist", thumb: artist.thumb });
  const name = artist?.title ?? "artist";
  return (
    <div className="action-row">
      <IconButton
        label={playing ? `Pause ${name}` : `Play ${name}`}
        variant="accent"
        size="xl"
        disabled={!artist}
        icon={playing ? <Pause size={24} fill="currentColor" /> : <Play size={24} fill="currentColor" className="nudge-right" />}
        onClick={onPlay}
      />
      <IconButton
        label={shuffle ? "Disable shuffle" : "Enable shuffle"}
        size="lg"
        pressed={shuffle}
        icon={<Shuffle size={22} />}
        onClick={() => player.setContextShuffle(id, !shuffle)}
      />
      <Button
        size="sm"
        className="artist-pin"
        aria-pressed={pinned}
        disabled={!artist}
        icon={pinned ? <PinOff size={14} /> : <Pin size={14} />}
        onClick={togglePin}
        title={pinned ? "Remove from the sidebar" : "Keep this artist in the sidebar"}
      >
        {pinned ? "Pinned" : "Pin"}
      </Button>
      <IconButton
        label={`More options for ${name}`}
        size="lg"
        icon={<Ellipsis size={22} />}
        disabled={!artist}
        onClick={(e) =>
          artist &&
          openContextMenu(
            `Options for ${artist.title}`,
            [
              { label: "Add popular songs to queue", onSelect: () => player.queueAdd(data!.popular.map((t) => t.id), "end") },
              shareItem("artist", artist.id),
              { label: pinned ? "Unpin from sidebar" : "Pin to sidebar", onSelect: togglePin },
            ],
            e.currentTarget,
          )
        }
      />
    </div>
  );
}

// ---------------------------------------------------------------- sections

function PopularSection({ id, data }: { id: string; data: ArtistPageData }) {
  const [more, setMore] = useState(false);
  const nowTrackId = usePlayer((s) => s.state?.track?.id);
  const status = usePlayer((s) => s.state?.status);
  const headingId = useId();
  const shown = data.popular.slice(0, more ? POPULAR_MORE : POPULAR_FIRST);
  return (
    <section className="detail-section artist-popular" aria-labelledby={headingId}>
      <h2 id={headingId} className="section-title">
        Popular
      </h2>
      <TrackList label="Popular tracks" header={false}>
        {shown.map((t, i) => (
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
        ))}
      </TrackList>
      {data.popular.length > POPULAR_FIRST && (
        <button type="button" className="see-more" onClick={() => setMore((v) => !v)} aria-expanded={more}>
          {more ? "Show less" : "See more"}
        </button>
      )}
    </section>
  );
}

function Discography({ sections }: { sections: ArtistPageSection[] }) {
  const [filter, setFilter] = useState<DiscographyFilter>("popular");
  const [all, setAll] = useState(false);
  const headingId = useId();
  const chips = discographyChips(sections);
  const chipRow = (
    <FilterChips label="Release type" items={chips} value={filter} onChange={setFilter} className="discography__chips" />
  );
  const cards = (limit?: number) =>
    discographyItems(sections, filter, limit).map(({ album, latest }) => (
      <ShelfItem key={album.id}>
        <AlbumCard album={album} subtitle="year" note={latest ? "Latest release" : undefined} />
      </ShelfItem>
    ));

  if (!all) {
    return (
      <div className="discography">
        <Shelf title="Discography" onShowAll={() => setAll(true)} filters={chipRow}>
          {cards()}
        </Shelf>
      </div>
    );
  }
  return (
    <section className="discography discography--all" aria-labelledby={headingId}>
      <header className="shelf__head">
        <h2 id={headingId} className="shelf__title">
          Discography
        </h2>
        <div className="shelf__tools">
          <button type="button" className="shelf__all" onClick={() => setAll(false)}>
            Show less
          </button>
        </div>
      </header>
      {chipRow}
      <div className="discography__grid" role="list">
        {cards(Infinity)}
      </div>
    </section>
  );
}

function FansAlsoLike({ artists }: { artists: Artist[] }) {
  return (
    <Shelf title="Fans also like" skeletonKind="artist">
      {artists.map((a) => (
        <ShelfItem key={a.id}>
          <ArtistCard artist={a} />
        </ShelfItem>
      ))}
    </Shelf>
  );
}
