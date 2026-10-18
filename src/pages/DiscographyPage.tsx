import { ChevronDown, CircleCheck, CirclePlus, Ellipsis, LayoutGrid, List, Pause, Play } from "lucide-react";
import { useEffect, useId, useRef, useState } from "react";
import { Art } from "../components/Art";
import { IconButton } from "../components/Button";
import { CardSkeleton } from "../components/Card";
import { openContextMenu } from "../components/ContextMenu";
import { Skeleton } from "../components/Skeleton";
import { TrackList, TrackRow, TrackRowSkeleton } from "../components/TrackRow";
import { backend } from "../lib/backend";
import { isCreditId } from "../lib/credits";
import type { Album, CatAlbum, Track } from "../lib/types";
import { useLoad } from "../lib/useLoad";
import { setLiked, useLikes } from "../stores/likes";
import { player, usePlayer } from "../stores/player";
import { nav } from "../stores/router";
import { updateSettings, useSettings } from "../stores/settings";
import type { MenuItem } from "../stores/ui";
import { catalogueEntries, type DiscoEntry } from "./artist";
import { AlbumCard, useAlbumActions } from "./cards";
import { CatalogueAction, CatalogueAlbumCard, catalogueAlbumMenu, openAlbum } from "./catalogue";
import {
  SORT_LABEL,
  VIEW_LABEL,
  discographyGroups,
  discographyPrefs,
  prefsPatch,
  releaseLine,
  typeOptions,
  validType,
  type DiscographyPrefs,
  type DiscographySort,
  type DiscographyType,
  type DiscographyView,
  type SectionId,
} from "./discography";
import { useArtistDiscography } from "./discographySources";
import { useLidarrReady } from "./lidarrShared";
import "./discography.css";
import { LoadError } from "../components/LoadError";

/**
 * Every release an artist has: the library's and the ones you can get,
 * sectioned by type. Starts over (type filter) for each artist; sort and
 * view are remembered on this device. `catalogue`: `id` is the MusicBrainz
 * id of an artist outside the library.
 */
export function DiscographyPage({ id, type, catalogue }: { id: string; type?: SectionId; catalogue?: boolean }) {
  return catalogue ? <CatalogueDiscography key={`mb:${id}`} mbid={id} type={type} /> : <LibraryDiscography key={id} id={id} type={type} />;
}

function LibraryDiscography({ id, type }: { id: string; type?: SectionId }) {
  const { data, error, retry } = useLoad(() => backend.artistPage(id), [id]);
  const disco = useArtistDiscography(id, data?.artist.title, data?.sections);
  if (error && !data) return <LoadError what="this discography" error={error} onRetry={retry} />;
  return (
    <DiscographyView
      title={data?.artist.title}
      onTitle={() => nav.go({ name: "artist", id })}
      entries={data ? disco.entries : null}
      startType={type}
      artistLibraryId={isCreditId(id) ? null : id}
      onChanged={disco.reload}
    />
  );
}

function CatalogueDiscography({ mbid, type }: { mbid: string; type?: SectionId }) {
  const [version, setVersion] = useState(0);
  const { data, error, retry } = useLoad(() => backend.catalogueArtist(mbid), [mbid, version]);
  if (error && !data) return <LoadError what="this discography" error={error} onRetry={retry} />;
  return (
    <DiscographyView
      title={data?.artist.name}
      onTitle={() => nav.go({ name: "catalogueArtist", mbid })}
      entries={data ? catalogueEntries(data.sections, data.popular) : null}
      startType={type}
      onChanged={() => setVersion((v) => v + 1)}
    />
  );
}

