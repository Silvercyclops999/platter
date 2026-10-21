/**
 * The library doctor's tag fix: the album tags it suggests for every song
 * (album artist, album name, year), which the user can change, and each
 * file's change, which they can untick, before anything is written. Saving
 * goes through the tag editor, so it's logged and the toast can undo it.
 */
import { useEffect, useId, useMemo, useState } from "react";
import { Button } from "../components/Button";
import { Dialog } from "../components/Dialog";
import { Skeleton } from "../components/Skeleton";
import { backend } from "../lib/backend";
import { FIELD_LABEL, asksFirst, doctorChanges, doctorValueProblem, fixRequest, type DoctorField } from "../lib/tagEdit";
import type { DoctorFinding, TagEditorData } from "../lib/types";
import { errText } from "../stores/plexEdit";
import { nav } from "../stores/router";
import { useSettings } from "../stores/settings";
import { saveTags, useTagEdit } from "../stores/tagEdit";
import { toast } from "../stores/toasts";
import "../components/plex-edit.css";
import "../components/tag-edit.css";

const FIELDS: DoctorField[] = ["albumArtist", "album", "year"];

/** The fields the finding sets, with its values as text. */
export function suggested(f: DoctorFinding): Partial<Record<DoctorField, string>> {
  const out: Partial<Record<DoctorField, string>> = {};
  const s = f.set;
  if (!s) return out;
  if (s.albumArtist != null) out.albumArtist = s.albumArtist;
  if (s.album != null) out.album = s.album;
  if (s.year != null) out.year = String(s.year);
  // An album artist finding always offers the field, even with nothing to suggest.
  if (f.kind === "albumArtist" && out.albumArtist === undefined) out.albumArtist = "";
  return out;
}

/** Where "Edit all tags" puts the cursor. */
function focusOf(f: DoctorFinding): DoctorField {
  return FIELDS.find((k) => k in suggested(f)) ?? "albumArtist";
}

/** Open the full tag editor on the finding's album instead. */
export function editAllTags(f: DoctorFinding, close?: () => void) {
  close?.();
  useTagEdit.getState().openEditor({ albumId: f.albumId, trackIds: [], focus: focusOf(f) });
}

export function DoctorTagFix({ finding, onClose }: { finding: DoctorFinding; onClose: () => void }) {
  const [data, setData] = useState<TagEditorData | null>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    let live = true;
    backend.tagEditOpen(finding.albumId, []).then(
      (d) => live && setData(d),
      (e) => live && setError(errText(e) || "Couldn't read the files."),
    );
    return () => {
      live = false;
    };
  }, [finding.albumId]);
  const title = finding.kind === "albumArtist" ? "Set the album artist" : "Make the tags match";
  return (
    <Dialog open onClose={onClose} label={title} className="pe pe--wide te doctor-fix">
      <h2 className="pe__title">
        {title}: {finding.album}
      </h2>
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
      ) : data ? (
        <FixForm finding={finding} data={data} onClose={onClose} />
      ) : (
        <div className="pe-form" aria-busy="true">
          {Array.from({ length: 3 }, (_, i) => (
            <Skeleton key={i} width="100%" height={36} />
          ))}
        </div>
      )}
    </Dialog>
  );
}

