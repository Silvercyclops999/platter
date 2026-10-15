import { Ellipsis, Pause, Pin, PinOff, Play, Shuffle, X } from "lucide-react";
import { Suspense, lazy, useEffect, useId, useState, type RefObject } from "react";
import { Art } from "../components/Art";
import { Button, IconButton } from "../components/Button";
import { Card, CardSkeleton } from "../components/Card";
import { openContextMenu } from "../components/ContextMenu";
import { shareItem } from "../components/ShareMenu";
import { plexEditItem } from "../components/PlexEditMenu";
import { bookmarkPageItem, dontPlayArtistItem } from "../stores/curate";
import { yourStatsItem } from "../stores/stats";
import { StatsStrip } from "../components/StatsStrip";
import { Shelf, ShelfItem } from "../components/Shelf";
import { Skeleton } from "../components/Skeleton";
import { FilterChips } from "../components/Tabs";
import { TrackList, TrackRow, TrackRowSkeleton } from "../components/TrackRow";
import { artUrl, backend } from "../lib/backend";
import { isCreditId } from "../lib/credits";
import { useLoad } from "../lib/useLoad";
import { useArtTint } from "../lib/useTint";
import type { Album, Artist, ArtistPage as ArtistPageData, ArtistPageSection, Era } from "../lib/types";
import { useSettings } from "../stores/settings";
import { eraRange, filterByEra, showEraCards } from "./era";
import { usePins } from "../stores/pins";
import { setFollow, useSocial } from "../stores/social";
import "../shell/social.css";
import { player, usePlayer } from "../stores/player";
import { nav, useRouter } from "../stores/router";
import { AlbumCard, ArtistCard, useContextState } from "./cards";
import { usePageBar, useTitleStep } from "./detail";
import { AboutCard, useArtistInfo } from "./ArtistAbout";
import { missingFor, useMissingReleases } from "./lidarrShared";

/** Cards for releases the library lacks (and their request dialog), loaded once an artist has some. */
const MissingReleaseCard = lazy(() => import("./LidarrRequest").then((m) => ({ default: m.MissingReleaseCard })));
import {
  POPULAR_FIRST,
  POPULAR_MORE,
  POPULAR_WAIT_MS,
  aboutText,
  artistSections,
  bannerPath,
  discographyChips,
  discographyItems,
  headerVariant,
  listenersLine,
  photoPath,
  playsLine,
  type DiscographyFilter,
} from "./artist";

/** Page state (see more, chip, expanded rows) starts over for each artist. `era` filters it to one era. */
export function ArtistPage({ id, era }: { id: string; era?: string }) {
  return <ArtistView key={id} id={id} era={era} />;
}

