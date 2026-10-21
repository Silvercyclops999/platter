/**
 * Music you don't have, in with search's library results: artists, albums
 * and songs, each kind asked for on its own and shown as it arrives
 * (artists and albums from Lidarr's search when it's set up, songs and the
 * rest from MusicBrainz), kept a while per query. Rows look like library
 * rows, with Get on albums and "Get the album" on songs. Hidden while
 * offline or with its setting off.
 */
import { MoreHorizontal } from "lucide-react";
import { useEffect, useRef, useState, type MouseEvent, type RefObject } from "react";
import { Art } from "../components/Art";
import { openContextMenu } from "../components/ContextMenu";
import { Skeleton } from "../components/Skeleton";
import { useArtistPhoto } from "../lib/artistPhotos";
import { backend } from "../lib/backend";
import { formatDuration } from "../lib/format";
import type { CatAlbum, CatArtist, CataloguePart, CataloguePartKind, CatSong, SearchFilter } from "../lib/types";
import type { MenuItem } from "../stores/ui";
import { nav } from "../stores/router";
import { useRequestFor } from "../stores/requests";
import { useSettings } from "../stores/settings";
import { useIsGuest } from "../stores/asks";
import { tapToPlay } from "../stores/taps";
import { toast } from "../stores/toasts";
import { addToMissing, catalogueKind, dismissArtist, useOnline } from "./catalogue";
import { GetButton } from "./GetButton";
import "./catalogue.css";

/** Typing must pause this long before the catalogue is asked. */
export const CATALOGUE_PAUSE_MS = 300;
/** How long a query's results are kept in this window. */
const KEEP_MS = 5 * 60_000;

const kept = new Map<string, { at: number; part: CataloguePart }>();
/** Answers on their way, so the same question isn't asked twice at once. */
const asking = new Map<string, Promise<CataloguePart>>();

/** Forget kept results (tests). */
export function resetCatalogueCache() {
  kept.clear();
  asking.clear();
}

export interface CatalogueResults {
  artists: CatArtist[];
  albums: CatAlbum[];
  songs: CatSong[];
  /** Library artists and albums a catalogue result folded into. */
  inLibrary: Set<string>;
  /** Kinds still on their way. */
  pending: CataloguePartKind[];
  /** Where artists and albums came from. */
  source: "lidarr" | "musicBrainz" | "";
  failed: boolean;
}

const PARTS_FOR: Record<SearchFilter, CataloguePartKind[]> = {
  all: ["artists", "albums", "songs"],
  artists: ["artists"],
  albums: ["albums"],
  songs: ["songs"],
  playlists: [],
};

/** Whether a tap on a song you don't have gets and plays it (Lidarr set up and the setting on). */
export function useTapPlays(lidarrReady: boolean): boolean {
  const on = useSettings((s) => s.settings?.lidarrTapToGet !== false);
  return on && lidarrReady;
}

/** Whether search shows music you don't have right now. */
export function useCatalogueOn(): boolean {
  const on = useSettings((s) => s.settings?.catalogueSearch !== false);
  const online = useOnline();
  return on && online;
}

/**
 * The catalogue's answers for a query, filled in kind by kind as they
 * arrive. Null while it's off, offline, or the query is too short.
 */
