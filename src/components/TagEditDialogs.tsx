/**
 * The tag editor, the tag check and the one-click fixes. The only screens
 * that change the user's music files, so every save shows what it will
 * change first and can be undone.
 */
import { Check, CircleAlert, Info, TriangleAlert } from "lucide-react";
import { HelpButton } from "../help/HelpButton";
import { useEffect, useId, useMemo, useState } from "react";
import { backend } from "../lib/backend";
import {
  ALBUM_FIELDS,
  FIELD_LABEL,
  PRIMARY_TYPES,
  QUALITIES,
  SECONDARY_TYPES,
  asksFirst,
  buildRequest,
  fixRequest,
  joinReleaseType,
  shared,
  splitReleaseType,
} from "../lib/tagEdit";
import type { IsrcRelease, IsrcReleases, TagCheckReport, TagEditorData, TagField, TagFinding, TagFixPreview } from "../lib/types";
import { errText } from "../stores/plexEdit";
import { nav } from "../stores/router";
import { useSettings } from "../stores/settings";
import { saveTags, useTagEdit, type EditorTarget, type ExplicitTarget, type FixTarget } from "../stores/tagEdit";
import { toast } from "../stores/toasts";
import { Button } from "./Button";
import { Dialog } from "./Dialog";
import { Skeleton } from "./Skeleton";
import "./plex-edit.css";
import "./tag-edit.css";

export function TagEditDialogs() {
  const s = useTagEdit();
  return (
    <>
      {s.editor && <EditorDialog key={`${s.editor.albumId}:${s.editor.trackIds.join(",")}`} target={s.editor} onClose={() => s.close("editor")} />}
      {s.check && <CheckDialog key={JSON.stringify(s.check.scope)} scope={s.check.scope} title={s.check.title} onClose={() => s.close("check")} />}
      {s.fix && <FixDialog key={`${s.fix.kind}:${s.fix.albumId}`} target={s.fix} onClose={() => s.close("fix")} />}
      {s.explicit && <ExplicitDialog target={s.explicit} onClose={() => s.close("explicit")} />}
    </>
  );
}

