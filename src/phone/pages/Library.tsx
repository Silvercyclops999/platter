/**
 * The library on the phone: one mixed list with four pills (Playlists,
 * Albums, Artists, Downloads), the current sort as a label (it opens a
 * sheet), a list or grid toggle, pinned items at the top, and small glyphs
 * for pinned and downloaded. Downloads also leads to managing them. While
 * the library doctor has something to fix, a quiet row at the end leads to it.
 */
import { ArrowUpDown, ChevronRight, LayoutGrid, List, Plus, X } from "lucide-react";
import { useMemo, useRef, useState } from "react";
import { Art } from "../../components/Art";
import { LikedCover } from "../../components/LikedCover";
import { KIND_LABEL, SORT_LABEL, useLibraryView, type LibEntry, type LibKind } from "../../stores/library";
import { openEntry, useLibraryLoader, useLibraryModel } from "../../shell/useLibrary";
import { useApp } from "../../stores/app";
import { useDoctorCount } from "../../stores/doctor";
import { useSettings } from "../../stores/settings";
import type { DownloadKind } from "../../lib/types";
import { RowGlyphs, tap } from "../bits";
import { letterOf, phoneRows, phoneSubtitle } from "../libraryRows";
import "./parity.css";
import { phone, usePhoneUi } from "../phoneUi";
import { nav } from "../../stores/router";
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
  const model = useLibraryModel("", false);
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
      <div className="ph-library__head">
        <h1 className="ph-title">Library</h1>
        <button type="button" className="ph-icon-btn" aria-label="Create" onClick={() => phone.push({ kind: "create" })}>
          <Plus size={24} aria-hidden="true" />
        </button>
      </div>
      <div className="ph-pills ph-pills--scroll" role="group" aria-label="Show">
        {model.kinds.map((k) => (
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
        {!crates && (
          <button
            type="button"
            className={`ph-chip ${model.downloaded.on ? "is-on" : ""}`}
            aria-pressed={model.downloaded.on}
            onClick={() => (tap("selection"), useLibraryView.getState().setDownloaded(!model.downloaded.on))}
          >
            Downloads
            {model.downloaded.on && <X size={14} aria-hidden="true" className="ph-chip__x" />}
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
        <button type="button" className="ph-icon-btn" aria-label={view === "list" ? "Show as a grid" : "Show as a list"} onClick={toggleView}>
          {view === "list" ? <LayoutGrid size={20} aria-hidden="true" /> : <List size={20} aria-hidden="true" />}
        </button>
      </div>
      {model.downloaded.on && (
        <ul className="ph-set-list ph-library__manage" aria-label="Downloads">
          <li>
            <button type="button" className="ph-set-link" onClick={() => (tap(), nav.go({ name: "downloads" }))}>
              <span className="ph-set-link__text">
                <span className="ph-set-link__title">Manage downloads</span>
                <span className="ph-set-link__sub">Storage, quality and Go offline</span>
              </span>
              <ChevronRight size={18} aria-hidden="true" className="ph-set-link__chev" />
            </button>
          </li>
        </ul>
      )}
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
              <LibRow key={`${e.kind}:${e.id}`} e={e} typed={model.filter !== "all"} letter={letterFor(e)} />
            ))}
          </ul>
          {indexed && <LetterIndex list={listRef} letters={[...firsts]} />}
        </div>
      ) : (
        <ul className={`ph-lib-grid ${model.filter === "artist" ? "ph-lib-grid--round" : ""}`} aria-label="Your library">
          {rows.map((e) => (
            <li key={`${e.kind}:${e.id}`}>
              <button type="button" className={`ph-card-tile ph-card-tile--fluid ${e.kind === "artist" ? "is-round" : ""}`} onClick={() => openEntry(e)}>
                {e.liked || e.bookmarks || e.stats ? (
                  <LikedCover size={160} icon={e.liked ? "heart" : e.bookmarks ? "bookmark" : "stats"} />
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
      {!model.loading && model.filter === "all" && !model.downloaded.on && <DoctorRow />}
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

/** The download a library item stands for: Liked Songs, an album or a playlist. */
export function downloadItemOf(e: Pick<LibEntry, "kind" | "id" | "liked">): { kind: DownloadKind; id: string } | undefined {
  if (e.liked) return { kind: "liked", id: "" };
  return e.kind === "album" || e.kind === "playlist" ? { kind: e.kind, id: e.id } : undefined;
}

function LibRow({ e, typed, letter }: { e: LibEntry; typed: boolean; letter?: string }) {
  const round = e.kind === "artist";
  return (
    <li data-letter={letter}>
      <button type="button" className="ph-lib-row" onClick={() => openEntry(e)} disabled={e.unavailable}>
        {e.liked || e.bookmarks || e.stats ? (
          <LikedCover size={56} icon={e.liked ? "heart" : e.bookmarks ? "bookmark" : "stats"} />
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
