import { ChevronDown, Ellipsis, LayoutGrid, List, Pause, Play } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { Art } from "../components/Art";
import { IconButton } from "../components/Button";
import { CardSkeleton } from "../components/Card";
import { openContextMenu } from "../components/ContextMenu";
import { Skeleton } from "../components/Skeleton";
import { TrackList, TrackRow, TrackRowSkeleton } from "../components/TrackRow";
import { backend } from "../lib/backend";
import type { Album, Track } from "../lib/types";
import { useLoad } from "../lib/useLoad";
import { player, usePlayer } from "../stores/player";
import { nav } from "../stores/router";
import type { MenuItem } from "../stores/ui";
import { AlbumCard, useAlbumActions } from "./cards";
import {
  SORT_LABEL,
  VIEW_LABEL,
  discographyReleases,
  loadDiscographyPrefs,
  releaseLine,
  saveDiscographyPrefs,
  typeOptions,
  validType,
  type DiscographyPrefs,
  type DiscographySort,
  type DiscographyType,
  type DiscographyView,
  type SectionId,
} from "./discography";
import "./discography.css";
import { LoadError } from "../components/LoadError";

/** Starts over (type filter) for each artist; sort and view are remembered on this device. */
export function DiscographyPage({ id, type }: { id: string; type?: SectionId }) {
  return <DiscographyView key={id} id={id} type={type} />;
}

function DiscographyView({ id, type: startType }: { id: string; type?: SectionId }) {
  const { data, error, retry } = useLoad(() => backend.artistPage(id), [id]);
  const [type, setType] = useState<DiscographyType>(startType ?? "all");
  const [prefs, setPrefs] = useState<DiscographyPrefs>(loadDiscographyPrefs);
  const choose = (p: Partial<DiscographyPrefs>) => {
    const next = { ...prefs, ...p };
    setPrefs(next);
    saveDiscographyPrefs(next);
  };

  if (error && !data) return <LoadError what="this discography" error={error} onRetry={retry} />;

  const sections = data?.sections ?? [];
  const shownType = data ? validType(sections, type) : type;
  const options = typeOptions(sections);
  const releases = data ? discographyReleases(sections, shownType, prefs.sort) : null;

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

  return (
    <div className="page-pad disco">
      <header className="disco__head">
        {data ? (
          <h1 className="disco__title">
            <button type="button" className="link" onClick={() => nav.go({ name: "artist", id })}>
              {data.artist.title}
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
            disabled={!data}
            onClick={(e) => typeMenu(e.currentTarget)}
          >
            <span>{typeLabel}</span>
            <ChevronDown size={16} aria-hidden="true" />
          </button>
          <button
            type="button"
            className="disco__menu-btn"
            aria-haspopup="menu"
            aria-label={`Sort and view: ${SORT_LABEL[prefs.sort]}, ${VIEW_LABEL[prefs.view].toLowerCase()} view`}
            onClick={(e) => sortViewMenu(e.currentTarget)}
          >
            <span>{SORT_LABEL[prefs.sort]}</span>
            <ViewIcon size={16} aria-hidden="true" />
          </button>
        </div>
      </header>

      {prefs.view === "grid" ? (
        <div className="shelf-page__grid disco__grid" role="list" aria-label="Releases" aria-busy={!releases || undefined}>
          {releases
            ? releases.map((album) => (
                <div role="listitem" key={album.id}>
                  <AlbumCard album={album} subtitle="year" />
                </div>
              ))
            : Array.from({ length: 12 }, (_, i) => (
                <div key={i} aria-hidden="true">
                  <CardSkeleton kind="album" />
                </div>
              ))}
        </div>
      ) : (
        <div className="disco__list" aria-busy={!releases || undefined}>
          {releases ? releases.map((album) => <ReleaseBlock key={album.id} album={album} />) : <ReleaseBlockSkeleton />}
        </div>
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

/** One release: cover, title, "type · year · N songs", a compact play and menu, then its full tracklist. */
function ReleaseBlock({ album }: { album: Album }) {
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
          <Art path={album.thumb} size={136} />
        </button>
        <div className="disco-release__text">
          <h2 className="disco-release__title">
            <button type="button" className="link" onClick={a.open}>
              {album.title}
            </button>
          </h2>
          <p className="disco-release__meta">{releaseLine(album)}</p>
          <div className="disco-release__actions">
            <IconButton
              label={a.playing ? `Pause ${album.title}` : `Play ${album.title}`}
              variant="accent"
              size="md"
              icon={a.playing ? <Pause size={16} fill="currentColor" /> : <Play size={16} fill="currentColor" className="nudge-right" />}
              onClick={a.play}
            />
            <IconButton label={`More options for ${album.title}`} size="md" icon={<Ellipsis size={20} />} onClick={(e) => a.openMenu(e.currentTarget)} />
          </div>
        </div>
      </header>
      <TrackList label={`${album.title} tracks`} showAlbum={false}>
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