function FixForm({ finding, data, onClose }: { finding: DoctorFinding; data: TagEditorData; onClose: () => void }) {
  const id = useId();
  const [want, setWant] = useState(() => suggested(finding));
  // Unticked rows, by file and field, so they stay unticked while values change.
  const [skip, setSkip] = useState<Set<string>>(new Set());
  const [saving, setSaving] = useState(false);
  const [problem, setProblem] = useState<string | null>(null);
  // A big album asks once more first, when Settings says so.
  const askMany = useSettings((s) => s.settings?.askBeforeManyFiles);
  const [askedFor, setAskedFor] = useState<number | null>(null);
  const { changes, kept } = useMemo(() => doctorChanges(data.files, want), [data.files, want]);
  const key = (c: { trackId: string; field: string }) => `${c.trackId}:${c.field}`;
  const picked = changes.filter((c) => !skip.has(key(c)));
  const files = new Set(picked.map((c) => c.trackId)).size;
  const fields = FIELDS.filter((k) => k in want);
  const problems = fields.map((k) => [k, doctorValueProblem(k, want[k] ?? "")] as const).filter(([, p]) => p);
  const blocked = data.needsMusicFolder || !data.writable;

  const save = async () => {
    if (data.needsMusicFolder) {
      onClose();
      nav.go({ name: "settings", page: "tags" });
      toast.info("Set the music folder under Library, Tag editor");
      return;
    }
    if (problems.length) return;
    if (asksFirst(files, askMany) && askedFor !== files) return setAskedFor(files);
    setSaving(true);
    setProblem(null);
    const r = await saveTags(fixRequest(`Library doctor: ${finding.title.toLowerCase()} on ${finding.album}`, picked));
    setSaving(false);
    if (!r) return;
    if (r.saved === 0 && r.problems.length) return setProblem(r.message);
    onClose();
  };

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
      <p className="pe__note">Every song on {finding.album} gets {fields.length === 1 ? "this" : "these"}. Change {fields.length === 1 ? "it" : "them"} first if it isn't right.</p>
      <div className="pe-form__row">
        {fields.map((k) => (
          <label key={k} className="pe-field" htmlFor={`${id}-${k}`}>
            <span className="pe-field__label">{FIELD_LABEL[k]}</span>
            <input
              id={`${id}-${k}`}
              className="pe-input"
              value={want[k] ?? ""}
              inputMode={k === "year" ? "numeric" : undefined}
              placeholder={k === "albumArtist" ? "Various Artists" : undefined}
              disabled={!data.writable && !data.needsMusicFolder}
              autoFocus={k === fields[0]}
              aria-invalid={!!doctorValueProblem(k, want[k] ?? "")}
              onChange={(e) => setWant((w) => ({ ...w, [k]: e.target.value }))}
            />
          </label>
        ))}
      </div>
      {!blocked &&
        problems.map(([k, p]) => (
          <p key={k} className="confirm__error" role="alert">
            {FIELD_LABEL[k]}: {p}
          </p>
        ))}

      {finding.kind === "albumArtist" &&
        finding.others.map((o) => (
          <p key={o.id} className="pe__note doctor-fix__other">
            {o.title} has the same name under another artist. Give its songs the same album artist too, so Plex puts them together.{" "}
            <button
              type="button"
              className="link"
              onClick={() => {
                onClose();
                useTagEdit.getState().openEditor({ albumId: o.id, trackIds: [], focus: "albumArtist" });
              }}
            >
              Edit its tags
            </button>
          </p>
        ))}

      {!blocked && problems.length === 0 && changes.length === 0 && (
        <p className="pe__note" role="status">
          Every file already has {fields.length === 1 ? "this" : "these"}. Plex catches up after its next scan.
        </p>
      )}
      {!blocked && problems.length === 0 && changes.length > 0 && (
        <div className="te-grid-wrap">
          <table className="te-grid te-preview">
            <caption className="sr-only">What changes in each file</caption>
            <thead>
              <tr>
                <th scope="col">
                  <span className="sr-only">Change</span>
                </th>
                <th scope="col">File</th>
                <th scope="col">Tag</th>
                <th scope="col">Now</th>
                <th scope="col">After</th>
              </tr>
            </thead>
            <tbody>
              {changes.map((c) => (
                <tr key={key(c)}>
                  <td>
                    <input
                      type="checkbox"
                      aria-label={`Change ${FIELD_LABEL[c.field]} on ${c.fileName}`}
                      checked={!skip.has(key(c))}
                      onChange={(e) => {
                        const n = new Set(skip);
                        if (e.target.checked) n.delete(key(c));
                        else n.add(key(c));
                        setSkip(n);
                      }}
                    />
                  </td>
                  <td className="te-wide">{c.fileName}</td>
                  <td>{FIELD_LABEL[c.field]}</td>
                  <td>{c.before ?? <span className="te-dim">not set</span>}</td>
                  <td>{c.after}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
      {!blocked && kept.length > 0 && (
        <details className="te-kept">
          <summary>Left as they are ({kept.length})</summary>
          <ul>
            {kept.map((k) => (
              <li key={k.trackId}>
                <strong>{k.fileName}</strong>: {k.reason}
              </li>
            ))}
          </ul>
        </details>
      )}
      <p className="pe__note">Writes only {fields.length === 1 ? "this tag" : "these tags"}, into the files in place. Never renames or moves files, and you can undo it from the message after saving.</p>
      {problem && (
        <p className="confirm__error" role="alert">
          {problem}
        </p>
      )}
      {askedFor === files && (
        <p className="te-confirm" role="alert">
          This changes {files} files. Select Change again to change them all.
        </p>
      )}
      <div className="confirm__actions">
        <Button variant="ghost" className="doctor-fix__all" onClick={() => editAllTags(finding, onClose)}>
          Edit all tags
        </Button>
        <Button onClick={onClose}>Cancel</Button>
        <Button variant="primary" type="submit" disabled={saving || (!data.needsMusicFolder && (!data.writable || files === 0 || problems.length > 0))}>
          {data.needsMusicFolder ? "Set up music folder" : saving ? "Saving..." : `Change ${files === 1 ? "1 file" : `${files} files`}`}
        </Button>
      </div>
    </form>
  );
}
