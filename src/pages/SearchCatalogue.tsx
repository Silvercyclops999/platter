/**
 * Search's "Not in your library" section: artists, albums and songs from
 * MusicBrainz below the library's results. It asks only once typing has
 * paused, and only when the library found little or the user opens the
 * section. Hidden while offline or with its setting off.
 */
import { MoreHorizontal } from "lucide-react";
import { useEffect, useRef, useState, type MouseEvent } from "react";
import { Art } from "../components/Art";
import { openContextMenu } from "../components/ContextMenu";
import { Skeleton } from "../components/Skeleton";
import { backend } from "../lib/backend";
import { formatDuration } from "../lib/format";
import type { CatAlbum, CatArtist, CatSong, CatalogueSearch, SearchFilter } from "../lib/types";
import type { MenuItem } from "../stores/ui";
import { nav } from "../stores/router";
import { useSettings } from "../stores/settings";
import { NOT_IN_LIBRARY, addToMissing, catalogueKind, dismissArtist, useOnline } from "./catalogue";
import "./catalogue.css";

/** Typing must pause this long before MusicBrainz is asked. */
export const CATALOGUE_PAUSE_MS = 700;
/** Fewer library results than this and the section loads by itself. */
export const FEW_RESULTS = 5;

interface Row {
  key: string;
  title: string;
  byline: string;
  type: string;
  thumb: string | null;
  round: boolean;
  open: () => void;
  menu: MenuItem[];
}

function artistRow(a: CatArtist): Row {
  const open = () => nav.go({ name: "catalogueArtist", mbid: a.mbid });
  return {
    key: `ar-${a.mbid}`,
    title: a.name,
    byline: [a.kind === "Person" ? "Artist" : a.kind === "Group" ? "Band" : "Artist", a.disambiguation].filter(Boolean).join(" · "),
    type: "Artist",
    thumb: a.photo,
    round: true,
    open,
    menu: [
      { label: "Open", onSelect: open },
      { label: "Not interested", onSelect: () => dismissArtist({ mbid: a.mbid, name: a.name }) },
    ],
  };
}

function albumRow(a: CatAlbum): Row {
  const open = () => nav.go({ name: "catalogueAlbum", id: a.releaseGroupId });
  return {
    key: `al-${a.releaseGroupId}`,
    title: a.title,
    byline: [a.year, a.artist].filter(Boolean).join(" · "),
    type: catalogueKind(a),
    thumb: a.thumb,
    round: false,
    open,
    menu: [
      { label: "Open", onSelect: open },
      ...(a.artistMbid ? [{ label: "Go to artist", onSelect: () => nav.go({ name: "catalogueArtist", mbid: a.artistMbid! }) }] : []),
      ...(a.inMissing ? [] : [{ label: "Add to Missing", onSelect: () => void addToMissing(a) }]),
    ],
  };
}

function songRow(s: CatSong): Row {
  const open = () => (s.releaseGroupId ? nav.go({ name: "catalogueAlbum", id: s.releaseGroupId }) : s.artistMbid && nav.go({ name: "catalogueArtist", mbid: s.artistMbid }));
  return {
    key: `so-${s.recordingId}`,
    title: s.title,
    byline: [s.artist, s.album, s.durationMs ? formatDuration(s.durationMs) : null].filter(Boolean).join(" · "),
    type: "Song",
    thumb: s.thumb,
    round: false,
    open,
    menu: [
      { label: s.releaseGroupId ? "Go to album" : "Open", onSelect: open },
      ...(s.artistMbid ? [{ label: "Go to artist", onSelect: () => nav.go({ name: "catalogueArtist", mbid: s.artistMbid! }) }] : []),
    ],
  };
}

