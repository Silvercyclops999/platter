/**
 * The library on the phone: one mixed list with type pills, the current
 * sort as a label (it opens a sheet), a list or grid toggle, pinned items at
 * the top, and small glyphs for pinned and downloaded.
 */
import { ArrowUpDown, LayoutGrid, List, Plus, X } from "lucide-react";
import { useState } from "react";
import { Art } from "../../components/Art";
import { LikedCover } from "../../components/LikedCover";
import { KIND_LABEL, SORT_LABEL, subtitle, useLibraryView, type LibEntry, type LibKind } from "../../stores/library";
import { openEntry, useLibraryLoader, useLibraryModel } from "../../shell/useLibrary";
import { RowGlyphs } from "../bits";
import { phone, usePhoneUi } from "../phoneUi";
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
          <button key={k} type="button" className={`ph-chip ${model.filter === k && !crates ? "is-on" : ""}`} aria-pressed={model.filter === k && !crates} onClick={() => pick(k)}>
            {KIND_LABEL[k]}
            {model.filter === k && !crates && <X size={14} aria-hidden="true" className="ph-chip__x" />}
          </button>
        ))}
        {cratesOn && (
          <button type="button" className={`ph-chip ${crates ? "is-on" : ""}`} aria-pressed={crates} onClick={() => setCrates(!crates)}>
            Crates
          </button>
        )}
        {model.downloaded.available && !crates && (
          <button
            type="button"
            className={`ph-chip ${model.downloaded.on ? "is-on" : ""}`}
            aria-pressed={model.downloaded.on}
            onClick={() => useLibraryView.getState().setDownloaded(!model.downloaded.on)}
          >
            Downloaded
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

function LibraryList({ model, view, toggleView }: { model: ReturnType<typeof useLibraryModel>; view: PhoneLibView; toggleView: () => void }) {
  const rows = model.shown.filter((e) => !e.folder);
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
      {model.loading ? (
        <ul className="ph-lib-list" aria-busy="true">
          {Array.from({ length: 6 }, (_, i) => (
            <li key={i} className="ph-lib-row ph-lib-row--ghost" />
          ))}
        </ul>
      ) : view === "list" ? (
        <ul className="ph-lib-list" aria-label="Your library">
          {rows.map((e) => (
            <LibRow key={`${e.kind}:${e.id}`} e={e} typed={model.filter !== "all"} />
          ))}
        </ul>
      ) : (
        <ul className={`ph-lib-grid ${model.filter === "artist" ? "ph-lib-grid--round" : ""}`} aria-label="Your library">
          {rows.map((e) => (
            <li key={`${e.kind}:${e.id}`}>
              <button type="button" className={`ph-card-tile ph-card-tile--fluid ${e.kind === "artist" ? "is-round" : ""}`} onClick={() => openEntry(e)}>
                <Art path={e.thumb} fluid requestSize={200} round={e.kind === "artist"} kind={e.kind} monogram={e.kind === "artist" ? e.title : undefined} />
                <span className="ph-card-tile__title">{e.title}</span>
                <span className="ph-card-tile__sub">
                  <RowGlyphs pinned={e.pinned && !e.liked} downloaded={e.downloaded} />
                  {subtitle(e, model.filter !== "all")}
                </span>
              </button>
            </li>
          ))}
        </ul>
      )}
      {!model.loading && rows.length === 0 && (
        <div className="ph-empty">
          <p className="ph-empty__title">Nothing here yet</p>
          <p className="ph-empty__text">Records, artists and playlists you save show up here.</p>
        </div>
      )}
    </>
  );
}

function LibRow({ e, typed }: { e: LibEntry; typed: boolean }) {
  const round = e.kind === "artist";
  return (
    <li>
      <button type="button" className="ph-lib-row" onClick={() => openEntry(e)} disabled={e.unavailable}>
        {e.liked || e.bookmarks || e.stats ? (
          <LikedCover size={56} icon={e.liked ? "heart" : e.bookmarks ? "bookmark" : "stats"} />
        ) : (
          <Art path={e.thumb} size={56} round={round} kind={e.kind} monogram={round ? e.title : undefined} sleeve={e.kind === "album" ? { title: e.title, artist: e.creator } : undefined} />
        )}
        <span className="ph-row__text">
          <span className="ph-row__title">{e.title}</span>
          <span className="ph-row__sub">
            <RowGlyphs pinned={e.pinned && !e.liked} downloaded={e.downloaded} />
            {subtitle(e, typed)}
          </span>
        </span>
      </button>
    </li>
  );
}