export function useCatalogueSearch(query: string, filter: SearchFilter): CatalogueResults | null {
  const on = useCatalogueOn();
  const q = query.trim();
  const parts = PARTS_FOR[filter];
  const want = on && q.length >= 2 && parts.length > 0;
  const [res, setRes] = useState<{ key: string; r: CatalogueResults } | null>(null);
  const seq = useRef(0);
  const key = `${filter}|${q.toLowerCase()}`;

  useEffect(() => {
    const mine = ++seq.current;
    if (!want) return;
    const start: CatalogueResults = { artists: [], albums: [], songs: [], inLibrary: new Set(), pending: [...parts], source: "", failed: false };
    const apply = (p: CataloguePart | null, kind: CataloguePartKind) =>
      setRes((prev) => {
        if (mine !== seq.current) return prev;
        const base = prev && prev.key === key ? prev.r : start;
        const next: CatalogueResults = { ...base, pending: base.pending.filter((k) => k !== kind) };
        if (!p) return { key, r: { ...next, failed: true } };
        if (!p.shown) return { key, r: next };
        next.inLibrary = new Set([...base.inLibrary, ...p.inLibrary]);
        if (kind === "artists") next.artists = p.artists;
        if (kind === "albums") next.albums = p.albums;
        if (kind === "songs") next.songs = p.songs;
        if (kind !== "songs" && p.source) next.source = p.source;
        return { key, r: next };
      });
    // What this window kept shows at once.
    const now = Date.now();
    const fresh = parts.map((k) => kept.get(`${k}|${q.toLowerCase()}`)).map((x) => (x && now - x.at < KEEP_MS ? x.part : null));
    setRes({ key, r: start });
    fresh.forEach((p, i) => p && apply(p, parts[i]));
    const missing = parts.filter((_, i) => !fresh[i]);
    if (!missing.length) return;
    // Only once typing has paused, so each keystroke doesn't ask.
    const t = setTimeout(() => {
      for (const kind of missing) {
        const k = `${kind}|${q.toLowerCase()}`;
        let p = asking.get(k);
        if (!p) {
          p = backend.catalogueSearchPart(q, kind).then((part) => {
            kept.set(k, { at: Date.now(), part });
            return part;
          });
          asking.set(k, p);
          p.then(
            () => asking.delete(k),
            () => asking.delete(k),
          );
        }
        p.then(
          (part) => apply(part, kind),
          () => apply(null, kind),
        );
      }
    }, CATALOGUE_PAUSE_MS);
    return () => clearTimeout(t);
    // `parts` follows `filter`.
  }, [key, want]);

  if (!want) return null;
  return res && res.key === key ? res.r : { artists: [], albums: [], songs: [], inLibrary: new Set(), pending: [...parts], source: "", failed: false };
}

/** Whether a catalogue answer has anything to show. */
export function hasCatalogue(c: CatalogueResults | null): boolean {
  return !!c && c.artists.length + c.albums.length + c.songs.length > 0;
}

/** Folded the way the library's top result is compared with the query. */
export const foldName = (s: string) =>
  s
    .toLowerCase()
    .normalize("NFKD")
    .replace(/[̀-ͯ]/g, "")
    .replace(/^the\s+/, "")
    .replace(/[^\p{L}\p{N}]+/gu, " ")
    .trim();

// ---------------------------------------------------------------- rows

interface Row {
  key: string;
  title: string;
  byline: string;
  type: string;
  thumb: string | null;
  round: boolean;
  open: () => void;
  menu: MenuItem[];
  /** For an artist: their id, so a photo is looked up as the row shows. */
  artist?: CatArtist;
  album?: CatAlbum;
  song?: CatSong;
  /** A song a tap gets and plays. */
  tapPlays?: boolean;
}

function artistRow(a: CatArtist): Row {
  const open = () => nav.go({ name: "catalogueArtist", mbid: a.mbid });
  return {
    key: `ar-${a.mbid}`,
    title: a.name,
    byline: [a.kind === "Group" ? "Band" : "Artist", a.disambiguation].filter(Boolean).join(" · "),
    type: "Artist",
    thumb: a.photo,
    round: true,
    open,
    artist: a,
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
    album: a,
    menu: [
      { label: "Open", onSelect: open },
      ...(a.artistMbid ? [{ label: "Go to artist", onSelect: () => nav.go({ name: "catalogueArtist", mbid: a.artistMbid! }) }] : []),
      ...(a.inMissing ? [] : [{ label: "Add to Missing", onSelect: () => void addToMissing(a) }]),
    ],
  };
}

/**
 * Tap a song you don't have: it's got through Lidarr and plays when it's
 * here. The very first request ever is confirmed on its album's page.
 */
