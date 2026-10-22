/**
 * The library on the phone: one mixed list with the type pills (Playlists,
 * Albums, Artists) after a round filter button, the current sort as a label
 * (it opens a sheet), a list or grid toggle, pinned items at the top, and
 * small glyphs for pinned and downloaded. Library is where downloads are
 * found: whatever is downloaded is a row here (a saved or downloaded mix
 * too, with the playlists). Downloaded is in the filter sheet; while it's
 * on, the row reads clear, Downloaded, then only Playlists and Albums, and
 * a type picked with it joins it in one pill. A slim strip over the title
 * counts the songs while they come in. Storage is in Settings. While the
 * library doctor has something to fix, a quiet row at the end leads to it.
 */
import { ArrowUpDown, ChevronRight, Disc3, LayoutGrid, List, Plus, SlidersHorizontal, X } from "lucide-react";
import { useMemo, useRef, useState } from "react";
import { Art } from "../../components/Art";
import { LikedCover } from "../../components/LikedCover";
import { KIND_LABEL, SORT_LABEL, useLibraryView, type LibEntry, type LibKind } from "../../stores/library";
import { openEntry, useLibraryLoader, useLibraryModel } from "../../shell/useLibrary";
import { useApp } from "../../stores/app";
import { useDoctorCount } from "../../stores/doctor";
import { useSettings } from "../../stores/settings";
import { useDownloads } from "../../stores/downloads";
import type { DownloadKind, MixSummary } from "../../lib/types";
import { MixArt } from "../../pages/MixCard";
import { RowGlyphs, tap } from "../bits";
import { letterOf, phoneRows, phoneSubtitle } from "../libraryRows";
import "./parity.css";
import { phone, usePhoneUi } from "../phoneUi";
import { nav } from "../../stores/router";
import { player } from "../../stores/player";
import { CratesBody } from "./Crates";

type PhoneLibView = "list" | "grid";
const VIEW_KEY = "platter.phone.libraryView";

function loadView(): PhoneLibView {
  try {
    return localStorage.getItem(VIEW_KEY) === "grid" ? "grid" : "list";
  } catch {
    return "list";
  }
}

export function PhoneLibrary() {
  useLibraryLoader();
  const model = useLibraryModel("", false, true);
  const [view, setView] = useState<PhoneLibView>(loadView);
  // Crates stay off the phone's Library for now: the owner found them
  // confusing there. They're still on the computer and in Settings.
  const cratesOn = false;
  const crates = usePhoneUi((u) => u.crates) && cratesOn;
  const setCrates = (on: boolean) => usePhoneUi.setState({ crates: on });
  const pick = (k: LibKind) => {
    setCrates(false);
    useLibraryView.getState().setFilter(model.filter === k && !crates ? "all" : k);
  };
  const dlOn = model.downloaded.on && !crates;
  // While Downloaded is on, only playlists and albums can be downloaded.
  const kinds = dlOn ? model.kinds.filter((k) => DOWNLOADED_KINDS.includes(k)) : model.kinds;
  const joined = dlOn && DOWNLOADED_KINDS.includes(model.filter as LibKind) ? (model.filter as LibKind) : null;
  const clearAll = () => {
    tap("selection");
    useLibraryView.getState().setDownloaded(false);
    useLibraryView.getState().setFilter("all");
  };
  const toggleView = () => {
    const next = view === "list" ? "grid" : "list";
    setView(next);
    try {
      localStorage.setItem(VIEW_KEY, next);
    } catch {
      // Kept for this visit only.
    }
  };
  return (
    <div className="ph-page ph-library">
      <DownloadingStrip />
      <div className="ph-library__head">
        <h1 className="ph-title">Library</h1>
        <button type="button" className="ph-icon-btn" aria-label="Create" onClick={() => phone.push({ kind: "create" })}>
          <Plus size={24} aria-hidden="true" />
        </button>
      </div>
      <div className="ph-pills ph-pills--scroll" role="group" aria-label="Show">
        {dlOn ? (
          <button type="button" className="ph-chip ph-chip--round" aria-label="Clear filters" onClick={clearAll}>
            <X size={16} aria-hidden="true" />
          </button>
        ) : (
          <button type="button" className="ph-chip ph-chip--round" aria-label="Filters" onClick={() => (tap("selection"), phone.push({ kind: "libraryFilter", kinds: model.kinds, filter: model.filter, downloaded: model.downloaded.on }))}>
            <SlidersHorizontal size={16} aria-hidden="true" />
          </button>
        )}
        {dlOn && joined ? (
          <span className="ph-chip-join" role="group" aria-label={`Downloaded ${KIND_LABEL[joined]}`}>
            <button type="button" className="ph-chip is-on" aria-pressed="true" onClick={() => (tap("selection"), useLibraryView.getState().setDownloaded(false))}>
              Downloaded
            </button>
            <button type="button" className="ph-chip is-on" aria-pressed="true" onClick={() => (tap("selection"), pick(joined))}>
              {KIND_LABEL[joined]}
            </button>
          </span>
        ) : (
          dlOn && (
            <button type="button" className="ph-chip is-on" aria-pressed="true" onClick={() => (tap("selection"), useLibraryView.getState().setDownloaded(false))}>
              Downloaded
            </button>
          )
        )}
        {!joined &&
          kinds.map((k) => (
            <button key={k} type="button" className={`ph-chip ${model.filter === k && !crates ? "is-on" : ""}`} aria-pressed={model.filter === k && !crates} onClick={() => (tap("selection"), pick(k))}>
              {KIND_LABEL[k]}
              {model.filter === k && !crates && <X size={14} aria-hidden="true" className="ph-chip__x" />}
            </button>
          ))}
        {cratesOn && (
          <button type="button" className={`ph-chip ${crates ? "is-on" : ""}`} aria-pressed={crates} onClick={() => (tap("selection"), setCrates(!crates))}>
            Crates
          </button>
        )}
      </div>
      {crates ? (
        <CratesBody />
      ) : (
        <LibraryList model={model} view={view} toggleView={toggleView} />
      )}
    </div>
  );
}