function CatalogueRow({ row }: { row: Row }) {
  const more = useRef<HTMLButtonElement>(null);
  const menu = (e: MouseEvent<HTMLElement>, at?: HTMLElement) => {
    e.preventDefault();
    openContextMenu(`Options for ${row.title}`, row.menu, at ?? e);
  };
  return (
    <li className="sr-row" aria-label={`${row.title}, ${row.type}, ${row.byline ? `${row.byline}, ` : ""}${NOT_IN_LIBRARY.toLowerCase()}`} onContextMenu={(e) => menu(e)} onDoubleClick={row.open}>
      <span className="sr-row__art">
        <Art path={row.thumb} size={48} round={row.round} kind={row.round ? "artist" : "album"} />
      </span>
      <span className="sr-row__text">
        <button type="button" className="sr-row__title" onClick={row.open}>
          <span className="truncate">{row.title}</span>
        </button>
        <span className="sr-row__by truncate">{row.byline || row.type}</span>
      </span>
      <span className="sr-type sr-row__type">{row.type}</span>
      <button ref={more} type="button" className="sr-row__more" aria-label={`More options for ${row.title}`} onClick={(e) => menu(e, more.current ?? undefined)}>
        <MoreHorizontal size={18} aria-hidden="true" />
      </button>
    </li>
  );
}

/** Which kinds a filter chip shows. */
function rowsFor(res: CatalogueSearch, filter: SearchFilter): Row[] {
  const artists = filter === "all" || filter === "artists" ? res.artists.map(artistRow) : [];
  const albums = filter === "all" || filter === "albums" ? res.albums.map(albumRow) : [];
  const songs = filter === "all" || filter === "songs" ? res.songs.map(songRow) : [];
  return [...artists, ...albums, ...songs];
}

export function SearchCatalogue({ query, filter, localCount }: { query: string; filter: SearchFilter; localCount: number }) {
  const on = useSettings((s) => s.settings?.catalogueSearch !== false);
  const online = useOnline();
  const few = localCount < FEW_RESULTS;
  const [opened, setOpened] = useState(false);
  const [shown, setShown] = useState<{ query: string; res: CatalogueSearch } | null>(null);
  const [failed, setFailed] = useState(false);
  const seq = useRef(0);
  const want = on && online && filter !== "playlists" && (few || opened);

  useEffect(() => {
    const mine = ++seq.current;
    setFailed(false);
    if (!want || query.trim().length < 2) return;
    // Only once typing has paused, so each keystroke doesn't ask MusicBrainz.
    const t = setTimeout(() => {
      backend.catalogueSearch(query).then(
        (res) => mine === seq.current && setShown({ query, res }),
        () => mine === seq.current && setFailed(true),
      );
    }, CATALOGUE_PAUSE_MS);
    return () => clearTimeout(t);
  }, [query, want]);

  // A new search starts closed again.
  useEffect(() => setOpened(false), [query]);

  if (!on || !online || filter === "playlists" || query.trim().length < 2) return null;
  const headingId = "sr-catalogue-title";
  if (!few && !opened) {
    return (
      <section className="sr-catalogue" aria-labelledby={headingId}>
        <h2 id={headingId} className="sr-only">
          {NOT_IN_LIBRARY}
        </h2>
        <button type="button" className="btn btn--secondary btn--sm sr-catalogue__open" onClick={() => setOpened(true)}>
          Show music that isn't in your library
        </button>
      </section>
    );
  }
  const res = shown && shown.query === query ? shown.res : null;
  if (res && !res.shown) return null;
  const rows = res ? rowsFor(res, filter) : [];
  return (
    <section className="sr-catalogue" aria-labelledby={headingId} aria-busy={!res && !failed}>
      <div className="sr-catalogue__head">
        <h2 id={headingId} className="section-title">
          {NOT_IN_LIBRARY}
        </h2>
        <span className="sr-catalogue__note">From MusicBrainz</span>
      </div>
      {failed ? (
        <p className="sr-catalogue__note" role="status">
          MusicBrainz isn't answering right now. Try again in a minute.
        </p>
      ) : !res ? (
        <div aria-hidden="true">
          {Array.from({ length: 3 }, (_, i) => (
            <div key={i} className="sr-row sr-row--skeleton">
              <Skeleton width={48} height={48} />
              <span className="sr-row__text">
                <Skeleton width={180} height={14} />
                <Skeleton width={110} height={12} />
              </span>
            </div>
          ))}
        </div>
      ) : rows.length === 0 ? (
        <p className="sr-catalogue__note" role="status">
          Nothing else found on MusicBrainz.
        </p>
      ) : (
        <ul className="sr-list" aria-label={NOT_IN_LIBRARY}>
          {rows.map((r) => (
            <CatalogueRow key={r.key} row={r} />
          ))}
        </ul>
      )}
    </section>
  );
}
