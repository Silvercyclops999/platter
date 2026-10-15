/**
 * Edit in Plex dialogs, mounted once in the shell: change cover or photo,
 * fix match, edit details, merge, split and recent changes. Which one is
 * open lives in stores/plexEdit.
 */
import { Check, ImagePlus, Lock, TriangleAlert } from "lucide-react";
import { useEffect, useId, useMemo, useRef, useState, type DragEvent } from "react";
import { backend } from "../lib/backend";
import { coverImage } from "../lib/coverImage";
import { timeAgo } from "../lib/relativeTime";
import type { ArtistName, EditChange, EditFields, EditInfo, ImageChoice, ImageKind, MatchCandidate } from "../lib/types";
import { editFailed, ensureLoaded, errText, runEdit, undoChange, usePlexEdit, type EditTarget } from "../stores/plexEdit";
import { useSettings } from "../stores/settings";
import { Art } from "./Art";
import { Button } from "./Button";
import { Dialog } from "./Dialog";
import { Skeleton } from "./Skeleton";
import "./plex-edit.css";

export function PlexEditDialogs() {
  const d = usePlexEdit((s) => s.dialog);
  // Menus are built on demand, so what's editable is known before the first one opens.
  useEffect(() => ensureLoaded(), []);
  if (!d) return null;
  const close = () => usePlexEdit.getState().close();
  const key = `${d.name}:${d.target.id}`;
  switch (d.name) {
    case "cover":
      return <ImageDialog key={key} target={d.target} which="poster" onClose={close} />;
    case "background":
      return <ImageDialog key={key} target={d.target} which="art" onClose={close} />;
    case "match":
      return <MatchDialog key={key} target={d.target} onClose={close} />;
    case "details":
      return <DetailsDialog key={key} target={d.target} onClose={close} />;
    case "merge":
      return <MergeDialog key={key} target={d.target} onClose={close} />;
    case "split":
      return <SplitDialog key={key} target={d.target} onClose={close} />;
    case "history":
      return <HistoryDialog key={key} target={d.target} onClose={close} />;
  }
}