function ArtistView({ id, era: eraName }: { id: string; era?: string }) {
  const { data: loaded, error } = useLoad(() => backend.artistPage(id), [id]);
  // Similar artists and the global Popular list arrive in the background
  // after the page opened; the page is asked again when they do.
  const [fresh, setFresh] = useState<ArtistPageData | null>(null);
  // While the global Popular list is on its way, Popular shows placeholder
  // rows; plays order shows only if the fetch fails or takes too long.
  const [gaveUp, setGaveUp] = useState(false);
  useEffect(() => {
    let alive = true;
    const reload = (e: { id: string }) => {
      if (e.id !== id) return;
      backend.artistPage(id).then(
        (p) => alive && setFresh(p),
        () => {},
      );
    };
    const done = (e: { id: string }) => {
      if (e.id !== id) return;
      setGaveUp(true);
      reload(e);
    };
    const timer = setTimeout(() => alive && setGaveUp(true), POPULAR_WAIT_MS);
    const uns = [
      backend.listen("artist://changed", reload),
      backend.listen("artistinfo://changed", reload),
      backend.listen("artistinfo://popular", done),
      backend.listen("archive://changed", () => reload({ id })),
    ];
    return () => {
      alive = false;
      clearTimeout(timer);
      uns.forEach((un) => un.then((f) => f()));
    };
  }, [id]);
  const page = fresh ?? loaded;
  const eraCardsOn = useSettings((s) => s.settings?.showEraCards);
  const era = (eraName && page?.eras?.find((e) => e.name === eraName)) || null;
  // The era filter covers Popular, every section and Appears on.
  const data = page && era ? filterByEra(page, era) : page;
  const popularWaiting = !!data?.popularPending && !gaveUp;
  const credited = isCreditId(id);
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

  // A library artist keeps About without a bio; a credited name needs something to say.
  const sections = data
    ? artistSections({
        ...data,
        about,
        listeners: info?.listeners,
        photo: credited ? null : photoPath(data.artist, info),
        plays: credited ? 0 : data.plays,
        eras: showEraCards(data.eras, eraCardsOn) ? data.eras : undefined,
      })
    : [];
  const text = artist ? (
    <>
      <span className="detail-header__type">Artist</span>
      <h1 className="detail-header__title" data-step={step}>
        {artist.title}
      </h1>
      <div className="detail-header__meta">
        <span>{playsLine(data!.plays)}</span>
        {info?.listeners != null && <span className="artist-listeners">{listenersLine(info.listeners, info.listenersFrom)}</span>}
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
      {/* A narrow main column swaps the banner for the compact round-photo header. */}
      {variant === "banner" && artist && !compact ? (
        <header ref={header} className="artist-banner" data-variant="banner">
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
            {artist ? <Art path={photoPath(artist, info)} size={232} round kind="artist" /> : <Skeleton width={232} height={232} radius="circle" />}
          </div>
          <div ref={column} className="detail-header__text">
            {text}
          </div>
        </header>
      )}

      <div className="detail-body">
        <ActionRow id={id} data={data} playing={isThisPlaying} onPlay={play} credited={credited} />
        <StatsStrip kind="artist" id={id} />
        {era && <EraFilterChip id={id} era={era} />}

        {!data && (
          <>
            <PopularSkeleton />
            <Shelf title="Discography" loading />
          </>
        )}

        {data &&
          sections.map((s) => {
            switch (s) {
              case "popular":
                return popularWaiting ? <PopularSkeleton key={s} /> : <PopularSection key={s} id={id} data={data} />;
              case "eras":
                return <EraShelf key={s} id={id} eras={data.eras ?? []} selected={era?.name ?? null} />;
              case "discography":
                return <Discography key={s} id={id} sections={data.sections} artist={credited ? undefined : data.artist.title} />;
              case "appearsOn":
                return <AppearsOn key={s} id={id} albums={data.appearsOn} />;
              case "about":
                return <AboutCard key={s} artist={data.artist} about={about} plays={data.plays} info={info} />;
              case "fansAlsoLike":
                return <FansAlsoLike key={s} id={id} artists={data.similar} />;
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

function ActionRow({
  id,
  data,
  playing,
  onPlay,
  credited,
}: {
  id: string;
  data: ArtistPageData | undefined;
  playing: boolean;
  onPlay: () => void;
  /** A credited artist has no Plex entry, so nothing that needs one (sharing) shows. */
  credited: boolean;
}) {
  const artist = data?.artist;
  const shuffle = usePlayer((s) => s.state?.shuffledContexts?.includes(id) ?? false);
  const pinned = usePins((s) => s.isPinned("artist", id));
  const togglePin = () =>
    artist && usePins.getState().toggle({ kind: "artist", id: artist.id, title: artist.title, subtitle: "Artist", thumb: artist.thumb });
  const name = artist?.title ?? "artist";
  const following = useSocial((s) => s.followIds.has(id));
  const toggleFollow = () => artist && void setFollow({ id, title: artist.title, thumb: artist.thumb }, !following);
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
      <button
        type="button"
        className="follow-pill"
        aria-pressed={following}
        aria-label={following ? `Following ${name}` : `Follow ${name}`}
        disabled={!artist}
        onClick={toggleFollow}
      >
        {following ? "Following" : "Follow"}
      </button>
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
              { label: following ? "Unfollow" : "Follow", onSelect: toggleFollow },
              { label: "Add popular songs to queue", onSelect: () => void player.addToQueue(data!.popular, artist.thumb) },
              { label: "Go to artist radio", onSelect: () => nav.go({ name: "radio", seed: `artist:${artist.id}` }) },
              dontPlayArtistItem({ id: artist.id, title: artist.title, thumb: artist.thumb }),
              bookmarkPageItem("artist", artist.id, artist.title, { thumb: artist.thumb }),
              yourStatsItem("artist", artist.id, artist.title),
              ...(credited ? [] : [shareItem("artist", artist.id, artist.thumb)]),
              { label: pinned ? "Unpin from sidebar" : "Pin to sidebar", onSelect: togglePin },
              ...(credited ? [] : [plexEditItem({ id: artist.id, kind: "artist", title: artist.title, thumb: artist.thumb })]),
            ],
            e.currentTarget,
          )
        }
      />
    </div>
  );
}

// ---------------------------------------------------------------- eras

/** "Era: Glass Season" with a way out, and the era's own page. */
function EraFilterChip({ id, era }: { id: string; era: Era }) {
  return (
    <div className="era-filter" role="group" aria-label="Era filter">
      <button
        type="button"
        className="chip is-selected era-filter__chip"
        aria-label={`Era: ${era.name}. Show everything`}
        onClick={() => useRouter.getState().replace({ name: "artist", id })}
      >
        <span>Era: {era.name}</span>
        <X size={14} aria-hidden="true" />
      </button>
      <button type="button" className="link era-filter__page" onClick={() => nav.go({ name: "era", id, era: era.name })}>
        Open the era page
      </button>
    </div>
  );
}

/** One card per era, oldest first: the most played album's cover, the dates and the song count. */
function EraShelf({ id, eras, selected }: { id: string; eras: Era[]; selected: string | null }) {
  return (
    <div className="era-shelf">
      <Shelf title="Eras">
        {eras.map((e) => {
          const range = eraRange(e);
          const songs = `${e.trackCount} ${e.trackCount === 1 ? "song" : "songs"}`;
          return (
            <ShelfItem key={e.name}>
              <div className={`era-card ${selected === e.name ? "is-selected" : ""}`} aria-current={selected === e.name || undefined}>
                <Card
                  kind="album"
                  title={e.name}
                  subtitle={range ? `${range} · ${songs}` : songs}
                  thumb={e.cover}
                  onOpen={() =>
                    selected === e.name
                      ? useRouter.getState().replace({ name: "artist", id })
                      : nav.go({ name: "artist", id, era: e.name })
                  }
                  onPlay={() => void player.playTracks(e.trackIds)}
                  onContextMenu={(ev) => {
                    ev.preventDefault();
                    openContextMenu(
                      `Options for ${e.name}`,
                      [
                        { label: "Show only this era", onSelect: () => nav.go({ name: "artist", id, era: e.name }) },
                        { label: "Open the era page", onSelect: () => nav.go({ name: "era", id, era: e.name }) },
                        { label: "Play", onSelect: () => void player.playTracks(e.trackIds) },
                        { label: "Add to queue", onSelect: () => void player.addToQueue(e.trackIds.map((t) => ({ id: t, thumb: e.cover ?? null }))) },
                      ],
                      ev,
                    );
                  }}
                />
              </div>
            </ShelfItem>
          );
        })}
      </Shelf>
    </div>
  );
}

// ---------------------------------------------------------------- sections

/** Popular's placeholder rows, while the page or the global list loads. */
function PopularSkeleton() {
  return (
    <section className="detail-section" aria-busy="true">
      <h2 className="section-title">Popular</h2>
      <TrackList label="Popular tracks" header={false}>
        {Array.from({ length: POPULAR_FIRST }, (_, i) => (
          <TrackRowSkeleton key={i} showArt />
        ))}
      </TrackList>
    </section>
  );
}

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

/** The discography shelf with its type chips; Show all opens the full discography page at the chosen type. */
function Discography({ id, sections, artist }: { id: string; sections: ArtistPageSection[]; artist?: string }) {
  const [filter, setFilter] = useState<DiscographyFilter>("popular");
  const missing = useMissingReleases(id, artist);
  const chips = discographyChips(sections);
  const chipRow = (
    <FilterChips label="Release type" items={chips} value={filter} onChange={setFilter} className="discography__chips" scroll />
  );
  return (
    <div className="discography">
      <Shelf
        title="Discography"
        onShowAll={() => nav.go(filter === "popular" ? { name: "discography", id } : { name: "discography", id, type: filter })}
        filters={chipRow}
      >
        {discographyItems(sections, filter).map(({ album, latest }) => (
          <ShelfItem key={album.id}>
            <AlbumCard album={album} subtitle="year" note={latest ? "Latest release" : undefined} />
          </ShelfItem>
        ))}
        {artist &&
          missingFor(missing.items, filter).map((item) => (
            <ShelfItem key={`missing-${item.releaseGroupId}`}>
              <Suspense fallback={<CardSkeleton />}>
                <MissingReleaseCard item={item} artist={artist} artistId={id} onChanged={missing.reload} />
              </Suspense>
            </ShelfItem>
          ))}
      </Shelf>
    </div>
  );
}

function AppearsOn({ id, albums }: { id: string; albums: Album[] }) {
  return (
    <Shelf title="Appears on" onShowAll={() => nav.go({ name: "artistShelf", id, shelf: "appearsOn" })}>
      {albums.map((album) => (
        <ShelfItem key={album.id}>
          <AlbumCard album={album} subtitle="artist" />
        </ShelfItem>
      ))}
    </Shelf>
  );
}

function FansAlsoLike({ id, artists }: { id: string; artists: Artist[] }) {
  return (
    <Shelf title="Fans also like" skeletonKind="artist" onShowAll={() => nav.go({ name: "artistShelf", id, shelf: "fansAlsoLike" })}>
      {artists.map((a) => (
        <ShelfItem key={a.id}>
          <ArtistCard artist={a} />
        </ShelfItem>
      ))}
    </Shelf>
  );
}