export async function playCatalogueSong(s: CatSong) {
  const out = await tapToPlay({
    title: s.title,
    artist: s.artist,
    artistMbid: s.artistMbid,
    album: s.album,
    releaseGroupId: s.releaseGroupId,
    durationMs: s.durationMs,
    thumb: s.thumb,
  });
  if (!out?.needs) return;
  if (out.needs.kind === "firstTime" && s.releaseGroupId) {
    toast.info("Your first request is confirmed by typing the album's name, once. Get it here, then a tap is all it takes");
    nav.go({ name: "catalogueAlbum", id: s.releaseGroupId });
  } else {
    toast.info(out.needs.reason);
  }
}

function songRow(s: CatSong, tap: boolean): Row {
  const page = () => (s.releaseGroupId ? nav.go({ name: "catalogueAlbum", id: s.releaseGroupId }) : s.artistMbid && nav.go({ name: "catalogueArtist", mbid: s.artistMbid }));
  const open = tap ? () => void playCatalogueSong(s) : page;
  return {
    key: `so-${s.recordingId}`,
    title: s.title,
    // A tap plays it, so the row says Get where a library song has its length.
    byline: [s.artist, s.album, !tap && s.durationMs ? formatDuration(s.durationMs) : null].filter(Boolean).join(" · "),
    type: "Song",
    thumb: s.thumb,
    round: false,
    open,
    song: s,
    tapPlays: tap,
    menu: [
      ...(tap ? [{ label: "Play", onSelect: open }] : []),
      { label: s.releaseGroupId ? "Go to album" : "Open", onSelect: page },
      ...(s.artistMbid ? [{ label: "Go to artist", onSelect: () => nav.go({ name: "catalogueArtist", mbid: s.artistMbid! }) }] : []),
    ],
  };
}

/** The picture for a row: an artist's photo is looked up as it shows. */
function RowArt({ row, box, size }: { row: Row; box: RefObject<HTMLElement | null>; size: number }) {
  const { photo } = useArtistPhoto(box, row.artist?.mbid, row.title, row.artist ? row.artist.photo : row.thumb);
  return <Art path={row.artist ? photo : row.thumb} size={size} round={row.round} kind={row.round ? "artist" : "album"} monogram={row.artist ? row.title : undefined} />;
}

/** Get (albums) or "Get the album" (songs). */
function RowGet({ row, lidarrReady }: { row: Row; lidarrReady: boolean }) {
  const guest = useIsGuest();
  if (row.album) {
    const a = row.album;
    return (
      <GetButton
        album={{ releaseGroupId: a.releaseGroupId, title: a.title, artist: a.artist, artistMbid: a.artistMbid, thumb: a.thumb }}
        known={a.request}
        libraryAlbumId={a.libraryAlbumId}
        refusal={a.refusal}
        lidarrReady={lidarrReady}
      />
    );
  }
  if (row.tapPlays) {
    return (
      <span className="sr-get-mark" aria-hidden="true">
        {guest ? "Request" : "Get"}
      </span>
    );
  }
  if (row.song?.releaseGroupId && row.song.album) {
    const s = row.song;
    return (
      <GetButton
        album={{ releaseGroupId: s.releaseGroupId!, title: s.album!, artist: s.artist, artistMbid: s.artistMbid, thumb: s.thumb }}
        lidarrReady={lidarrReady}
        idleLabel="Get the album"
      />
    );
  }
  return null;
}