/** Loads once; data or why not. */
function useLoad<T>(fn: () => Promise<T>, deps: unknown[] = []): { data: T | null; error: string | null; reload: () => void } {
  const [n, setN] = useState(0);
  const [state, setState] = useState<{ data: T | null; error: string | null }>({ data: null, error: null });
  useEffect(() => {
    let live = true;
    setState({ data: null, error: null });
    fn().then(
      (data) => live && setState({ data, error: null }),
      (e) => live && setState({ data: null, error: errText(e) || "Couldn't read the files." }),
    );
    return () => {
      live = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [n, ...deps]);
  return { ...state, reload: () => setN((x) => x + 1) };
}

function Loading() {
  return (
    <div className="pe-form" aria-busy="true">
      {Array.from({ length: 4 }, (_, i) => (
        <Skeleton key={i} width="100%" height={36} />
      ))}
    </div>
  );
}

function Problem({ text, onClose }: { text: string; onClose: () => void }) {
  return (
    <>
      <p className="pe__empty" role="alert">
        {text}
      </p>
      <div className="confirm__actions">
        <Button variant="primary" onClick={onClose}>
          Close
        </Button>
      </div>
    </>
  );
}

function goSetUp(onClose: () => void) {
  onClose();
  nav.go({ name: "settings", page: "tags" });
  toast.info("Set the music folder under Library, Tag editor");
}

/** "This changes 120 files." before a save that big, when Settings asks for it. */
function useConfirmMany() {
  const ask = useSettings((s) => s.settings?.askBeforeManyFiles);
  const [pending, setPending] = useState<number | null>(null);
  return {
    pending,
    clear: () => setPending(null),
    /** True when it may save now; else it asks first. */
    ok: (files: number) => {
      if (pending === files || !asksFirst(files, ask)) return true;
      setPending(files);
      return false;
    },
  };
}

function ConfirmMany({ n }: { n: number | null }) {
  return n ? (
    <p className="te-confirm" role="alert">
      This changes {n} files. Select Save again to change them all.
    </p>
  ) : null;
}

// ---------------------------------------------------------------- the editor

function EditorDialog({ target, onClose }: { target: EditorTarget; onClose: () => void }) {
  const { data, error } = useLoad(() => backend.tagEditOpen(target.albumId, target.trackIds));
  return (
    <Dialog open onClose={onClose} label="Edit tags" className="pe pe--wide te">
      <h2 className="pe__title">Edit tags{data ? `: ${data.title}` : ""}</h2>
      {error ? <Problem text={error} onClose={onClose} /> : data ? <EditorForm data={data} focus={target.focus ?? null} onClose={onClose} /> : <Loading />}
    </Dialog>
  );
}

type Edits = Partial<Record<TagField, string>>;

function EditorForm({ data, focus, onClose }: { data: TagEditorData; focus: TagField | null; onClose: () => void }) {
  const id = useId();
  const [album, setAlbum] = useState<Edits>({});
  const [tracks, setTracks] = useState<Record<string, Edits>>({});
  const [saving, setSaving] = useState(false);
  const [problem, setProblem] = useState<string | null>(null);
  const many = useConfirmMany();
  const readable = data.files.filter((f) => f.stamp);
  const viewOnly = !data.writable;
  const single = data.files.length === 1;
  const albumId = data.files[0]?.albumId;

  const request = useMemo(() => buildRequest(single ? `Edited tags on ${data.title}` : `Edited tags on ${data.title}`, data.files, album, tracks), [album, tracks, data, single]);

  const save = async () => {
    if (data.needsMusicFolder) return goSetUp(onClose);
    if (!request.files.length) return onClose();
    if (!many.ok(request.files.length)) return;
    setSaving(true);
    setProblem(null);
    const r = await saveTags(request);
    setSaving(false);
    if (!r) return;
    if (r.saved === 0 && r.problems.length) {
      setProblem(r.message);
      return;
    }
    onClose();
  };

  const albumValue = (f: TagField) => (f in album ? album[f]! : shared(data.files, f) ?? "");
  const several = (f: TagField) => !(f in album) && shared(data.files, f) === null;
  const setA = (f: TagField, v: string) => {
    many.clear();
    setAlbum((a) => ({ ...a, [f]: v }));
  };
  const trackValue = (trackId: string, f: TagField) => tracks[trackId]?.[f] ?? data.files.find((x) => x.trackId === trackId)?.values[f] ?? "";
  const setT = (trackId: string, f: TagField, v: string) => {
    many.clear();
    setTracks((t) => ({ ...t, [trackId]: { ...t[trackId], [f]: v } }));
  };

  const text = (f: TagField, hint?: string) => (
    <label className="pe-field" htmlFor={`${id}-${f}`} key={f}>
      <span className="pe-field__label">{FIELD_LABEL[f]}</span>
      <input
        id={`${id}-${f}`}
        className="pe-input"
        value={albumValue(f)}
        placeholder={several(f) ? "(several)" : hint}
        disabled={viewOnly}
        autoFocus={focus === f}
        onChange={(e) => setA(f, e.target.value)}
      />
    </label>
  );

  const choice = (f: TagField, options: readonly string[], labels: Record<string, string> = {}) => (
    <label className="pe-field" htmlFor={`${id}-${f}`} key={f}>
      <span className="pe-field__label">{FIELD_LABEL[f]}</span>
      <select id={`${id}-${f}`} className="pe-input" value={several(f) ? "\u0000" : albumValue(f)} disabled={viewOnly} onChange={(e) => setA(f, e.target.value)}>
        {several(f) && <option value={"\u0000"}>(several)</option>}
        <option value="">Not set</option>
        {options.map((o) => (
          <option key={o} value={o}>
            {labels[o] ?? o}
          </option>
        ))}
      </select>
    </label>
  );

  const rt = splitReleaseType(albumValue("releaseType"));
  const rtSeveral = several("releaseType");
  const status = data.statusField;
  const statusOptions = (current: string) => [...new Set([...data.statusChoices, ...(current ? [current] : [])])];

  return (
    <form
      onSubmit={(e) => {
        e.preventDefault();
        void save();
      }}
    >
      {data.reason && (
        <p className={`te-banner ${data.writable ? "" : "te-banner--block"}`} role="status">
          {data.reason}
        </p>
      )}
      <fieldset className="te-album" disabled={viewOnly}>
        <legend className="te-legend">{single ? "Album fields" : "Album"}</legend>
        <div className="pe-form__row">
          {text("albumArtist")}
          {text("album")}
        </div>
        <div className="pe-form__row">
          {text("year", "2019 or 2019-05-31")}
          {text("genre")}
        </div>
        <div className="pe-form__row">
          <label className="pe-field" htmlFor={`${id}-rt1`}>
            <span className="pe-field__label">Release type</span>
            <select id={`${id}-rt1`} className="pe-input" value={rtSeveral ? "\u0000" : rt.primary} onChange={(e) => setA("releaseType", joinReleaseType(e.target.value === "\u0000" ? "" : e.target.value, rtSeveral ? "" : rt.secondary))}>
              {rtSeveral && <option value={"\u0000"}>(several)</option>}
              <option value="">Not set</option>
              {PRIMARY_TYPES.map((p) => (
                <option key={p} value={p}>
                  {p}
                </option>
              ))}
            </select>
          </label>
          <label className="pe-field" htmlFor={`${id}-rt2`}>
            <span className="pe-field__label">Second part</span>
            <select id={`${id}-rt2`} className="pe-input" value={rtSeveral ? "\u0000" : rt.secondary} onChange={(e) => setA("releaseType", joinReleaseType(rtSeveral ? "album" : rt.primary, e.target.value === "\u0000" ? "" : e.target.value))}>
              {rtSeveral && <option value={"\u0000"}>(several)</option>}
              <option value="">None</option>
              {SECONDARY_TYPES.map((p) => (
                <option key={p} value={p}>
                  {p}
                </option>
              ))}
            </select>
          </label>
        </div>
        <div className="pe-form__row">
          {text("edition", "Standard, Deluxe")}
          {text("group")}
        </div>
        <div className="pe-form__row">
          {text("era")}
          {choice("quality", QUALITIES)}
          {choice("explicit", ["explicit", "clean"], { explicit: "Explicit", clean: "Clean" })}
        </div>
      </fieldset>

      <div className="te-grid-wrap">
        <table className="te-grid">
          <caption className="sr-only">Songs</caption>
          <thead>
            <tr>
              <th scope="col">Track</th>
              <th scope="col">Of</th>
              <th scope="col">Disc</th>
              <th scope="col">Of</th>
              <th scope="col">Title</th>
              <th scope="col">Artist</th>
              <th scope="col">Release status</th>
              <th scope="col">Version note</th>
              <th scope="col">Date</th>
              <th scope="col">Source</th>
              <th scope="col">Flags</th>
              <th scope="col">Explicit</th>
              <th scope="col">ISRC</th>
            </tr>
          </thead>
          <tbody>
            {data.files.map((f) => {
              const off = viewOnly || !f.stamp;
              const cell = (k: TagField, cls = "") => (
                <td className={cls}>
                  <input
                    className="te-cell"
                    aria-label={`${FIELD_LABEL[k]}, ${f.values.title ?? f.fileName}`}
                    value={trackValue(f.trackId, k)}
                    disabled={off}
                    autoFocus={focus === k && f === data.files[0]}
                    onChange={(e) => setT(f.trackId, k, e.target.value)}
                  />
                </td>
              );
              const cur = trackValue(f.trackId, status);
              return (
                <tr key={f.trackId} title={f.error ?? f.localPath ?? f.fileName} className={f.error ? "te-row--error" : ""}>
                  {cell("trackNumber", "te-num")}
                  {cell("trackTotal", "te-num")}
                  {cell("discNumber", "te-num")}
                  {cell("discTotal", "te-num")}
                  {cell("title", "te-wide")}
                  {cell("artist", "te-wide")}
                  <td>
                    <select className="te-cell" aria-label={`Release status, ${f.values.title ?? f.fileName}`} value={cur} disabled={off} onChange={(e) => setT(f.trackId, status, e.target.value)}>
                      <option value="">Not set</option>
                      {statusOptions(cur).map((o) => (
                        <option key={o} value={o}>
                          {o}
                        </option>
                      ))}
                    </select>
                  </td>
                  {cell("versionNote")}
                  {cell("archiveDate", "te-mid")}
                  {cell("source")}
                  {cell("flags", "te-mid")}
                  <td>
                    <select className="te-cell" aria-label={`Explicit, ${f.values.title ?? f.fileName}`} value={trackValue(f.trackId, "explicit")} disabled={off} onChange={(e) => setT(f.trackId, "explicit", e.target.value)}>
                      <option value="">Not set</option>
                      <option value="explicit">Explicit</option>
                      <option value="clean">Clean</option>
                    </select>
                  </td>
                  {cell("isrc", "te-mid")}
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>

      {!viewOnly && albumId && !single && (
        <div className="te-fixes" role="group" aria-label="One-click fixes">
          <Button size="sm" onClick={() => fixFrom("discTags", albumId, data.title, onClose)}>
            Fill disc tags
          </Button>
          <Button size="sm" onClick={() => fixFrom("releaseStatus", albumId, data.title, onClose)}>
            Fill release status
          </Button>
          <Button size="sm" onClick={() => fixFrom("isrc", albumId, data.title, onClose)}>
            Look up ISRCs
          </Button>
        </div>
      )}
      <p className="pe__note">
        Writes only the fields here, into {readable.length === 1 ? "the file" : `${readable.length} files`} in place. Never renames or moves files.
        {data.scheme !== "archive" && ` Release status goes in ${data.scheme === "custom" ? data.customKey : data.scheme === "picard" ? "RELEASESTATUS" : "STATUS"}, as these files already have it.`}
      </p>
      {problem && (
        <p className="confirm__error" role="alert">
          {problem}
        </p>
      )}
      <ConfirmMany n={many.pending} />
      <div className="confirm__actions">
        <Button onClick={onClose}>Cancel</Button>
        <Button variant="primary" type="submit" disabled={saving || (!data.needsMusicFolder && !request.files.length && !viewOnly)}>
          {data.needsMusicFolder ? "Set up music folder" : saving ? "Saving..." : many.pending ? `Save ${many.pending} files` : "Save"}
        </Button>
      </div>
    </form>
  );
}

function fixFrom(kind: FixTarget["kind"], albumId: string, title: string, close: () => void) {
  close();
  useTagEdit.getState().openFix({ kind, albumId, title });
}

// ---------------------------------------------------------------- fixes

function PreviewTable({ p, picked, setPicked }: { p: TagFixPreview; picked: Set<number>; setPicked: (s: Set<number>) => void }) {
  return (
    <>
      {p.changes.length > 0 && (
        <div className="te-grid-wrap">
          <table className="te-grid te-preview">
            <thead>
              <tr>
                <th scope="col">
                  <span className="sr-only">Change</span>
                </th>
                <th scope="col">File</th>
                <th scope="col">Field</th>
                <th scope="col">Now</th>
                <th scope="col">After</th>
                <th scope="col">Why</th>
              </tr>
            </thead>
            <tbody>
              {p.changes.map((c, i) => (
                <tr key={`${c.trackId}:${c.field}`}>
                  <td>
                    <input
                      type="checkbox"
                      aria-label={`Change ${FIELD_LABEL[c.field]} on ${c.fileName}`}
                      checked={picked.has(i)}
                      onChange={(e) => {
                        const n = new Set(picked);
                        if (e.target.checked) n.add(i);
                        else n.delete(i);
                        setPicked(n);
                      }}
                    />
                  </td>
                  <td className="te-wide">{c.fileName}</td>
                  <td>{FIELD_LABEL[c.field]}</td>
                  <td>{c.before ?? <span className="te-dim">not set</span>}</td>
                  <td>{c.after}</td>
                  <td className="te-dim">{c.note ?? ""}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
      {p.kept.length > 0 && (
        <details className="te-kept" open={p.changes.length === 0}>
          <summary>Kept as they are ({p.kept.length})</summary>
          <ul>
            {p.kept.map((k) => (
              <li key={k.trackId}>
                <strong>{k.fileName}</strong>: {k.reason}
              </li>
            ))}
          </ul>
        </details>
      )}
    </>
  );
}

function PreviewActions({ p, picked, onClose }: { p: TagFixPreview; picked: Set<number>; onClose: () => void }) {
  const [saving, setSaving] = useState(false);
  const [problem, setProblem] = useState<string | null>(null);
  const many = useConfirmMany();
  const changes = p.changes.filter((_, i) => picked.has(i));
  const files = new Set(changes.map((c) => c.trackId)).size;
  const apply = async () => {
    if (!many.ok(files)) return;
    setSaving(true);
    const r = await saveTags(fixRequest(p.label, changes));
    setSaving(false);
    if (!r) return;
    if (r.saved === 0 && r.problems.length) return setProblem(r.message);
    onClose();
  };
  return (
    <>
      {problem && (
        <p className="confirm__error" role="alert">
          {problem}
        </p>
      )}
      <ConfirmMany n={many.pending} />
      <div className="confirm__actions">
        <Button onClick={onClose}>{p.changes.length ? "Cancel" : "Close"}</Button>
        {p.changes.length > 0 && (
          <Button variant="primary" disabled={saving || changes.length === 0} onClick={() => void apply()}>
            {saving ? "Saving..." : `Change ${files === 1 ? "1 file" : `${files} files`}`}
          </Button>
        )}
      </div>
    </>
  );
}

function Preview({ p, onClose }: { p: TagFixPreview; onClose: () => void }) {
  // Release status suggestions start unticked: each one is the user's call.
  const [picked, setPicked] = useState<Set<number>>(() => new Set(p.pickEach ? [] : p.changes.map((_, i) => i)));
  return (
    <>
      <p className="pe__note" role="status">
        {p.message}
        {p.pickEach && p.changes.length > 0 && " Tick each one you want: albums often mix statuses on purpose."}
      </p>
      <PreviewTable p={p} picked={picked} setPicked={setPicked} />
      <PreviewActions p={p} picked={picked} onClose={onClose} />
    </>
  );
}

const FIX_TITLE: Record<FixTarget["kind"], string> = {
  discTags: "Fill disc tags",
  releaseStatus: "Fill release status",
  isrc: "Look up ISRCs",
  explicit: "Mark explicit",
};

function FixDialog({ target, onClose }: { target: FixTarget; onClose: () => void }) {
  return (
    <Dialog open onClose={onClose} label={FIX_TITLE[target.kind]} className="pe pe--wide te">
      <h2 className="pe__title">
        {FIX_TITLE[target.kind]}: {target.title}
      </h2>
      {target.kind === "isrc" ? <IsrcFix albumId={target.albumId} onClose={onClose} /> : <SimpleFix target={target} onClose={onClose} />}
    </Dialog>
  );
}

function SimpleFix({ target, onClose }: { target: FixTarget; onClose: () => void }) {
  const { data, error } = useLoad(() => backend.tagEditFixPreview(target.kind, target.albumId));
  return error ? <Problem text={error} onClose={onClose} /> : data ? <Preview p={data} onClose={onClose} /> : <Loading />;
}

function IsrcFix({ albumId, onClose }: { albumId: string; onClose: () => void }) {
  const { data, error } = useLoad<IsrcReleases>(() => backend.tagEditIsrcReleases(albumId));
  const [release, setRelease] = useState<IsrcRelease | null>(null);
  if (error) return <Problem text={error} onClose={onClose} />;
  if (!data) return <Loading />;
  if (release) return <IsrcPreview albumId={albumId} release={release} onClose={onClose} />;
  return <ReleasePick r={data} onPick={setRelease} onClose={onClose} />;
}

function ReleasePick({ r, onPick, onClose }: { r: IsrcReleases; onPick: (x: IsrcRelease) => void; onClose: () => void }) {
  const [sel, setSel] = useState<number | null>(null);
  const name = useId();
  return (
    <>
      <p className="pe__note">
        {r.message}
        {r.candidates.length > 0 && ` ${r.wanted === 1 ? "1 released song needs" : `${r.wanted} released songs need`} an ISRC. Songs are matched by title, artist and length.`}
      </p>
      {r.candidates.length > 0 && (
        <ul className="te-releases" role="radiogroup" aria-label="Releases">
          {r.candidates.map((c, i) => (
            <li key={`${c.source}:${c.id}`}>
              <label>
                <input type="radio" name={name} checked={sel === i} onChange={() => setSel(i)} />
                <span>
                  <strong>{c.title}</strong> by {c.artist}
                  <span className="te-dim">
                    {" "}
                    {[c.date, c.country, `${c.tracks} tracks`, c.source === "musicbrainz" ? "MusicBrainz" : "Deezer"].filter(Boolean).join(" · ")}
                  </span>
                </span>
              </label>
            </li>
          ))}
        </ul>
      )}
      <div className="confirm__actions">
        <Button onClick={onClose}>{r.candidates.length ? "Cancel" : "Close"}</Button>
        {r.candidates.length > 0 && (
          <Button variant="primary" disabled={sel === null} onClick={() => sel !== null && onPick(r.candidates[sel])}>
            Use this release
          </Button>
        )}
      </div>
    </>
  );
}

function IsrcPreview({ albumId, release, onClose }: { albumId: string; release: IsrcRelease; onClose: () => void }) {
  const { data, error } = useLoad(() => backend.tagEditIsrcPreview(albumId, release));
  return error ? <Problem text={error} onClose={onClose} /> : data ? <Preview p={data} onClose={onClose} /> : <Loading />;
}

function ExplicitDialog({ target, onClose }: { target: ExplicitTarget; onClose: () => void }) {
  const title = target.explicit ? "Mark as explicit" : "Mark as clean";
  const { data, error } = useLoad(() => backend.tagEditExplicitPreview(target.albumIds, target.explicit, target.era ?? null));
  return (
    <Dialog open onClose={onClose} label={title} className="pe pe--wide te">
      <h2 className="pe__title">
        {title}: {target.title}
      </h2>
      {error ? <Problem text={error} onClose={onClose} /> : data ? <Preview p={data} onClose={onClose} /> : <Loading />}
    </Dialog>
  );
}

// ---------------------------------------------------------------- the check

const SEVERITY = {
  error: { icon: CircleAlert, label: "Needs fixing" },
  warning: { icon: TriangleAlert, label: "Worth fixing" },
  info: { icon: Info, label: "Good to know" },
} as const;

function CheckDialog({ scope, title, onClose }: { scope: Parameters<typeof backend.tagEditCheck>[0]; title: string; onClose: () => void }) {
  const { data, error, reload } = useLoad<TagCheckReport>(() => backend.tagEditCheck(scope));
  const [hidden, setHidden] = useState<Set<string>>(new Set());
  const fine = async (f: TagFinding) => {
    try {
      await backend.tagEditCheckFine(f.id, true);
      setHidden(new Set(hidden).add(f.id));
    } catch (e) {
      toast.error(errText(e) || "Couldn't mark it as fine");
    }
  };
  const fix = (f: TagFinding) => {
    if (!f.fix) return;
    onClose();
    useTagEdit.getState().openEditor({ albumId: f.fix.albumId, trackIds: [], focus: f.fix.field });
  };
  const rows = data?.findings.filter((f) => !hidden.has(f.id)) ?? [];
  return (
    <Dialog open onClose={onClose} label="Check tags" className="pe pe--wide te">
      <div className="help-heading">
        <h2 className="pe__title">Check tags: {title}</h2>
        <HelpButton page="tag-editor" anchor="check-tags" label="What Check tags looks for" inline />
      </div>
      {error ? (
        <Problem text={error} onClose={onClose} />
      ) : !data ? (
        <Loading />
      ) : (
        <>
          <p className="pe__note" role="status">
            {rows.length === 0 ? "Nothing to fix." : `${rows.length} ${rows.length === 1 ? "thing" : "things"} to look at`} in {data.albums === 1 ? "1 album" : `${data.albums} albums`} ({data.files} songs).
            {!data.fromFiles && " Read from what Plex has. Set up the music folder to check the files themselves and fix them here."}
            {data.ignored > 0 && ` ${data.ignored} marked fine.`}
          </p>
          {data.styles.slice(0, scope.kind === "album" ? 1 : 0).map((s) => (
            <p key={s.albumId} className="te-style">
              {s.text}
            </p>
          ))}
          {scope.kind === "album" && data.fromFiles && (
            <div className="te-fixes" role="group" aria-label="One-click fixes">
              {(["discTags", "releaseStatus", "isrc"] as const).map((k) => (
                <Button key={k} size="sm" onClick={() => fixFrom(k, scope.id, title, onClose)}>
                  {FIX_TITLE[k]}
                </Button>
              ))}
            </div>
          )}
          <ul className="te-findings">
            {rows.map((f) => {
              const S = SEVERITY[f.severity];
              return (
                <li key={f.id} className={`te-finding te-finding--${f.severity}`}>
                  <S.icon size={16} aria-hidden="true" className="te-finding__icon" />
                  <div className="te-finding__text">
                    <div className="te-finding__what">
                      <span className="sr-only">{S.label}: </span>
                      {scope.kind !== "album" && <span className="te-dim">{f.albumTitle}: </span>}
                      {f.what}
                    </div>
                    <div className="te-finding__why">{f.why}</div>
                  </div>
                  <div className="te-finding__actions">
                    {f.fix && (
                      <Button size="sm" onClick={() => fix(f)}>
                        Fix
                      </Button>
                    )}
                    <Button size="sm" variant="ghost" icon={<Check size={14} />} onClick={() => void fine(f)}>
                      This is fine
                    </Button>
                  </div>
                </li>
              );
            })}
          </ul>
          <div className="confirm__actions">
            <Button onClick={reload}>Check again</Button>
            <Button variant="primary" onClick={onClose}>
              Close
            </Button>
          </div>
        </>
      )}
    </Dialog>
  );
}

/** Album fields shown first in the editor (for tests and the check's Fix). */
export const EDITOR_ALBUM_FIELDS = ALBUM_FIELDS;
