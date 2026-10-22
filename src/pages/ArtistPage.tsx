import { Download, Ellipsis, Pause, Pin, PinOff, Play, Shuffle, X } from "lucide-react";
import { PageMissing } from "../components/PageMissing";
import { useEffect, useId, useState, type RefObject } from "react";
import { Art } from "../components/Art";
import { Button, IconButton } from "../components/Button";
import { Card } from "../components/Card";
import { openContextMenu } from "../components/ContextMenu";
import { sendToDeviceItem } from "../components/DeviceMenu";
import { shareItem } from "../components/ShareMenu";
import { plexEditItem } from "../components/PlexEditMenu";
import { tagArtistItem } from "../components/TagEditMenu";
import { bookmarkPageItem, dontPlayArtistItem } from "../stores/curate";
import { yourStatsItem } from "../stores/stats";
import { useItemStats } from "../components/StatsStrip";
import { Shelf, ShelfItem } from "../components/Shelf";
import { Skeleton } from "../components/Skeleton";
import { TrackList, TrackRow, TrackRowSkeleton } from "../components/TrackRow";
import { artUrl, backend } from "../lib/backend";
import { toast } from "../stores/toasts";
import { isCreditId } from "../lib/credits";
import { useLoad, useReload } from "../lib/useLoad";
import { useArtTint } from "../lib/useTint";
import type { Album, Artist, ArtistPage as ArtistPageData, Era, PopularElsewhere, Track } from "../lib/types";
import { tapToPlay } from "../stores/taps";
import { useTapPlays } from "./SearchCatalogue";
import "./catalogue.css";
import { useSettings } from "../stores/settings";
import { eraRange, filterByEra, showEraCards } from "./era";
import { usePins } from "../stores/pins";
import { setFollow, setJustShow, useJustShow, useSocial } from "../stores/social";
import "../shell/social.css";
import { player, usePlayer } from "../stores/player";
import { nav, useRouter } from "../stores/router";
import { AlbumCard, ArtistCard, useContextState } from "./cards";
import { usePageBar, useTitleStep } from "./detail";
import { AboutCard, useArtistInfo } from "./ArtistAbout";
import { lidarrNote, useGetSpots, useLidarrReady, useLidarrStatus } from "./lidarrShared";
import { useIsGuest } from "../stores/asks";
import { GetAlbumsDialog } from "./GetAlbumsDialog";
import { CatalogueArtistCard, NOT_IN_LIBRARY, dismissArtist, undismissArtist, useCatalogueFans } from "./catalogue";
import type { CatArtist, CatalogueArtistPage } from "../lib/types";
import { DiscographyShelf } from "./DiscographyShelf";
import { useArtistDiscography } from "./discographySources";
import {
  POPULAR_FIRST,
  POPULAR_MORE,
  POPULAR_WAIT_MS,
  aboutText,
  artistSections,
  bannerPath,
  catalogueEntries,
  headerVariant,
  listenersLine,
  photoPath,
  playsLine,
  type DiscographyFilter,
} from "./artist";