/** The types that can be downloaded, the only ones shown while Downloaded is on. */
export const DOWNLOADED_KINDS: readonly LibKind[] = ["playlist", "album"];

/** "12 of 40": songs in from the downloads under way, while one is actually coming. */
export function downloadingCount(s: { items: { state: string; total: number; done: number }[]; current?: unknown } | null | undefined): { done: number; total: number } | null {
  if (!s?.current) return null;
  const going = s.items.filter((i) => i.state === "queued" || i.state === "downloading");
  const total = going.reduce((n, i) => n + i.total, 0);
  if (!total) return null;
  return { done: Math.min(total, going.reduce((n, i) => n + i.done, 0)), total };
}

/** A slim strip over the title while songs are coming in: Downloading, N of M, and a thin bar. */
function DownloadingStrip() {
  const st = useDownloads((s) => s.status);
  const count = useMemo(() => downloadingCount(st), [st]);
  if (!count) return null;
  return (
    <div className="ph-dl-strip">
      <span className="ph-dl-strip__line">
        <span>Downloading</span>
        <span className="tabular">
          {count.done} of {count.total}
        </span>
      </span>
      <span className="ph-dl-strip__bar" aria-hidden="true">
        <span style={{ width: `${(count.done / count.total) * 100}%` }} />
      </span>
    </div>
  );
}

/** A long A to Z list gets the letter index down the right edge. */
const INDEX_FROM = 30;