function DiscographyView({
  title,
  onTitle,
  entries,
  startType,
  artistLibraryId,
  onChanged,
}: {
  title: string | undefined;
  onTitle: () => void;
  /** Null while loading. */
  entries: DiscoEntry[] | null;
  startType?: SectionId;
  artistLibraryId?: string | null;
  onChanged: () => void;
}) {
  const [type, setType] = useState<DiscographyType>(startType ?? "all");
  const saved = discographyPrefs(useSettings((s) => s.settings));
  // Choices show at once, before Settings has them.
  const [mine, setMine] = useState<Partial<DiscographyPrefs>>({});
  const prefs = { ...saved, ...mine };
  const lidarrReady = useLidarrReady();
  const choose = (p: Partial<DiscographyPrefs>) => {
    setMine((m) => ({ ...m, ...p }));
    void updateSettings(prefsPatch(p));
  };

  const shownType = entries ? validType(entries, type) : type;
  const options = typeOptions(entries ?? []);
  const groups = entries ? discographyGroups(entries, shownType, prefs.sort, prefs.reverse) : null;
  // Headings only when the page holds more than one type.
  const headed = !!groups && groups.length > 1;

  const typeMenu = (el: HTMLElement) =>
    openContextMenu(
      "Release type",
      options.map((o): MenuItem => ({ label: o.label, icon: null, checked: o.id === shownType, onSelect: () => setType(o.id) })),
      el,
    );
  const sortViewMenu = (el: HTMLElement) =>
    openContextMenu(
      "Sort and view",
      [
        ...(["date", "name"] as DiscographySort[]).map(
          (s, i): MenuItem => ({
            heading: i === 0 ? "Sort by" : undefined,
            label: s === "name" ? "Name (A to Z)" : "Release date (newest first)",
            icon: null,
            checked: prefs.sort === s,
            onSelect: () => choose({ sort: s }),
          }),
        ),
        { label: "Reverse order", icon: null, checked: prefs.reverse, onSelect: () => choose({ reverse: !prefs.reverse }) },
        ...(["list", "grid"] as DiscographyView[]).map(
          (v, i): MenuItem => ({
            heading: i === 0 ? "View as" : undefined,
            separator: i === 0,
            label: VIEW_LABEL[v],
            icon: v === "list" ? List : LayoutGrid,
            checked: prefs.view === v,
            onSelect: () => choose({ view: v }),
          }),
        ),
      ],
      el,
    );
  const typeLabel = options.find((o) => o.id === shownType)?.label ?? "All";
  const ViewIcon = prefs.view === "list" ? List : LayoutGrid;
  const sortLabel = `${SORT_LABEL[prefs.sort]}${prefs.reverse ? ", reversed" : ""}`;

  return (
    <div className="page-pad disco">
      <header className="disco__head">
        {title ? (
          <h1 className="disco__title">
            <button type="button" className="link" onClick={onTitle}>
              {title}
            </button>
          </h1>
        ) : (
          <Skeleton width={220} height={28} />
        )}
        <div className="disco__tools">
          <button
            type="button"
            className="disco__menu-btn"
            aria-haspopup="menu"
            aria-label={`Release type: ${typeLabel}`}
            disabled={!entries}
            onClick={(e) => typeMenu(e.currentTarget)}
          >
            <span>{typeLabel}</span>
            <ChevronDown size={16} aria-hidden="true" />
          </button>
          <button
            type="button"
            className="disco__menu-btn"
            aria-haspopup="menu"
            aria-label={`Sort and view: ${sortLabel}, ${VIEW_LABEL[prefs.view].toLowerCase()} view`}
            onClick={(e) => sortViewMenu(e.currentTarget)}
          >
            <span>{sortLabel}</span>
            <ViewIcon size={16} aria-hidden="true" />
          </button>
        </div>
      </header>

      {!groups ? (
        prefs.view === "grid" ? (
          <div className="shelf-page__grid disco__grid" aria-busy="true">
            {Array.from({ length: 12 }, (_, i) => (
              <div key={i} aria-hidden="true">
                <CardSkeleton kind="album" />
              </div>
            ))}
          </div>
        ) : (
          <div className="disco__list" aria-busy="true">
            <ReleaseBlockSkeleton />
          </div>
        )
      ) : groups.length === 0 ? (
        <p className="disco__none">No releases to show yet.</p>
      ) : (
        groups.map((g) => (
          <DiscographyGroupView key={g.section} label={g.label} headed={headed} entries={g.entries} view={prefs.view} lidarrReady={lidarrReady} artistLibraryId={artistLibraryId} onChanged={onChanged} />
        ))
      )}
    </div>
  );
}

