/**
 * An artist on the phone: a large photo with the name over it, which the
 * page slides up over as it scrolls; what of theirs is in your library;
 * follow, radio, shuffle and play; top tracks, everyone's or yours;
 * their eras; the discography with the records you can get (Get, through
 * Lidarr); Appears on and Fans also like, each with See all.
 */
import { Radio } from "lucide-react";
import { useEffect, useRef, useState, type ReactNode } from "react";
import { Art } from "../../components/Art";
import { backend } from "../../lib/backend";
import { isCreditId } from "../../lib/credits";
import { plural } from "../../lib/format";
import type { Settings } from "../../lib/types";
import { useLoad, useReload } from "../../lib/useLoad";
import { photoPath, POPULAR_WAIT_MS, type DiscographyFilter } from "../../pages/artist";
import { useCatalogueFans } from "../../pages/catalogue";
import { eraRange, showEraCards } from "../../pages/era";
import { useArtistDiscography } from "../../pages/discographySources";
import { useGetSpots } from "../../pages/lidarrShared";
import { player, usePlayer } from "../../stores/player";
import { nav, type Route } from "../../stores/router";
import { updateSettings, useSettings } from "../../stores/settings";
import { setFollow, useSocial } from "../../stores/social";
import { BackBar, Carousel, PageFailed, PageLoading, ShelfCard, SongRow } from "./common";
import { DiscoShelf } from "./DiscoShelf";
import { CataloguePhoto, Pills, PhoneShuffleToggle, PlayPill } from "./kit";

/** Top tracks shown at first. */
export const TOP_TRACKS = 5;
/** Top tracks after See all. */
export const TOP_TRACKS_ALL = 20;

type TopRank = "all" | "yours";

const RANKS = [
  { id: "all", label: "All time" },
  { id: "yours", label: "Yours" },
] as const;

/** Whether top tracks rank by everyone's plays: the setting says so and a source for it is on. */
export function topRank(s: Settings | null): TopRank {
  return s?.popularSource === "global" && (s.infoListenbrainz || s.infoLastfm) ? "all" : "yours";
}

/** The settings change a pill makes; All time turns ListenBrainz on when neither source is. */
export function rankPatch(rank: TopRank, s: Settings | null): Partial<Settings> {
  if (rank === "yours") return { popularSource: "myPlays" };
  return s?.infoListenbrainz || s?.infoLastfm ? { popularSource: "global" } : { popularSource: "global", infoListenbrainz: true };
}

/** "In your library: 4 albums, 52 songs". */
export function inLibraryLine(albums: number, tracks: number): string {
  // A count the server didn't give stays out rather than saying "0 songs".
  return tracks > 0 ? `In your library: ${plural(albums, "album")}, ${plural(tracks, "song")}` : `In your library: ${plural(albums, "album")}`;
}

/** The discography's See all: the full page, on the type that was picked. */
export function discographyRoute(id: string, f: DiscographyFilter): Route {
  return { name: "discography", id, ...(f === "popular" ? {} : { type: f }) };
}

/** A sideways shelf with its title and See all. */
function Shelf({ title, onAll, children }: { title: string; onAll?: () => void; children: ReactNode }) {
  return (
    <section className="ph-shelf ph-shelf--all" aria-label={title}>
      <div className="ph-disco__head">
        <h2 className="ph-shelf__title">{title}</h2>
        {onAll && (
          <button type="button" className="ph-link-btn" onClick={onAll} aria-label={`See all: ${title}`}>
            See all
          </button>
        )}
      </div>
      <div className="ph-shelf__row">{children}</div>
    </section>
  );
}