function LibraryList({ model, view, toggleView }: { model: ReturnType<typeof useLibraryModel>; view: PhoneLibView; toggleView: () => void }) {
  const playlists = useApp((s) => s.playlists);
  const counts = useMemo(() => new Map((playlists ?? []).map((p) => [p.id, p.trackCount])), [playlists]);
  const rows = useMemo(() => phoneRows(model.shown, model.sort, counts), [model.shown, model.sort, counts]);
  const listRef = useRef<HTMLUListElement>(null);
  const indexed = model.sort === "alpha" && rows.length >= INDEX_FROM;
  const firsts = new Set<string>();
  const letterFor = (e: LibEntry) => {
    if (!indexed || e.liked) return undefined;
    const l = letterOf(e.title);
    if (firsts.has(l)) return undefined;
    firsts.add(l);
    return l;
  };
  return (
    <>
      <div className="ph-library__bar">
        <button type="button" className="ph-text-btn ph-library__sort" onClick={() => phone.push({ kind: "librarySort" })}>
          <ArrowUpDown size={16} aria-hidden="true" />
          {SORT_LABEL[model.sort]}
        </button>
        <span className="ph-library__end">
          {model.filter === "album" && !model.loading && model.shown.length > 1 && (
            <button
              type="button"
              className="ph-text-btn ph-library__shuffle"
              onClick={() => (tap(), void player.shuffleAlbums(model.shown.filter((e) => e.kind === "album").map((e) => e.id)))}
            >
              <Disc3 size={16} aria-hidden="true" />
              Shuffle albums
            </button>
          )}
          <button type="button" className="ph-icon-btn" aria-label={view === "list" ? "Show as a grid" : "Show as a list"} onClick={toggleView}>
            {view === "list" ? <LayoutGrid size={20} aria-hidden="true" /> : <List size={20} aria-hidden="true" />}
          </button>
        </span>
      </div>
      {model.loading ? (
        <ul className="ph-lib-list" aria-busy="true">
          {Array.from({ length: 6 }, (_, i) => (
            <li key={i} className="ph-lib-row ph-lib-row--ghost" />
          ))}
        </ul>
      ) : view === "list" ? (
        <div className={`ph-lib-index-wrap ${indexed ? "is-indexed" : ""}`}>
          <ul ref={listRef} className="ph-lib-list" aria-label="Your library">
            {rows.map((e) => (
              <LibRow key={rowKey(e)} e={e} typed={model.filter !== "all"} letter={letterFor(e)} />
            ))}
          </ul>
          {indexed && <LetterIndex list={listRef} letters={[...firsts]} />}
        </div>
      ) : (
        <ul className={`ph-lib-grid ${model.filter === "artist" ? "ph-lib-grid--round" : ""}`} aria-label="Your library">
          {rows.map((e) => (
            <li key={rowKey(e)}>
              <button type="button" className={`ph-card-tile ph-card-tile--fluid ${e.kind === "artist" ? "is-round" : ""}`} onClick={() => openEntry(e)}>
                {e.liked || e.bookmarks || e.stats ? (
                  <LikedCover fluid icon={e.liked ? "heart" : e.bookmarks ? "bookmark" : "stats"} />
                ) : e.mix ? (
                  <MixRowSleeve e={e} fluid />
                ) : (
                  <Art
                    path={e.thumb}
                    fluid
                    requestSize={200}
                    tint={e.tint}
                    round={e.kind === "artist"}
                    kind={e.kind}
                    monogram={e.kind === "artist" ? e.title : undefined}
                    sleeve={e.kind === "album" ? { title: e.title, artist: e.creator ?? "" } : undefined}
                  />
                )}
                <span className="ph-card-tile__title">{e.title}</span>
                <span className="ph-card-tile__sub">
                  <RowGlyphs pinned={e.pinned && !e.liked} downloaded={e.downloaded} item={downloadItemOf(e)} albumId={e.kind === "album" ? e.id : undefined} />
                  {phoneSubtitle(e, model.filter !== "all")}
                </span>
              </button>
            </li>
          ))}
        </ul>
      )}
      {!model.loading && rows.length > 0 && model.filter === "all" && !model.downloaded.on && <DoctorRow />}
      {!model.loading && rows.length === 0 && (
        <div className="ph-empty">
          <p className="ph-empty__title">{model.downloaded.on ? "Nothing downloaded yet" : "Nothing here yet"}</p>
          <p className="ph-empty__text">
            {model.downloaded.on ? "Download a record or playlist from its page to play it without a connection." : "Records, artists and playlists you save show up here."}
          </p>
        </div>
      )}
    </>
  );
}

/** The library doctor, at the end of the list while it has something to fix. */
function DoctorRow() {
  const on = useSettings((s) => s.settings?.libraryDoctor !== false);
  const count = useDoctorCount();
  if (!on || count <= 0) return null;
  return (
    <ul className="ph-set-list ph-library__doctor" aria-label="Library doctor">
      <li>
        <button type="button" className="ph-set-link" onClick={() => (tap(), nav.go({ name: "doctor" }))}>
          <span className="ph-set-link__text">
            <span className="ph-set-link__title">Library doctor</span>
            <span className="ph-set-link__sub">{count === 1 ? "1 thing to fix" : `${count} things to fix`}</span>
          </span>
          <ChevronRight size={18} aria-hidden="true" className="ph-set-link__chev" />
        </button>
      </li>
    </ul>
  );
}