function DiscographyGroupView({
  label,
  headed,
  entries,
  view,
  lidarrReady,
  artistLibraryId,
  onChanged,
}: {
  label: string;
  headed: boolean;
  entries: DiscoEntry[];
  view: DiscographyView;
  lidarrReady: boolean;
  artistLibraryId?: string | null;
  onChanged: () => void;
}) {
  const headingId = useId();
  const heading = headed ? (
    <h2 id={headingId} className="disco__section-title">
      {label}
    </h2>
  ) : null;
  if (view === "grid")
    return (
      <div className="disco__group">
        {heading}
        <div className="shelf-page__grid disco__grid" role="list" aria-label={headed ? label : "Releases"}>
          {entries.map((e) => (
            <div role="listitem" key={e.key}>
              {e.album ? (
                <AlbumCard album={e.album} subtitle="year" />
              ) : (
                <CatalogueAlbumCard album={e.cat!} lidarrReady={lidarrReady || !!e.missing} artistLibraryId={artistLibraryId} onChanged={onChanged} />
              )}
            </div>
          ))}
        </div>
      </div>
    );
  return (
    <div className="disco__group disco__list">
      {heading}
      {entries.map((e) =>
        e.album ? (
          <ReleaseBlock key={e.key} album={e.album} line={releaseLine(e)} headed={headed} />
        ) : (
          <CatalogueReleaseBlock key={e.key} album={e.cat!} line={releaseLine(e)} headed={headed} lidarrReady={lidarrReady || !!e.missing} artistLibraryId={artistLibraryId} onChanged={onChanged} />
        ),
      )}
    </div>
  );
}

/** How far outside the view a release's tracklist starts loading. */
const LOAD_MARGIN = "800px";

/** True once the element has come near the view (always true without IntersectionObserver). */
function useNearView(ref: React.RefObject<HTMLElement | null>): boolean {
  const [near, setNear] = useState(() => typeof IntersectionObserver === "undefined");
  useEffect(() => {
    const el = ref.current;
    if (near || !el) return;
    const io = new IntersectionObserver(
      (entries) => {
        if (entries.some((e) => e.isIntersecting)) setNear(true);
      },
      { rootMargin: LOAD_MARGIN },
    );
    io.observe(el);
    return () => io.disconnect();
  }, [ref, near]);
  return near;
}

/** A release's title: one level under the section headings when there are some. */
function ReleaseTitle({ headed, children }: { headed: boolean; children: React.ReactNode }) {
  return headed ? <h3 className="disco-release__title">{children}</h3> : <h2 className="disco-release__title">{children}</h2>;
}

/** Save: every song of the release in Liked Songs (or out again when they all are). */
function SaveRelease({ title, tracks }: { title: string; tracks: Track[] | null }) {
  const all = useLikes((s) => !!tracks?.length && tracks.every((t) => s.ids.has(t.id)));
  return (
    <IconButton
      label={all ? `Remove ${title} from Liked Songs` : `Save ${title} to Liked Songs`}
      size="md"
      disabled={!tracks?.length}
      icon={all ? <CircleCheck size={20} /> : <CirclePlus size={20} />}
      onClick={(e) => tracks && void setLiked(tracks, !all, { from: e.currentTarget })}
    />
  );
}