export function PhoneArtist({ id }: { id: string }) {
  const { data: loaded, error, retry } = useLoad(() => backend.artistPage(id), [id]);
  // Asked again when the ranking changes or everyone's list arrives.
  const { data, reload } = useReload(loaded, () => backend.artistPage(id));
  const rank = useSettings((s) => topRank(s.settings));
  const [gaveUp, setGaveUp] = useState(false);
  const ctx = usePlayer((s) => s.state?.queue.context ?? null);
  const status = usePlayer((s) => s.state?.status ?? "idle");
  const following = useSocial((s) => s.followIds.has(id));
  const erasOn = useSettings((s) => s.settings?.showEraCards);
  const credited = isCreditId(id);
  const disco = useArtistDiscography(id, data?.artist.title, data?.sections);
  const fans = useCatalogueFans(credited ? null : id, data?.artist.title);
  const lidarrReady = useGetSpots();
  const [allTop, setAllTop] = useState(false);
  const hero = useRef<HTMLDivElement>(null);
  const firstRank = useRef(rank);
  useEffect(() => {
    if (rank === firstRank.current) return;
    firstRank.current = rank;
    reload();
  }, [rank, reload]);
  // Everyone's list comes from the artist's info; until it's here (or a
  // short wait runs out) the rows are dimmed rather than shown in the wrong order.
  useEffect(() => {
    if (rank !== "all") return;
    let alive = true;
    setGaveUp(false);
    backend.artistInfo(id).catch(() => {});
    const timer = setTimeout(() => alive && setGaveUp(true), POPULAR_WAIT_MS);
    const done = (e: { id: string }) => {
      if (e.id !== id) return;
      setGaveUp(true);
      reload();
    };
    const un = backend.listen("artistinfo://popular", done);
    return () => {
      alive = false;
      clearTimeout(timer);
      un.then((f) => f());
    };
  }, [id, rank, reload]);
  // The shelves come a frame after the header, so the page opens at once.
  const [rest, setRest] = useState(false);
  useEffect(() => {
    if (!data || rest) return;
    const f = requestAnimationFrame(() => setRest(true));
    return () => cancelAnimationFrame(f);
  }, [data, rest]);
  // The photo fades as the page slides over it: set straight on the element, never a re-render.
  useEffect(() => {
    const el = hero.current;
    const page = el?.closest(".ph-page");
    if (!el || !page) return;
    let frame = 0;
    const onScroll = () => {
      if (frame) return;
      frame = requestAnimationFrame(() => {
        frame = 0;
        el.style.opacity = String(Math.max(0, 1 - page.scrollTop / 260));
      });
    };
    page.addEventListener("scroll", onScroll, { passive: true });
    return () => {
      page.removeEventListener("scroll", onScroll);
      cancelAnimationFrame(frame);
    };
  }, [data]);
  if (error && !data) return <PageFailed what="this artist" retry={retry} framed />;
  if (!data) return <PageLoading />;
  const { artist, sections } = data;
  const albums = sections.flatMap((s) => s.albums);
  const songs = albums.reduce((n, a) => n + a.trackCount, 0);
  const top = data.popular.slice(0, allTop ? TOP_TRACKS_ALL : TOP_TRACKS);
  const waiting = rank === "all" && !!data.popularPending && !gaveUp;
  const here = ctx?.kind === "artist" && ctx.id === artist.id;
  const playing = here && (status === "playing" || status === "loading" || status === "buffering");
  const eras = showEraCards(data.eras, erasOn) ? (data.eras ?? []) : [];
  return (
    <div className="ph-page ph-artist" data-testid="artist-page">
      <div ref={hero} className="ph-artist__hero">
        <Art path={photoPath(artist) ?? disco.photo} fluid requestSize={600} kind="artist" monogram={artist.title} />
      </div>
      <BackBar title={artist.title} after={220} />
      <h1 className="ph-artist__name">{artist.title}</h1>
      <div className="ph-artist__body">
        <p className="ph-artist__lib tabular">{inLibraryLine(albums.length, songs)}</p>
        <div className="ph-actions ph-actions--busy">
          <div className="ph-actions__left">
            <PlayPill playing={playing} label={playing ? "Pause" : `Play ${artist.title}`} onClick={() => (here ? void player.toggle() : void player.playArtist(artist.id))} />
            <PhoneShuffleToggle id={artist.id} kind="artist" name={artist.title} />
          </div>
          <div className="ph-actions__right">
            {!credited && (
              <button
                type="button"
                className={`ph-chip ph-artist__follow ${following ? "is-on" : ""}`}
                aria-pressed={following}
                aria-label={following ? `Following ${artist.title}` : `Follow ${artist.title}`}
                onClick={() => void setFollow({ id, title: artist.title, thumb: artist.thumb }, !following)}
              >
                {following ? "Following" : "Follow"}
              </button>
            )}
            <button type="button" className="ph-icon-btn" aria-label={`${artist.title} radio`} onClick={() => nav.go({ name: "radio", seed: `artist:${artist.id}` })}>
              <Radio size={22} aria-hidden="true" />
            </button>
          </div>
        </div>
        {top.length > 0 && (
          <section aria-label="Top tracks">
            <div className="ph-disco__head">
              <h2 className="ph-section-title">Top tracks</h2>
              {data.popular.length > TOP_TRACKS && (
                <button type="button" className="ph-link-btn" aria-expanded={allTop} onClick={() => setAllTop(!allTop)}>
                  {allTop ? "Show fewer" : "See all"}
                </button>
              )}
            </div>
            <Pills label="Rank top tracks by" items={RANKS} value={rank} onChange={(r) => void updateSettings(rankPatch(r, useSettings.getState().settings))} />
            <ul className={`ph-songs ${waiting ? "ph-songs--waiting" : ""}`} aria-busy={waiting || undefined}>
              {top.map((t, i) => (
                <SongRow
                  key={t.id}
                  track={t}
                  index={i + 1}
                  sub={t.viewCount ? plural(t.viewCount, "play") : t.albumTitle}
                  onPlay={() => void player.playTracks(top.map((x) => x.id), t.id)}
                />
              ))}
            </ul>
          </section>
        )}
        {rest && eras.length > 0 && (
          <Carousel title="Eras">
            {eras.map((e) => (
              <ShelfCard key={e.name} title={e.name} sub={[eraRange(e), plural(e.trackCount, "song")].filter(Boolean).join(" · ")} thumb={e.cover ?? null} kind="album" onOpen={() => nav.go({ name: "era", id, era: e.name })} />
            ))}
          </Carousel>
        )}
        {rest && (
          <DiscoShelf
            entries={disco.entries}
            lidarrReady={lidarrReady}
            artistLibraryId={credited ? null : id}
            onChanged={disco.reload}
            onShowAll={(f) => nav.go(discographyRoute(id, f))}
          />
        )}
        {rest && data.appearsOn.length > 0 && (
          <Shelf title="Appears on" onAll={() => nav.go({ name: "artistShelf", id, shelf: "appearsOn" })}>
            {data.appearsOn.slice(0, 12).map((a) => (
              <ShelfCard key={a.id} title={a.title} sub={a.artistTitle} thumb={a.thumb} kind="album" onOpen={() => nav.go({ name: "album", id: a.id })} />
            ))}
          </Shelf>
        )}
        {rest && data.similar.length + fans.length > 0 && (
          <Shelf title="Fans also like" onAll={() => nav.go({ name: "artistShelf", id, shelf: "fansAlsoLike" })}>
            {data.similar.slice(0, 12).map((a) => (
              <ShelfCard key={a.id} title={a.title} thumb={a.thumb} round kind="artist" onOpen={() => nav.go({ name: "artist", id: a.id })} />
            ))}
            {fans.slice(0, 8).map((a) => (
              <button key={a.mbid} type="button" className="ph-card-tile is-round" onClick={() => nav.go({ name: "catalogueArtist", mbid: a.mbid })}>
                <span className="ph-x__round-112">
                  <CataloguePhoto mbid={a.mbid} name={a.name} given={a.photo} size={112} />
                </span>
                <span className="ph-card-tile__title">{a.name}</span>
                <span className="ph-card-tile__sub">Not in your library</span>
              </button>
            ))}
          </Shelf>
        )}
      </div>
    </div>
  );
}
