import { Download, Ellipsis, Pause, Pin, PinOff, Play, Shuffle, X } from "lucide-react";
import { PageMissing } from "../components/PageMissing";
import { Suspense, lazy, useEffect, useId, useState, type RefObject } from "react";
import { Art } from "../components/Art";
import { Button, IconButton } from "../components/Button";
import { Card, CardSkeleton } from "../components/Card";
import { openContextMenu } from "../components/ContextMenu";
import { sendToDeviceItem } from "../components/DeviceMenu";
import { shareItem } from "../components/ShareMenu";
import { plexEditItem } from "../components/PlexEditMenu";
import { tagArtistItem } from "../components/TagEditMenu";
import { bookmarkPageItem, dontPlayArtistItem } from "../stores/curate";
import { yourStatsItem } from "../stores/stats";
import { StatsStrip, useItemStats } from "../components/StatsStrip";
import { Shelf, ShelfItem } from "../components/Shelf";
import { Skeleton } from "../components/Skeleton";
import { FilterChips } from "../components/Tabs";
import { TrackList, TrackRow, TrackRowSkeleton } from "../components/TrackRow";
import { artUrl, backend } from "../lib/backend";
import { isCreditId } from "../lib/credits";
import { useLoad, useReload } from "../lib/useLoad";
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
import { missingFor, useLidarrReady, useMissingReleases } from "./lidarrShared";
import { useAutoOn, useAutoRequest } from "../stores/radar";
import { AutoRequestDialog } from "./AutoRequestDialog";
import { GetAlbumsDialog } from "./GetAlbumsDialog";
import { CatalogueAlbumCard, CatalogueArtistCard, NOT_IN_LIBRARY, SECTION_TITLE, dismissArtist, undismissArtist, useCatalogueFans } from "./catalogue";
import type { CatArtist, CatalogueArtistPage } from "../lib/types";

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

/**
 * Page state (see more, chip, expanded rows) starts over for each artist. `era` filters it to one era.
 * `catalogue`: `id` is a MusicBrainz artist id for an artist that isn't in the library.
 */
export function ArtistPage({ id, era, catalogue }: { id: string; era?: string; catalogue?: boolean }) {
  return catalogue ? <CatalogueArtistView key={`mb:${id}`} mbid={id} /> : <ArtistView key={id} id={id} era={era} />;
}