export function CatalogueRow({ row, lidarrReady }: { row: Row; lidarrReady: boolean }) {
  const more = useRef<HTMLButtonElement>(null);
  const box = useRef<HTMLLIElement>(null);
  const menu = (e: MouseEvent<HTMLElement>, at?: HTMLElement) => {
    e.preventDefault();
    openContextMenu(`Options for ${row.title}`, row.menu, at ?? e);
  };
  const rec = useRequestFor(row.album?.releaseGroupId ?? row.song?.releaseGroupId);
  const inLibrary = !!row.album?.libraryAlbumId || rec?.state === "imported";
  const where = row.tapPlays ? "not in your library, a tap gets it and plays it" : inLibrary ? (row.song ? "its album is in your library" : "in your library") : "not in your library";
  return (
    <li ref={box} className="sr-row sr-row--catalogue" aria-label={`${row.title}, ${row.type}${row.byline ? `, ${row.byline}` : ""}, ${where}`} onContextMenu={(e) => menu(e)} onDoubleClick={row.open}>
      <span className="sr-row__art">
        <RowArt row={row} box={box} size={48} />
      </span>
      <span className="sr-row__text">
        <button type="button" className="sr-row__title" onClick={row.open}>
          <span className="truncate">{row.title}</span>
        </button>
        <span className="sr-row__by truncate">{row.byline || row.type}</span>
      </span>
      {/* A double click or right click on Get stays on Get (it doesn't open the row). */}
      <span className="sr-row__action" onDoubleClick={(e) => e.stopPropagation()} onContextMenu={(e) => e.stopPropagation()}>
        <RowGet row={row} lidarrReady={lidarrReady} />
      </span>
      <span className="sr-type sr-row__type">{row.type}</span>
      <button ref={more} type="button" className="sr-row__more" aria-label={`More options for ${row.title}`} onClick={(e) => menu(e, more.current ?? undefined)}>
        <MoreHorizontal size={18} aria-hidden="true" />
      </button>
    </li>
  );
}

/** A catalogue artist as the top result: big photo, name, and their page a click away. */
export function CatalogueTopResult({ artist }: { artist: CatArtist }) {
  const box = useRef<HTMLElement>(null);
  const { photo } = useArtistPhoto(box, artist.mbid, artist.name, artist.photo);
  const open = () => nav.go({ name: "catalogueArtist", mbid: artist.mbid });
  return (
    <section ref={box} className="sr-top" aria-label="Top result">
      <span className="sr-row__art sr-top__art">
        <Art path={photo} size={92} round kind="artist" monogram={artist.name} />
      </span>
      <div className="sr-top__text">
        <button type="button" className="sr-top__title" onClick={open}>
          <span className="truncate">{artist.name}</span>
        </button>
        <div className="sr-top__sub truncate">
          <span className="sr-type">Artist</span>
          {artist.disambiguation && <> · {artist.disambiguation}</>}
        </div>
      </div>
    </section>
  );
}

/** Rows for a filter, catalogue kinds in order. */
export function catalogueRows(c: CatalogueResults, filter: SearchFilter, skipArtist?: string, tap = false): Row[] {
  const artists = filter === "all" || filter === "artists" ? c.artists.filter((a) => a.mbid !== skipArtist).map(artistRow) : [];
  const albums = filter === "all" || filter === "albums" ? c.albums.map(albumRow) : [];
  const songs = filter === "all" || filter === "songs" ? c.songs.map((s) => songRow(s, tap)) : [];
  return [...artists, ...albums, ...songs];
}

/** Rows on their way: a couple of skeletons where the next kind will land. */
export function CatalogueSkeleton() {
  return (
    <div aria-hidden="true">
      {Array.from({ length: 2 }, (_, i) => (
        <div key={i} className="sr-row sr-row--skeleton">
          <Skeleton width={48} height={48} />
          <span className="sr-row__text">
            <Skeleton width={180} height={14} />
            <Skeleton width={110} height={12} />
          </span>
        </div>
      ))}
    </div>
  );
}

/** Catalogue rows as their own list (the Songs chip, and searches the library found nothing for). */
export function CatalogueList({ c, filter, lidarrReady, skipArtist, label = "Music you don't have" }: { c: CatalogueResults; filter: SearchFilter; lidarrReady: boolean; skipArtist?: string; label?: string }) {
  const tap = useTapPlays(lidarrReady);
  const rows = catalogueRows(c, filter, skipArtist, tap);
  return (
    <>
      {rows.length > 0 && (
        <ul className="sr-list" aria-label={label}>
          {rows.map((r) => (
            <CatalogueRow key={r.key} row={r} lidarrReady={lidarrReady} />
          ))}
        </ul>
      )}
      {c.pending.length > 0 && <CatalogueSkeleton />}
    </>
  );
}