/**
 * The A to Z index down the list's right edge: touch or drag along it and
 * the list jumps to that letter's first row.
 */
function LetterIndex({ list, letters }: { list: React.RefObject<HTMLUListElement | null>; letters: string[] }) {
  const last = useRef("");
  const go = (clientY: number, el: HTMLElement) => {
    const box = el.getBoundingClientRect();
    const i = Math.min(letters.length - 1, Math.max(0, Math.floor(((clientY - box.top) / box.height) * letters.length)));
    const l = letters[i];
    if (!l || l === last.current) return;
    last.current = l;
    tap();
    list.current?.querySelector<HTMLElement>(`[data-letter="${l}"]`)?.scrollIntoView({ block: "start" });
  };
  return (
    <div
      className="ph-lib-index"
      role="navigation"
      aria-label="Jump to a letter"
      onPointerDown={(ev) => {
        ev.currentTarget.setPointerCapture?.(ev.pointerId);
        last.current = "";
        go(ev.clientY, ev.currentTarget);
      }}
      onPointerMove={(ev) => ev.buttons && go(ev.clientY, ev.currentTarget)}
    >
      {letters.map((l) => (
        <button
          key={l}
          type="button"
          className="ph-lib-index__letter"
          aria-label={`Jump to ${l}`}
          onClick={() => list.current?.querySelector<HTMLElement>(`[data-letter="${l}"]`)?.scrollIntoView({ block: "start" })}
        >
          {l}
        </button>
      ))}
    </div>
  );
}

/** A saved mix's cover, as its page shows it: a printed sleeve with its name, or the daylist's colours. */
export function MixRowSleeve({ e, fluid, card }: { e: Pick<LibEntry, "id" | "title" | "thumb">; fluid?: boolean; card?: boolean }) {
  const kind = e.id === "daylist" ? "daylist" : e.id.startsWith("topSongs:") ? "topSongs" : "daily";
  return (
    <span className={`ph-lib-mix ${fluid ? "is-fluid" : ""} ${card ? "is-card" : ""}`}>
      <MixArt mix={{ kind, title: e.title, description: "", color: "#5a5048", art: null, covers: e.thumb ? [e.thumb] : [] } as MixSummary} />
    </span>
  );
}

/** A row's key: a downloaded mix's id can't clash with a playlist's. */
const rowKey = (e: LibEntry) => `${e.mix ? "mix" : e.kind}:${e.id}`;

/** The download a library item stands for: Liked Songs, an album, a playlist or a downloaded mix. */
export function downloadItemOf(e: Pick<LibEntry, "kind" | "id" | "liked" | "mix">): { kind: DownloadKind; id: string } | undefined {
  if (e.liked) return { kind: "liked", id: "" };
  if (e.mix) return { kind: "mix", id: e.id };
  return e.kind === "album" || e.kind === "playlist" ? { kind: e.kind, id: e.id } : undefined;
}

function LibRow({ e, typed, letter }: { e: LibEntry; typed: boolean; letter?: string }) {
  const round = e.kind === "artist";
  return (
    <li data-letter={letter}>
      <button type="button" className="ph-lib-row" onClick={() => openEntry(e)} disabled={e.unavailable}>
        {e.liked || e.bookmarks || e.stats ? (
          <LikedCover size={56} icon={e.liked ? "heart" : e.bookmarks ? "bookmark" : "stats"} />
        ) : e.mix ? (
          <MixRowSleeve e={e} />
        ) : (
          <Art path={e.thumb} size={56} round={round} kind={e.kind} tint={e.tint} monogram={round ? e.title : undefined} sleeve={e.kind === "album" ? { title: e.title, artist: e.creator } : undefined} />
        )}
        <span className="ph-row__text">
          <span className="ph-row__title">{e.title}</span>
          <span className="ph-row__sub">
            <RowGlyphs pinned={e.pinned && !e.liked} downloaded={e.downloaded} item={downloadItemOf(e)} albumId={e.kind === "album" ? e.id : undefined} />
            {phoneSubtitle(e, typed)}
          </span>
        </span>
      </button>
    </li>
  );
}