function ArtistView({ id, era: eraName }: { id: string; era?: string }) {
  const { data: loaded, error, retry } = useLoad(() => backend.artistPage(id), [id]);
  // The header's play count matches the stats strip: imported history counts
  // too, so an artist played elsewhere doesn't read "0 plays".
  const itemStats = useItemStats("artist", id);
  const allPlays = itemStats && itemStats !== "error" ? itemStats.plays : 0;
  const headerPlays = { plays: allPlays, history: allPlays > (loaded?.plays ?? 0) };
  // Similar artists and the global Popular list arrive in the background
  // after the page opened; the page is asked again when they do.
  const { data: page, reload: reloadPage } = useReload(loaded, () => backend.artistPage(id));
  // While the global Popular list is on its way, Popular shows placeholder
  // rows; plays order shows only if the fetch fails or takes too long.
  const [gaveUp, setGaveUp] = useState(false);
  useEffect(() => {
    let alive = true;
    const reload = (e: { id: string }) => {
      if (e.id === id) reloadPage();
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
  }, [id, reloadPage]);
  const eraCardsOn = useSettings((s) => s.settings?.showEraCards);
  const era = (eraName && page?.eras?.find((e) => e.name === eraName)) || null;
  // The era filter covers Popular, every section and Appears on.
  const data = page && era ? filterByEra(page, era) : page;
  const popularWaiting = !!data?.popularPending && !gaveUp;
  const credited = isCreditId(id);
  const artist = data?.artist;
  const info = useArtistInfo(id);
  const fans = useCatalogueFans(credited ? null : id, artist?.title);
  const banner = artist ? bannerPath(artist, info) : null;
  const variant = artist ? headerVariant(artist, banner) : "photo";
  const about = data ? aboutText(data.about, info) : null;
  useArtTint(null, artist ? (artist.art ?? artist.thumb) : undefined);
  const { isContext: isThis, playing: isThisPlaying } = useContextState("artist", id);
  const play = () => (isThis ? player.toggle() : player.playArtist(id));
  usePageBar(artist?.title, isThisPlaying, play);
  const { header, column, step, compact } = useTitleStep(artist?.title);

  if (error && !data) return <PageMissing what="artist" error={error} onRetry={retry} />;

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
  // Artists outside the library alone still make a "Fans also like" shelf.
  if (data && fans.length && !sections.includes("fansAlsoLike")) {
    const at = sections.indexOf("appearsOn");
    sections.splice(at < 0 ? sections.length : at, 0, "fansAlsoLike");
  }
  const text = artist ? (
    <>
      <span className="detail-header__type">Artist</span>
      <h1 className="detail-header__title" data-step={step}>
        {artist.title}
      </h1>
      <div className="detail-header__meta">
        <span>{headerPlays.history ? playsLine(headerPlays.plays, true) : playsLine(data!.plays)}</span>
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
                return <FansAlsoLike key={s} id={id} artists={data.similar} more={fans} />;
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
  // Auto-request: only with the release radar on and Lidarr ready.
  const radarOn = useSettings((s) => s.settings?.releaseRadar !== false);
  const lidarrReady = useLidarrReady();
  const autoOn = useAutoOn(id);
  const [askAuto, setAskAuto] = useState(false);
  const toggleAuto = () => {
    if (!artist) return;
    if (autoOn) void useAutoRequest.getState().set(id, artist.title, false);
    else setAskAuto(true);
  };
  const autoItem = radarOn && lidarrReady ? [{ label: "Auto-request new albums", checked: autoOn, onSelect: toggleAuto }] : [];
  return (
    <div className="action-row">
      {artist && askAuto && <AutoRequestDialog id={id} name={artist.title} open onClose={() => setAskAuto(false)} />}
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
              ...autoItem,
              { label: "Add popular songs to queue", onSelect: () => void player.addToQueue(data!.popular, artist.thumb) },
              { label: "Go to artist radio", onSelect: () => nav.go({ name: "radio", seed: `artist:${artist.id}` }) },
              dontPlayArtistItem({ id: artist.id, title: artist.title, thumb: artist.thumb }),
              bookmarkPageItem("artist", artist.id, artist.title, { thumb: artist.thumb }),
              yourStatsItem("artist", artist.id, artist.title),
              ...(credited ? [] : [shareItem("artist", artist.id, artist.thumb)]),
              ...(credited ? [] : [sendToDeviceItem({ kind: "artist", id: artist.id, title: artist.title })]),
              { label: pinned ? "Unpin from sidebar" : "Pin to sidebar", onSelect: togglePin },
              ...(credited ? [] : [plexEditItem({ id: artist.id, kind: "artist", title: artist.title, thumb: artist.thumb })]),
              ...(credited ? [] : tagArtistItem(artist)),
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
      <TrackList label="Popular songs" header={false}>
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
      <TrackList label="Popular songs" header={false}>
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

/** Library artists first, then (dimmed) artists that aren't in the library. */
function FansAlsoLike({ id, artists, more = [] }: { id: string; artists: Artist[]; more?: CatArtist[] }) {
  return (
    <Shelf title="Fans also like" skeletonKind="artist" onShowAll={() => nav.go({ name: "artistShelf", id, shelf: "fansAlsoLike" })}>
      {artists.map((a) => (
        <ShelfItem key={a.id}>
          <ArtistCard artist={a} />
        </ShelfItem>
      ))}
      {more.map((a) => (
        <ShelfItem key={`mb-${a.mbid}`}>
          <CatalogueArtistCard artist={a} />
        </ShelfItem>
      ))}
    </Shelf>
  );
}

// ---------------------------------------------------------------- an artist that isn't in the library

/**
 * The same page for an artist the library doesn't have: header photo and
 * bio from the artist info sources, their popular albums, the discography
 * of official releases by type (each with Get), "Get albums", and similar
 * artists. An artist the library turns out to have opens their own page
 * instead.
 */
function CatalogueArtistView({ mbid }: { mbid: string }) {
  const [version, setVersion] = useState(0);
  const { data, error, retry } = useLoad(() => backend.catalogueArtist(mbid), [mbid, version]);
  const lidarrReady = useLidarrReady();
  const [getting, setGetting] = useState(false);
  const dismissed = useSettings((s) => s.settings?.dismissedArtists?.some((d) => d.mbid === mbid)) ?? false;
  const { header, column, step, compact } = useTitleStep(data?.artist.name);
  const libraryId = data?.artist.libraryId;
  useEffect(() => {
    if (libraryId) useRouter.getState().replace({ name: "artist", id: libraryId });
  }, [libraryId]);
  useArtTint(null, data?.artist.photo ?? undefined);

  if (error && !data) return <PageMissing what="artist" error={error} missing="Couldn't find this artist" onRetry={retry} />;
  const artist = data?.artist;
  const meta = data ? [data.artist.disambiguation, data.area, data.lifeSpan].filter(Boolean).join(" · ") : "";
  const reload = () => setVersion((v) => v + 1);
  const text = artist ? (
    <>
      <span className="detail-header__type">
        Artist <span className="cat-tag">{NOT_IN_LIBRARY}</span>
      </span>
      <h1 className="detail-header__title" data-step={step}>
        {artist.name}
      </h1>
      <div className="detail-header__meta">
        {meta && <span>{meta}</span>}
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
    <div className="detail artist-page cat-page">
      {data?.banner && !compact ? (
        <header ref={header} className="artist-banner" data-variant="banner">
          <BannerImage path={data.banner} box={header} />
          <div ref={column} className="detail-header__text artist-banner__text">
            {text}
          </div>
        </header>
      ) : (
        <header ref={header} className={`detail-header detail-header--artist ${compact ? "detail-header--compact" : ""}`} data-variant="photo">
          <div className="detail-header__art">
            {artist ? <Art path={artist.photo} size={232} round kind="artist" monogram={artist.name} /> : <Skeleton width={232} height={232} radius="circle" />}
          </div>
          <div ref={column} className="detail-header__text">
            {text}
          </div>
        </header>
      )}
      <div className="detail-body">
        <div className="action-row cat-actions">
          {lidarrReady && artist && (
            <Button variant="primary" icon={<Download size={16} />} onClick={() => setGetting(true)}>
              Get albums
            </Button>
          )}
          <button
            type="button"
            className="follow-pill"
            aria-pressed={dismissed}
            disabled={!artist}
            onClick={() => artist && (dismissed ? undismissArtist(artist.mbid) : dismissArtist({ mbid: artist.mbid, name: artist.name }))}
          >
            {dismissed ? "Hidden from suggestions" : "Not interested"}
          </button>
          <IconButton
            label={`More options for ${artist?.name ?? "artist"}`}
            size="lg"
            icon={<Ellipsis size={22} />}
            disabled={!artist}
            onClick={(e) =>
              artist &&
              openContextMenu(
                `Options for ${artist.name}`,
                [
                  dismissed
                    ? { label: "Show in suggestions again", onSelect: () => undismissArtist(artist.mbid) }
                    : { label: "Not interested", onSelect: () => dismissArtist({ mbid: artist.mbid, name: artist.name }) },
                  {
                    label: "Open on MusicBrainz",
                    onSelect: () => void backend.openInfoLink(`https://musicbrainz.org/artist/${artist.mbid}`).catch(() => {}),
                  },
                ],
                e.currentTarget,
              )
            }
          />
          {!lidarrReady && data && <span className="cat-actions__note">Set up Lidarr in Settings to get albums, or buy them from a store.</span>}
        </div>
        {artist && getting && <GetAlbumsDialog mbid={artist.mbid} name={artist.name} open onClose={() => setGetting(false)} />}
        {!data && <Shelf title="Discography" loading />}
        {data?.popular && data.popular.length > 0 && (
          <Shelf title="Popular">
            {data.popular.map((a) => (
              <ShelfItem key={`p-${a.releaseGroupId}`}>
                <CatalogueAlbumCard album={a} lidarrReady={lidarrReady} onChanged={reload} />
              </ShelfItem>
            ))}
          </Shelf>
        )}
        {data && data.sections.length === 0 && <p className="cat-bio__none">MusicBrainz has no official albums for this artist yet.</p>}
        {data?.sections.map((s) => (
          <Shelf key={s.kind} title={SECTION_TITLE[s.kind] ?? "Albums"}>
            {s.albums.map((a) => (
              <ShelfItem key={a.releaseGroupId}>
                <CatalogueAlbumCard album={a} lidarrReady={lidarrReady} onChanged={reload} />
              </ShelfItem>
            ))}
          </Shelf>
        ))}
        {data && <CatalogueAbout page={data} />}
        {data && data.similar.length > 0 && (
          <Shelf title="Fans also like" skeletonKind="artist">
            {data.similar.map((a) => (
              <ShelfItem key={a.mbid}>
                <CatalogueArtistCard artist={a} />
              </ShelfItem>
            ))}
          </Shelf>
        )}
      </div>
    </div>
  );
}

function CatalogueAbout({ page }: { page: CatalogueArtistPage }) {
  const headingId = useId();
  return (
    <section className="detail-section" aria-labelledby={headingId}>
      <h2 id={headingId} className="section-title">
        About
      </h2>
      {page.bio ? (
        <>
          <p className="cat-bio">{page.bio}</p>
          {page.bioUrl && (
            <p className="cat-bio__credit">
              <button type="button" className="link" onClick={() => void backend.openInfoLink(page.bioUrl!).catch(() => {})}>
                From {page.bioCredit ?? "Wikipedia"}
              </button>
            </p>
          )}
        </>
      ) : (
        <p className="cat-bio__none">There's no biography for {page.artist.name} yet.</p>
      )}
      {page.artist.photoCredit && <p className="cat-bio__credit">Photo from {page.artist.photoCredit}</p>}
    </section>
  );
}