/** Loads `fn` once; data, error and a reload. */
function useOnce<T>(fn: () => Promise<T>): { data: T | null; error: string | null } {
  const [state, setState] = useState<{ data: T | null; error: string | null }>({ data: null, error: null });
  useEffect(() => {
    let live = true;
    fn().then(
      (data) => live && setState({ data, error: null }),
      (e) => live && setState({ data: null, error: errText(e) || "Couldn't load from Plex." }),
    );
    return () => {
      live = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  return state;
}

const PROVIDER: Record<string, string> = {
  embedded: "From the files",
  local: "From the folder",
  upload: "Uploaded",
  caa: "Cover Art Archive",
  fanarttv: "fanart.tv",
  lastfm: "Last.fm",
  discogs: "Discogs",
  tivo: "TiVo",
};

function providerLabel(c: ImageChoice): string {
  if (c.key.startsWith("upload://")) return "Uploaded";
  return (c.provider && PROVIDER[c.provider]) || c.provider || "Plex";
}

// ---------------------------------------------------------------- images

type Pick = { kind: "choice"; key: string } | { kind: "file"; bytes: Uint8Array; type: string; url: string } | { kind: "link" };

function ImageDialog({ target, which, onClose }: { target: EditTarget; which: ImageKind; onClose: () => void }) {
  const id = useId();
  const noun = target.kind === "album" ? "cover" : which === "art" ? "background" : "photo";
  const title = `Change ${noun}`;
  const { data: choices, error } = useOnce(() => backend.plexEditImages(target.id, which));
  const [pick, setPick] = useState<Pick | null>(null);
  const [link, setLink] = useState("");
  const [problem, setProblem] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [dragging, setDragging] = useState(false);
  const fileRef = useRef<HTMLInputElement>(null);
  const round = target.kind === "artist" && which === "poster";

  useEffect(
    () => () => {
      if (pick?.kind === "file") URL.revokeObjectURL(pick.url);
    },
    [pick],
  );

  const takeFile = async (file: File | undefined) => {
    if (!file) return;
    try {
      const img = await coverImage(file);
      setPick({ kind: "file", ...img, url: URL.createObjectURL(new Blob([img.bytes as BlobPart], { type: img.type })) });
      setProblem(null);
    } catch (e) {
      setProblem(e instanceof Error ? e.message : "Couldn't read that image.");
    }
  };

  const onDrop = (e: DragEvent) => {
    e.preventDefault();
    setDragging(false);
    void takeFile(e.dataTransfer.files?.[0]);
  };

  const save = async () => {
    if (!pick) return;
    if (pick.kind === "link" && !/^https?:\/\/\S+/i.test(link.trim())) {
      setProblem("Paste a link that starts with https://.");
      return;
    }
    setSaving(true);
    const art = { path: target.thumb, kind: target.kind };
    const done = await runEdit(
      `Couldn't change the ${noun}`,
      () =>
        pick.kind === "choice"
          ? backend.plexEditSelectImage(target.id, target.kind, which, pick.key)
          : pick.kind === "file"
            ? backend.plexEditUploadImage(target.id, target.kind, which, pick.bytes, pick.type)
            : backend.plexEditImageLink(target.id, target.kind, which, link.trim()),
      art,
    );
    setSaving(false);
    if (done) onClose();
  };

  const current = choices?.find((c) => c.selected);
  const pickedKey = pick?.kind === "choice" ? pick.key : null;
  const changed = !!pick && !(pick.kind === "choice" && pick.key === current?.key);

  return (
    <Dialog open onClose={onClose} label={title} className="pe pe--wide">
      <div
        className={`pe__drop ${dragging ? "is-dragging" : ""}`}
        onDragOver={(e) => {
          e.preventDefault();
          setDragging(true);
        }}
        onDragLeave={() => setDragging(false)}
        onDrop={onDrop}
      >
        <h2 className="pe__title">{title}</h2>
        <p className="pe__sub">
          {target.title}
          {target.artistTitle ? ` · ${target.artistTitle}` : ""}
        </p>
        {error ? (
          <p className="pe__empty" role="alert">
            {error}
          </p>
        ) : (
          <div className={`pe-grid ${which === "art" ? "pe-grid--wide" : ""}`} role="radiogroup" aria-labelledby={`${id}-suggest`}>
            <span id={`${id}-suggest`} className="sr-only">
              Suggestions from Plex
            </span>
            <button
              type="button"
              className={`pe-tile pe-tile--upload ${pick?.kind === "file" ? "is-picked" : ""}`}
              role="radio"
              aria-checked={pick?.kind === "file"}
              onClick={() => fileRef.current?.click()}
            >
              <span className={`pe-tile__art ${round ? "pe-tile__art--round" : ""}`}>
                {pick?.kind === "file" ? (
                  <img src={pick.url} alt="" />
                ) : (
                  <span className="pe-tile__upload">
                    <ImagePlus size={28} aria-hidden="true" />
                  </span>
                )}
                {pick?.kind === "file" && <PickedMark />}
              </span>
              <span className="pe-tile__label">Upload an image</span>
              <span className="pe-tile__note">{dragging ? "Drop it here" : "Or drag one here"}</span>
            </button>
            {choices
              ? choices.map((c) => (
                  <button
                    key={c.key}
                    type="button"
                    role="radio"
                    aria-checked={pickedKey === c.key || (!pick && c.selected)}
                    className={`pe-tile ${pickedKey === c.key || (!pick && c.selected) ? "is-picked" : ""}`}
                    onClick={() => setPick({ kind: "choice", key: c.key })}
                  >
                    <span className={`pe-tile__art ${round ? "pe-tile__art--round" : ""}`}>
                      <Art path={c.preview} fluid requestSize={160} kind={target.kind} round={round} />
                      {(pickedKey === c.key || (!pick && c.selected)) && <PickedMark />}
                    </span>
                    <span className="pe-tile__label">{providerLabel(c)}</span>
                    <span className="pe-tile__note">{c.selected ? "In use now" : " "}</span>
                  </button>
                ))
              : Array.from({ length: 5 }, (_, i) => (
                  <span key={i} className="pe-tile" aria-hidden="true">
                    <Skeleton width="100%" height={which === "art" ? 90 : 140} />
                  </span>
                ))}
          </div>
        )}
        <input
          ref={fileRef}
          type="file"
          accept="image/jpeg,image/png,image/webp"
          hidden
          data-testid="pe-file"
          onChange={(e) => void takeFile(e.target.files?.[0])}
        />
        <div className="pe__link">
          <label className="pe-field pe-field--grow" htmlFor={`${id}-link`}>
            <span className="pe-field__label">Use an image link</span>
            <input
              id={`${id}-link`}
              className="pe-input"
              placeholder="https://"
              inputMode="url"
              value={link}
              onChange={(e) => {
                setLink(e.target.value);
                setPick(e.target.value.trim() ? { kind: "link" } : null);
                setProblem(null);
              }}
            />
          </label>
        </div>
        {problem && (
          <p className="confirm__error" role="alert">
            {problem}
          </p>
        )}
        <div className="confirm__actions">
          <Button variant="ghost" onClick={onClose}>
            Cancel
          </Button>
          <Button variant="primary" disabled={!changed || saving} onClick={() => void save()}>
            Save
          </Button>
        </div>
      </div>
    </Dialog>
  );
}

function PickedMark() {
  return (
    <span className="pe-tile__check" aria-hidden="true">
      <Check size={14} strokeWidth={3} />
    </span>
  );
}

// ---------------------------------------------------------------- match

function MatchDialog({ target, onClose }: { target: EditTarget; onClose: () => void }) {
  const id = useId();
  const { data: info, error } = useOnce(() => backend.plexEditItem(target.id));
  const [title, setTitle] = useState(target.title);
  const [year, setYear] = useState("");
  const [results, setResults] = useState<MatchCandidate[] | null>(null);
  const [searching, setSearching] = useState(false);
  const [problem, setProblem] = useState<string | null>(null);
  const [chosen, setChosen] = useState<MatchCandidate | null>(null);
  const [saving, setSaving] = useState(false);

  const search = async (t: string, y: string) => {
    setSearching(true);
    setProblem(null);
    const yn = /^\d{4}$/.test(y.trim()) ? Number(y.trim()) : null;
    try {
      const r = await backend.plexEditMatches(target.id, t.trim() || undefined, yn);
      setResults(r);
      setChosen(null);
    } catch (e) {
      setProblem(errText(e) || "Couldn't search.");
      setResults([]);
    }
    setSearching(false);
  };

  // Prefilled with the title and year, and searched once.
  useEffect(() => {
    if (!info) return;
    const y = target.kind === "album" ? info.fields.released.slice(0, 4) : "";
    setTitle(info.fields.title);
    setYear(y);
    void search(info.fields.title, y);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [info]);

  const save = async () => {
    if (!chosen) return;
    setSaving(true);
    const done = await runEdit("Couldn't fix the match", () => backend.plexEditMatch(target.id, target.kind, chosen), {
      path: target.thumb,
      kind: target.kind,
    });
    setSaving(false);
    if (done) onClose();
  };

  const unmatch = async () => {
    setSaving(true);
    const done = await runEdit("Couldn't unmatch", () => backend.plexEditUnmatch(target.id, target.kind), { path: target.thumb, kind: target.kind });
    setSaving(false);
    if (done) onClose();
  };

  return (
    <Dialog open onClose={onClose} label="Fix match" className="pe pe--wide">
      <h2 className="pe__title">Fix match</h2>
      <p className="pe__sub">
        {target.title}
        {target.artistTitle ? ` · ${target.artistTitle}` : ""}
      </p>
      {info?.unreleased && (
        <div className="pe-warn" role="note">
          <TriangleAlert size={18} aria-hidden="true" className="pe-warn__icon" />
          <div className="pe-warn__text">
            <p className="pe-warn__title">This album counts as unreleased</p>
            <p>
              Matching it to an online entry can replace its title, cover and details with those of a different, released album.
              {info.matched ? " Unmatching keeps the details from its own tags." : " Leaving it unmatched keeps the details from its own tags."}
            </p>
          </div>
          {info.matched && (
            <Button size="sm" variant="secondary" disabled={saving} onClick={() => void unmatch()}>
              Unmatch instead
            </Button>
          )}
        </div>
      )}
      <form
        className="pe-search"
        onSubmit={(e) => {
          e.preventDefault();
          void search(title, year);
        }}
      >
        <label className="pe-field pe-field--grow" htmlFor={`${id}-t`}>
          <span className="pe-field__label">{target.kind === "album" ? "Album title" : "Artist name"}</span>
          <input id={`${id}-t`} className="pe-input" value={title} onChange={(e) => setTitle(e.target.value)} />
        </label>
        {target.kind === "album" && (
          <label className="pe-field pe-field--year" htmlFor={`${id}-y`}>
            <span className="pe-field__label">Year</span>
            <input id={`${id}-y`} className="pe-input" inputMode="numeric" maxLength={4} value={year} onChange={(e) => setYear(e.target.value)} />
          </label>
        )}
        <Button type="submit" variant="secondary" disabled={searching}>
          Search
        </Button>
      </form>
      <div className="pe-results" role="radiogroup" aria-label="Matches" aria-busy={searching || (!results && !error)}>
        {error ? (
          <p className="pe__empty" role="alert">
            {error}
          </p>
        ) : !results || searching ? (
          Array.from({ length: 3 }, (_, i) => (
            <div key={i} className="pe-result pe-result--skeleton" aria-hidden="true">
              <Skeleton width="50%" height={14} />
              <Skeleton width="30%" height={12} />
            </div>
          ))
        ) : results.length === 0 ? (
          <p className="pe__empty">No matches. Try a different title or year.</p>
        ) : (
          results.map((r) => (
            <button
              key={r.guid}
              type="button"
              role="radio"
              aria-checked={chosen?.guid === r.guid}
              className={`pe-result ${chosen?.guid === r.guid ? "is-picked" : ""}`}
              onClick={() => setChosen(r)}
            >
              <span className="pe-result__text">
                <span className="pe-result__name truncate">{r.name}</span>
                <span className="pe-result__meta truncate">{[r.artist, r.year].filter(Boolean).join(" · ") || " "}</span>
              </span>
              {r.score !== null && <span className="pe-result__score">{r.score}% match</span>}
              {chosen?.guid === r.guid && <Check size={16} aria-hidden="true" className="pe-result__check" />}
            </button>
          ))
        )}
      </div>
      {problem && (
        <p className="confirm__error" role="alert">
          {problem}
        </p>
      )}
      <div className="confirm__actions">
        {info?.matched && !info.unreleased && (
          <Button variant="ghost" className="pe__left" disabled={saving} onClick={() => void unmatch()}>
            Unmatch
          </Button>
        )}
        <Button variant="ghost" onClick={onClose}>
          Cancel
        </Button>
        <Button variant="primary" disabled={!chosen || saving} onClick={() => void save()}>
          Match
        </Button>
      </div>
    </Dialog>
  );
}

// ---------------------------------------------------------------- details

const SPLIT = /\s*[,;]\s*/;
const listText = (l: string[]) => l.join(", ");
const textList = (s: string) => s.split(SPLIT).map((x) => x.trim()).filter(Boolean);

/** Plex's name for each field, for its lock. */
const LOCK_NAME: Record<keyof EditFields, string> = {
  title: "title",
  sortTitle: "titleSort",
  released: "originallyAvailableAt",
  studio: "studio",
  summary: "summary",
  genres: "genre",
  styles: "style",
  moods: "mood",
};

function DetailsDialog({ target, onClose }: { target: EditTarget; onClose: () => void }) {
  const { data: info, error } = useOnce(() => backend.plexEditItem(target.id));
  return (
    <Dialog open onClose={onClose} label="Edit details" className="pe pe--wide">
      <h2 className="pe__title">Edit details</h2>
      {error ? (
        <>
          <p className="pe__empty" role="alert">
            {error}
          </p>
          <div className="confirm__actions">
            <Button variant="primary" onClick={onClose}>
              Close
            </Button>
          </div>
        </>
      ) : info ? (
        <DetailsForm target={target} info={info} onClose={onClose} />
      ) : (
        <div className="pe-form" aria-busy="true">
          {Array.from({ length: 5 }, (_, i) => (
            <Skeleton key={i} width="100%" height={40} />
          ))}
        </div>
      )}
    </Dialog>
  );
}

function DetailsForm({ target, info, onClose }: { target: EditTarget; info: EditInfo; onClose: () => void }) {
  const id = useId();
  const lockOn = useSettings((s) => s.settings?.plexEditLock !== false);
  const album = target.kind === "album";
  const [f, setF] = useState({
    ...info.fields,
    genresText: listText(info.fields.genres),
    stylesText: listText(info.fields.styles),
    moodsText: listText(info.fields.moods),
  });
  const [problem, setProblem] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const titleRef = useRef<HTMLInputElement>(null);

  const next: EditFields = useMemo(
    () => ({
      title: f.title,
      sortTitle: f.sortTitle,
      released: f.released,
      studio: f.studio,
      summary: f.summary,
      genres: textList(f.genresText),
      styles: textList(f.stylesText),
      moods: textList(f.moodsText),
    }),
    [f],
  );
  const dirty = (Object.keys(next) as (keyof EditFields)[]).some((k) => JSON.stringify(next[k]) !== JSON.stringify(info.fields[k]));

  const save = async () => {
    if (!next.title.trim()) {
      setProblem(album ? "The title can't be empty." : "The name can't be empty.");
      titleRef.current?.focus();
      return;
    }
    if (album && next.released.trim() && !/^\d{4}(-\d{2}-\d{2})?$/.test(next.released.trim())) {
      setProblem("Use a year like 2021 or a date like 2021-06-25.");
      return;
    }
    setSaving(true);
    const done = await runEdit("Couldn't save the details", () => backend.plexEditDetails(target.id, next), { path: target.thumb, kind: target.kind });
    setSaving(false);
    if (done) onClose();
  };

  const locked = (k: keyof EditFields) => info.locked.includes(LOCK_NAME[k]);
  const field = (k: keyof EditFields, label: string, value: string, set: (v: string) => void, opts: { area?: boolean; hint?: string; ref?: React.Ref<HTMLInputElement> } = {}) => (
    <label className={`pe-field ${opts.area ? "pe-field--area" : ""}`} htmlFor={`${id}-${k}`}>
      <span className="pe-field__label">
        {label}
        {locked(k) && (
          <span className="pe-lock" title="Locked: a refresh keeps this">
            <Lock size={11} aria-hidden="true" />
            <span className="sr-only">(locked)</span>
          </span>
        )}
      </span>
      {opts.area ? (
        <textarea id={`${id}-${k}`} className="pe-input pe-input--area" value={value} onChange={(e) => set(e.target.value)} />
      ) : (
        <input ref={opts.ref} id={`${id}-${k}`} className="pe-input" value={value} placeholder={opts.hint} onChange={(e) => set(e.target.value)} />
      )}
    </label>
  );

  return (
    <form
      onSubmit={(e) => {
        e.preventDefault();
        void save();
      }}
    >
      <div className="pe-form">
        <div className="pe-form__row">
          {field("title", album ? "Title" : "Name", f.title, (v) => setF({ ...f, title: v }), { ref: titleRef })}
          {field("sortTitle", album ? "Sort title" : "Sort name", f.sortTitle, (v) => setF({ ...f, sortTitle: v }), { hint: "Same as the title" })}
        </div>
        {album && (
          <div className="pe-form__row">
            {field("released", "Release date", f.released, (v) => setF({ ...f, released: v }), { hint: "2021 or 2021-06-25" })}
            {field("studio", "Record label", f.studio, (v) => setF({ ...f, studio: v }))}
          </div>
        )}
        {field("genres", "Genres", f.genresText, (v) => setF({ ...f, genresText: v }), { hint: "Separate with commas" })}
        <div className="pe-form__row">
          {field("styles", "Styles", f.stylesText, (v) => setF({ ...f, stylesText: v }), { hint: "Separate with commas" })}
          {field("moods", "Moods", f.moodsText, (v) => setF({ ...f, moodsText: v }), { hint: "Separate with commas" })}
        </div>
        {field("summary", album ? "Review" : "Biography", f.summary, (v) => setF({ ...f, summary: v }), { area: true })}
      </div>
      <p className="pe__note">
        {album ? "The album artist and song artists come from the files' tags, so they're changed there. " : ""}
        {lockOn ? "Fields you change are locked, so refreshing from Plex keeps them." : "Fields you change aren't locked, so a refresh can replace them."}
      </p>
      {problem && (
        <p className="confirm__error" role="alert">
          {problem}
        </p>
      )}
      <div className="confirm__actions">
        <Button variant="ghost" onClick={onClose}>
          Cancel
        </Button>
        <Button variant="primary" type="submit" disabled={!dirty || saving}>
          Save
        </Button>
      </div>
    </form>
  );
}

// ---------------------------------------------------------------- merge

function MergeDialog({ target, onClose }: { target: EditTarget; onClose: () => void }) {
  const id = useId();
  const { data: names } = useOnce(() => backend.artistNames());
  const [query, setQuery] = useState("");
  const [other, setOther] = useState<ArtistName | null>(null);
  const [nameFrom, setNameFrom] = useState<"this" | "other">("this");
  const [photoFrom, setPhotoFrom] = useState<"this" | "other">("this");
  const [otherThumb, setOtherThumb] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const findRef = useRef<HTMLInputElement>(null);

  const found = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!names || !q) return [];
    return names.filter((n) => n.id !== target.id && n.title.toLowerCase().includes(q)).slice(0, 8);
  }, [names, query, target.id]);

  useEffect(() => {
    if (!other) return;
    let live = true;
    backend.artistPage(other.id).then(
      (p) => live && setOtherThumb(p.artist.thumb),
      () => {},
    );
    return () => {
      live = false;
    };
  }, [other]);

  const merge = async () => {
    if (!other) return;
    const keep = nameFrom === "this" ? target.id : other.id;
    const drop = nameFrom === "this" ? other.id : target.id;
    const photo = photoFrom === "this" ? target.id : other.id;
    setSaving(true);
    const done = await runEdit("Couldn't merge", () => backend.plexEditMerge(keep, [drop], photo), { path: target.thumb, kind: "artist" });
    setSaving(false);
    if (done) onClose();
  };

  const side = (who: "this" | "other") => {
    const name = who === "this" ? target.title : other!.title;
    const thumb = who === "this" ? target.thumb : otherThumb;
    return (
      <div className="pe-merge__side">
        <Art path={thumb} size={96} round kind="artist" />
        <span className="pe-merge__name truncate">{name}</span>
        <label className="pe-radio">
          <input type="radio" name={`${id}-name`} checked={nameFrom === who} onChange={() => setNameFrom(who)} />
          <span>Keep this name</span>
        </label>
        <label className="pe-radio">
          <input type="radio" name={`${id}-photo`} checked={photoFrom === who} onChange={() => setPhotoFrom(who)} />
          <span>Keep this photo</span>
        </label>
      </div>
    );
  };

  return (
    <Dialog open onClose={onClose} label={`Merge ${target.title}`} className="pe" initialFocus={findRef}>
      <h2 className="pe__title">Merge with another artist</h2>
      <p className="pe__sub">Their albums and songs move under one artist in Plex. You can split them again later.</p>
      {!other ? (
        <>
          <label className="pe-field" htmlFor={`${id}-find`}>
            <span className="pe-field__label">Find an artist</span>
            <input ref={findRef} id={`${id}-find`} className="pe-input" value={query} onChange={(e) => setQuery(e.target.value)} autoComplete="off" />
          </label>
          <div className="pe-results" role="listbox" aria-label="Artists">
            {!names ? (
              <Skeleton width="100%" height={40} />
            ) : query.trim() && found.length === 0 ? (
              <p className="pe__empty">No artist by that name in your library.</p>
            ) : (
              found.map((n) => (
                <button key={n.id} type="button" role="option" aria-selected={false} className="pe-result" onClick={() => setOther(n)}>
                  <span className="pe-result__text">
                    <span className="pe-result__name truncate">{n.title}</span>
                  </span>
                </button>
              ))
            )}
          </div>
          <div className="confirm__actions">
            <Button variant="ghost" onClick={onClose}>
              Cancel
            </Button>
          </div>
        </>
      ) : (
        <>
          <div className="pe-merge">
            {side("this")}
            {side("other")}
          </div>
          <div className="confirm__actions">
            <Button variant="ghost" className="pe__left" onClick={() => setOther(null)}>
              Pick another
            </Button>
            <Button variant="ghost" onClick={onClose}>
              Cancel
            </Button>
            <Button variant="primary" disabled={saving} onClick={() => void merge()}>
              Merge
            </Button>
          </div>
        </>
      )}
    </Dialog>
  );
}

// ---------------------------------------------------------------- split

function SplitDialog({ target, onClose }: { target: EditTarget; onClose: () => void }) {
  const [saving, setSaving] = useState(false);
  const split = async () => {
    setSaving(true);
    const done = await runEdit("Couldn't split", () => backend.plexEditSplit(target.id), { path: target.thumb, kind: "artist" });
    setSaving(false);
    if (done) onClose();
  };
  return (
    <Dialog open onClose={onClose} label={`Split ${target.title}?`} className="confirm">
      <h2 className="confirm__title">Split this artist?</h2>
      <p className="confirm__body">
        Plex separates the artists it merged into "{target.title}". This can't be undone from here, but you can merge them again.
      </p>
      <div className="confirm__actions">
        <Button variant="ghost" onClick={onClose}>
          Cancel
        </Button>
        <Button variant="primary" disabled={saving} onClick={() => void split()}>
          Split
        </Button>
      </div>
    </Dialog>
  );
}

// ---------------------------------------------------------------- recent changes

function HistoryDialog({ target, onClose }: { target: EditTarget; onClose: () => void }) {
  const [list, setList] = useState<EditChange[] | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const load = () =>
    backend.plexEditHistory(target.id).then(setList, (e) => {
      editFailed("Couldn't load recent changes", e);
      setList([]);
    });
  useEffect(() => {
    void load();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [target.id]);

  const undo = async (c: EditChange) => {
    setBusy(c.id);
    await undoChange(c.id);
    await load();
    setBusy(null);
  };

  return (
    <Dialog open onClose={onClose} label="Recent changes" className="pe">
      <h2 className="pe__title">Recent changes</h2>
      <p className="pe__sub">{target.title}</p>
      <ul className="pe-history" aria-busy={!list}>
        {!list ? (
          <li>
            <Skeleton width="100%" height={40} />
          </li>
        ) : list.length === 0 ? (
          <li className="pe__empty">No changes made in Platter yet.</li>
        ) : (
          list.map((c) => (
            <li key={c.id} className="pe-history__row">
              <span className="pe-history__text">
                <span className={`pe-history__label ${c.undone ? "is-undone" : ""}`}>{c.label}</span>
                <span className="pe-history__when">{timeAgo(c.at / 1000)}</span>
              </span>
              {c.undone ? (
                <span className="pe-history__state">Undone</span>
              ) : c.undoable ? (
                <Button size="sm" variant="secondary" disabled={busy !== null} onClick={() => void undo(c)}>
                  Undo
                </Button>
              ) : null}
            </li>
          ))
        )}
      </ul>
      <div className="confirm__actions">
        <Button variant="primary" onClick={onClose}>
          Done
        </Button>
      </div>
    </Dialog>
  );
}