/** One release: cover, title, "type · year · N songs", a compact play, save and menu, then its full tracklist. */
function ReleaseBlock({ album, line, headed }: { album: Album; line: string; headed: boolean }) {
  const box = useRef<HTMLElement>(null);
  const near = useNearView(box);
  const a = useAlbumActions(album);
  const nowTrackId = usePlayer((s) => s.state?.track?.id);
  const status = usePlayer((s) => s.state?.status);
  const [tracks, setTracks] = useState<Track[] | null>(null);
  useEffect(() => {
    if (!near) return;
    let alive = true;
    backend.albumPage(album.id).then(
      (p) => alive && setTracks(p.tracks),
      () => alive && setTracks([]),
    );
    return () => {
      alive = false;
    };
  }, [near, album.id]);

  return (
    <section ref={box} className="disco-release" aria-label={album.title}>
      <header className="disco-release__head">
        <button type="button" className="disco-release__cover" tabIndex={-1} aria-hidden="true" onClick={a.open}>
          <Art path={album.thumb} size={136} sleeve={{ title: album.title, artist: album.artistTitle }} />
        </button>
        <div className="disco-release__text">
          <ReleaseTitle headed={headed}>
            <button type="button" className="link" onClick={a.open}>
              {album.title}
            </button>
          </ReleaseTitle>
          <p className="disco-release__meta">{line}</p>
          <div className="disco-release__actions">
            <IconButton
              label={a.playing ? `Pause ${album.title}` : `Play ${album.title}`}
              variant="accent"
              size="md"
              icon={a.playing ? <Pause size={16} fill="currentColor" /> : <Play size={16} fill="currentColor" className="nudge-right" />}
              onClick={a.play}
            />
            <SaveRelease title={album.title} tracks={tracks} />
            <IconButton label={`More options for ${album.title}`} size="md" icon={<Ellipsis size={20} />} onClick={(e) => a.openMenu(e.currentTarget)} />
          </div>
        </div>
      </header>
      <TrackList label={`${album.title} songs`} showAlbum={false}>
        {tracks
          ? tracks.map((t) => (
              <TrackRow
                key={t.id}
                track={t}
                index={t.trackNumber}
                showAlbum={false}
                showArtist={!!t.trackArtist || t.artistId !== album.artistId}
                active={t.id === nowTrackId}
                playing={t.id === nowTrackId && status === "playing"}
                onPlay={() => player.playAlbum(album.id, t.id)}
              />
            ))
          : Array.from({ length: Math.min(Math.max(album.trackCount, 1), 12) }, (_, i) => <TrackRowSkeleton key={i} showAlbum={false} />)}
      </TrackList>
    </section>
  );
}

/** A release you can get: cover, title, "type · year", and Get with a menu (no tracklist). */
function CatalogueReleaseBlock({
  album,
  line,
  headed,
  lidarrReady,
  artistLibraryId,
  onChanged,
}: {
  album: CatAlbum;
  line: string;
  headed: boolean;
  lidarrReady: boolean;
  artistLibraryId?: string | null;
  onChanged: () => void;
}) {
  return (
    <section className="disco-release disco-release--catalogue" aria-label={album.title}>
      <header className="disco-release__head">
        <button type="button" className="disco-release__cover" tabIndex={-1} aria-hidden="true" onClick={() => openAlbum(album)}>
          <Art path={album.thumb} size={136} sleeve={{ title: album.title, artist: album.artist }} />
        </button>
        <div className="disco-release__text">
          <ReleaseTitle headed={headed}>
            <button type="button" className="link" onClick={() => openAlbum(album)}>
              {album.title}
            </button>
          </ReleaseTitle>
          <p className="disco-release__meta">{line}</p>
          <div className="disco-release__actions">
            <CatalogueAction album={album} lidarrReady={lidarrReady} artistLibraryId={artistLibraryId} onChanged={onChanged} />
            <IconButton
              label={`More options for ${album.title}`}
              size="md"
              icon={<Ellipsis size={20} />}
              onClick={(e) => openContextMenu(`Options for ${album.title}`, catalogueAlbumMenu(album, onChanged), e.currentTarget)}
            />
          </div>
        </div>
      </header>
    </section>
  );
}

function ReleaseBlockSkeleton() {
  return (
    <div className="disco-release" aria-hidden="true">
      <div className="disco-release__head">
        <Skeleton width={136} height={136} />
        <div className="disco-release__text">
          <Skeleton width={240} height={28} />
          <Skeleton width={140} height={14} style={{ marginTop: 8 }} />
        </div>
      </div>
      {Array.from({ length: 6 }, (_, i) => (
        <TrackRowSkeleton key={i} showAlbum={false} />
      ))}
    </div>
  );
}