/** The full discography's route, at a chip's release type. */
function discographyRoute(id: string, filter: DiscographyFilter, catalogue?: boolean) {
  return { name: "discography" as const, id, ...(filter === "popular" ? {} : { type: filter }), ...(catalogue ? { catalogue: true } : {}) };
}

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
  // Releases you can get, and for an artist the server has no albums (or
  // photo) for, the catalogue's photo.
  const disco = useArtistDiscography(id, artist?.title, data?.sections);
  const lidarrReady = useGetSpots();
  const photo = artist ? (photoPath(artist, info) ?? disco.photo) : null;
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
        catalogue: disco.catalogue,
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
            {artist ? <Art path={photo} size={232} round kind="artist" monogram={credited ? artist.title : undefined} /> : <Skeleton width={232} height={232} radius="circle" />}
          </div>
          <div ref={column} className="detail-header__text">
            {text}
          </div>
        </header>
      )}

      <div className="detail-body">
        <ActionRow id={id} data={data} playing={isThisPlaying} onPlay={play} credited={credited} />
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
                return (
                  <DiscographyShelf
                    key={s}
                    entries={disco.entries}
                    lidarrReady={lidarrReady}
                    artistLibraryId={credited ? null : id}
                    onChanged={disco.reload}
                    onShowAll={(f) => nav.go(discographyRoute(id, f))}
                  />
                );
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
  // Following gets their new albums; this one can just show them instead.
  const getsNew = useSettings((s) => s.settings?.followGet !== false);
  const lidarrReady = useLidarrReady();
  const justShow = useJustShow(id);
  const autoItem = following && getsNew && lidarrReady ? [{ label: "Just show new albums", checked: justShow, onSelect: () => void setJustShow(id, !justShow) }] : [];
  // Get albums: the artist's MusicBrainz id is looked up when it's picked.
  const [getMbid, setGetMbid] = useState<string | null>(null);
  const getAlbums = async () => {
    if (!artist) return;
    const f = await backend.featuredArtist(id, artist.title).catch(() => null);
    if (f?.mbid) setGetMbid(f.mbid);
    else toast.info(`Couldn't find ${artist.title} on MusicBrainz to get their albums`);
  };
  const getItem = lidarrReady ? [{ label: "Get albums", onSelect: () => void getAlbums() }] : [];
  return (
    <div className="action-row">
      {artist && getMbid && <GetAlbumsDialog mbid={getMbid} name={artist.title} open onClose={() => setGetMbid(null)} />}
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
              ...getItem,
              ...autoItem,
              { label: "Add popular songs to queue", onSelect: () => void player.addToQueue(data!.popular, artist.thumb) },
              { label: "Go to artist radio", onSelect: () => nav.go({ name: "radio", seed: `artist:${artist.id}` }) },
              ...shuffleAlbumsItem(data!),
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

/** "Shuffle albums" for an artist with two albums or more. */
function shuffleAlbumsItem(data: ArtistPageData) {
  const ids = [...new Set(data.sections.flatMap((s) => s.albums.map((a) => a.id)))];
  return ids.length > 1 ? [{ label: "Shuffle albums", onSelect: () => void player.shuffleAlbums(ids) }] : [];
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

type PopularItem = { own: Track } | { get: PopularElsewhere };

/**
 * Popular as the world plays it: the library's songs, and with tap to get
 * on, the artist's top songs you don't have, each where it ranks.
 */
export function popularItems(own: Track[], elsewhere: PopularElsewhere[] | undefined, tap: boolean): PopularItem[] {
  const out: PopularItem[] = [];
  const more = tap ? (elsewhere ?? []) : [];
  let j = 0;
  own.forEach((t, i) => {
    while (j < more.length && more[j].after <= i) out.push({ get: more[j++] });
    out.push({ own: t });
  });
  while (j < more.length) out.push({ get: more[j++] });
  return out;
}

function PopularSection({ id, data }: { id: string; data: ArtistPageData }) {
  const [more, setMore] = useState(false);
  const nowTrackId = usePlayer((s) => s.state?.track?.id);
  const status = usePlayer((s) => s.state?.status);
  const headingId = useId();
  const tap = useTapPlays(useGetSpots());
  const items = popularItems(data.popular, data.popularElsewhere, tap);
  const shown = items.slice(0, more ? POPULAR_MORE : POPULAR_FIRST);
  return (
    <section className="detail-section artist-popular" aria-labelledby={headingId}>
      <h2 id={headingId} className="section-title">
        Popular
      </h2>
      <TrackList label="Popular songs" header={false}>
        {shown.map((item, i) => {
          if ("get" in item) return <PopularGetRow key={`get-${item.get.title}`} song={item.get} artist={data.artist.title} index={i + 1} />;
          const t = item.own;
          return (
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
          );
        })}
      </TrackList>
      {items.length > POPULAR_FIRST && (
        <button type="button" className="see-more" onClick={() => setMore((v) => !v)} aria-expanded={more}>
          {more ? "Show less" : "See more"}
        </button>
      )}
    </section>
  );
}

/** A popular song the library doesn't have: Get where the length would be, and a tap gets and plays it. */
function PopularGetRow({ song, artist, index }: { song: PopularElsewhere; artist: string; index: number }) {
  const play = () => void tapToPlay({ title: song.title, artist, album: song.release });
  return (
    <div
      role="row"
      tabIndex={0}
      aria-label={`${song.title} by ${artist}, not in your library, a tap gets it and plays it`}
      className="track-row is-missing track-row--get"
      onDoubleClick={play}
      onKeyDown={(e) => {
        if (e.key === "Enter") play();
      }}
    >
      <div className="track-row__num" role="gridcell">
        <span className="track-row__index tabular">{index}</span>
        <button type="button" tabIndex={-1} className="track-row__play" aria-label={`Get and play ${song.title}`} onClick={play}>
          <Play size={14} fill="currentColor" aria-hidden="true" />
        </button>
      </div>
      <div className="track-row__main" role="gridcell">
        <span className="track-row__art">
          <Art path={null} sleeve={{ title: song.release ?? song.title, artist }} size={40} />
        </span>
        <div className="track-row__text">
          <button type="button" tabIndex={-1} className="track-row__title track-row__get-title truncate" onClick={play}>
            {song.title}
          </button>
        </div>
      </div>
      <div className="track-row__album truncate" role="gridcell">
        {song.release ?? ""}
      </div>
      <div className="track-row__badges" role="gridcell" />
      <div className="track-row__end" role="gridcell">
        <span className="sr-get-mark" aria-hidden="true">
          Get
        </span>
      </div>
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
  const lidarr = useLidarrStatus();
  const lidarrReady = !!lidarr?.ready;
  const lidarrSays = lidarrNote(lidarr, "Set up Lidarr in Settings to get albums, or buy them from a store.");
  const spots = useGetSpots();
  const guest = useIsGuest();
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
          {!lidarrReady && lidarrSays && !guest && data && <span className="cat-actions__note">{lidarrSays}</span>}
        </div>
        {artist && getting && <GetAlbumsDialog mbid={artist.mbid} name={artist.name} open onClose={() => setGetting(false)} />}
        {!data && <Shelf title="Discography" loading />}
        {data && data.sections.length === 0 && <p className="cat-bio__none">MusicBrainz has no official albums for this artist yet.</p>}
        {data && data.sections.length > 0 && (
          <DiscographyShelf
            entries={catalogueEntries(data.sections, data.popular)}
            lidarrReady={spots}
            onChanged={reload}
            onShowAll={(f) => nav.go(discographyRoute(data.artist.mbid, f, true))}
          />
        )}
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
