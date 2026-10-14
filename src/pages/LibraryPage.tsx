import { Disc3, ListMusic, User } from "lucide-react";
import { useCallback, useEffect, useRef, useState, type ReactNode } from "react";
import { CardSkeleton } from "../components/Card";
import { FilterChips, Tabs } from "../components/Tabs";
import { VirtualGrid } from "../components/VirtualGrid";
import { backend } from "../lib/backend";
import { SECTION_LABELS, plural } from "../lib/format";
import type { Album, AlbumSort, Artist, ArtistSort, ReleaseSection } from "../lib/types";
import { useLoad } from "../lib/useLoad";
import { useApp } from "../stores/app";
import { useRouter, type LibraryTab } from "../stores/router";
import { AlbumCard, ArtistCard, PlaylistCard } from "./cards";

const PAGE = 120;

/** Shown in place of an empty grid. */
export function EmptyLibrary({ icon, title, text }: { icon: ReactNode; title: string; text: string }) {
  return (
    <div className="empty-state" role="status">
      {icon}
      <h2 className="empty-state__title">{title}</h2>
      <p className="empty-state__text">{text}</p>
    </div>
  );
}

/**
 * Lazily paged list: fetches `PAGE`-sized pages as the grid asks for ranges.
 * Resets when the query (sort/filter) or the library changes.
 */
function usePaged<T>(fetchPage: (offset: number, limit: number) => Promise<{ items: T[]; total: number }>, key: string) {
  const libraryVersion = useApp((s) => s.libraryVersion);
  const [total, setTotal] = useState<number | null>(null);
  const [items, setItems] = useState<Map<number, T>>(new Map());
  const pending = useRef(new Set<number>());
  const gen = useRef(0);

  const load = useCallback(
    (page: number) => {
      if (pending.current.has(page)) return;
      pending.current.add(page);
      const g = gen.current;
      fetchPage(page * PAGE, PAGE).then(
        (res) => {
          if (g !== gen.current) return;
          setTotal(res.total);
          setItems((prev) => {
            const next = new Map(prev);
            res.items.forEach((it, i) => next.set(page * PAGE + i, it));
            return next;
          });
        },
        () => pending.current.delete(page),
      );
    },
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [key],
  );

  useEffect(() => {
    gen.current++;
    pending.current = new Set();
    setItems(new Map());
    setTotal(null);
    load(0);
  }, [key, libraryVersion, load]);

  const ensure = useCallback(
    (start: number, end: number) => {
      for (let p = Math.floor(start / PAGE); p <= Math.floor(Math.max(start, end - 1) / PAGE); p++) load(p);
    },
    [load],
  );

  return { total, items, ensure };
}

const ALBUM_SORTS: { id: AlbumSort; label: string }[] = [
  { id: "recent", label: "Recently added" },
  { id: "name", label: "Alphabetical" },
  { id: "artist", label: "Artist" },
  { id: "year", label: "Release year" },
  { id: "plays", label: "Most played" },
];

const SECTIONS: { id: ReleaseSection | "all"; label: string }[] = [
  { id: "all", label: "All" },
  ...(["albums", "singlesEps", "live", "compilations", "soundtracks", "remixes", "demos"] as ReleaseSection[]).map((s) => ({
    id: s,
    label: SECTION_LABELS[s],
  })),
];

function AlbumsGrid() {
  const [sort, setSort] = useState<AlbumSort>("recent");
  const [section, setSection] = useState<ReleaseSection | "all">("all");
  const { total, items, ensure } = usePaged<Album>(
    (offset, limit) => backend.albums({ sort, offset, limit, section: section === "all" ? undefined : section }),
    `${sort}|${section}`,
  );
  return (
    <>
      <div className="library__filters">
        <FilterChips label="Release type" items={SECTIONS} value={section} onChange={setSection} />
        <label className="library__sort">
          <span className="sr-only">Sort albums by</span>
          <select value={sort} onChange={(e) => setSort(e.target.value as AlbumSort)} aria-label="Sort albums by">
            {ALBUM_SORTS.map((s) => (
              <option key={s.id} value={s.id}>
                {s.label}
              </option>
            ))}
          </select>
        </label>
      </div>
      <p className="library__count" aria-live="polite">
        {total == null ? " " : plural(total, "album")}
      </p>
      {total === 0 ? (
        <EmptyLibrary
          icon={<Disc3 size={40} strokeWidth={1.5} aria-hidden="true" />}
          title={section === "all" ? "No albums yet" : `No ${SECTIONS.find((s) => s.id === section)!.label.toLowerCase()} yet`}
          text={section === "all" ? "Albums in your Plex music library show up here after the next sync." : "Try another release type, or All."}
        />
      ) : (
      <VirtualGrid
        label="Albums"
        count={total ?? 24}
        onRangeChange={ensure}
        renderItem={(i) => {
          const a = items.get(i);
          return a ? <AlbumCard album={a} /> : <CardSkeleton />;
        }}
      />
      )}
    </>
  );
}

function ArtistsGrid() {
  const [sort] = useState<ArtistSort>("name");
  const { total, items, ensure } = usePaged<Artist>((offset, limit) => backend.artists({ sort, offset, limit }), sort);
  return (
    <>
      <p className="library__count">{total == null ? " " : plural(total, "artist")}</p>
      {total === 0 ? (
        <EmptyLibrary
          icon={<User size={40} strokeWidth={1.5} aria-hidden="true" />}
          title="No artists yet"
          text="Artists in your Plex music library show up here after the next sync."
        />
      ) : (
      <VirtualGrid
        label="Artists"
        count={total ?? 12}
        onRangeChange={ensure}
        renderItem={(i) => {
          const a = items.get(i);
          return a ? <ArtistCard artist={a} /> : <CardSkeleton kind="artist" />;
        }}
      />
      )}
    </>
  );
}

function PlaylistsGrid() {
  const { data } = useLoad(() => backend.playlists(), []);
  return (
    <>
      <p className="library__count">{data ? plural(data.length, "playlist") : " "}</p>
      {data && data.length === 0 ? (
        <EmptyLibrary
          icon={<ListMusic size={40} strokeWidth={1.5} aria-hidden="true" />}
          title="No playlists yet"
          text="Playlists you make in Plex show up here."
        />
      ) : (
        <VirtualGrid
          label="Playlists"
          count={data?.length ?? 8}
          renderItem={(i) => (data ? <PlaylistCard playlist={data[i]} /> : <CardSkeleton kind="playlist" />)}
        />
      )}
    </>
  );
}

export function LibraryPage({ tab }: { tab: LibraryTab }) {
  return (
    <div className="page-pad library">
      <div className="library__head">
        <h1 className="page-title">Your Library</h1>
        <Tabs
          label="Library sections"
          idPrefix="library"
          items={[
            { id: "albums", label: "Albums" },
            { id: "artists", label: "Artists" },
            { id: "playlists", label: "Playlists" },
          ]}
          value={tab}
          onChange={(t) => useRouter.getState().replace({ name: "library", tab: t })}
        />
      </div>
      <div id="library-panel" role="tabpanel" aria-labelledby={`library-tab-${tab}`}>
        {tab === "albums" && <AlbumsGrid />}
        {tab === "artists" && <ArtistsGrid />}
        {tab === "playlists" && <PlaylistsGrid />}
      </div>
    </div>
  );
}
