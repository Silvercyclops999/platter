/**
 * One diary entry's editor: the date, a rating in half stars and a short
 * plain note. Delete asks once more before the entry goes; a deleted entry
 * never comes back on its own.
 */
import { useRef, useState } from "react";
import { Art } from "../components/Art";
import { Button } from "../components/Button";
import { Dialog } from "../components/Dialog";
import { backend } from "../lib/backend";
import { NOTE_MAX, longDate, playedLine, whereLine } from "../lib/diary";
import { errorMessage } from "../lib/errorText";
import type { DiaryItem } from "../lib/types";
import { toast } from "../stores/toasts";
import { StarInput } from "./StarRating";

/** "2026-10-17" in local time, for a date field. */
function dayValue(ms: number): string {
  const d = new Date(ms);
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
}

/** The same time of day on another date. */
function onDay(ms: number, day: string): number | null {
  const m = /^(\d{4})-(\d{2})-(\d{2})$/.exec(day);
  if (!m) return null;
  const was = new Date(ms);
  return new Date(Number(m[1]), Number(m[2]) - 1, Number(m[3]), was.getHours(), was.getMinutes(), was.getSeconds()).getTime();
}

export function DiaryEntryDialog({ entry, onClose, onSaved, onDeleted }: { entry: DiaryItem; onClose: () => void; onSaved: (e: DiaryItem) => void; onDeleted: () => void }) {
  const [rating, setRating] = useState<number | null>(entry.rating);
  const [note, setNote] = useState(entry.note);
  const [day, setDay] = useState(dayValue(entry.at));
  const [busy, setBusy] = useState(false);
  const [confirming, setConfirming] = useState(false);
  const noteRef = useRef<HTMLTextAreaElement>(null);
  const length = [...note].length;
  const today = dayValue(Date.now());
  const where = whereLine(entry);

  const save = async () => {
    const at = day === dayValue(entry.at) ? undefined : (onDay(entry.at, day) ?? undefined);
    setBusy(true);
    try {
      const saved = await backend.diaryEntryEdit(entry.id, { rating, note, ...(at != null ? { at } : {}) });
      onSaved(saved);
    } catch (e) {
      toast.error(errorMessage(e));
      setBusy(false);
    }
  };

  const remove = async () => {
    setBusy(true);
    try {
      await backend.diaryEntryDelete(entry.id);
      toast.info(`Deleted the entry for ${entry.title}`);
      onDeleted();
    } catch (e) {
      toast.error(errorMessage(e));
      setBusy(false);
    }
  };

  return (
    <Dialog open onClose={onClose} label={`Diary entry for ${entry.title}`} className="diary-dialog" initialFocus={noteRef}>
      <div className="diary-dialog__card">
        <div className="diary-dialog__sleeve">
          <span className="diary-entry__disc" aria-hidden="true" />
          <Art path={entry.thumb} size={96} alt="" />
        </div>
        <div className="diary-dialog__about">
          <h2 className="diary-dialog__title">{entry.title}</h2>
          <p className="diary-dialog__artist">{entry.artist}</p>
          <p className="diary-dialog__meta">
            {longDate(entry.at)} · {playedLine(entry)}
            {where && ` · ${where}`}
          </p>
        </div>
      </div>

      <div className="diary-dialog__row">
        <span className="diary-dialog__label" id="diary-rating-label">
          Rating
        </span>
        <StarInput value={rating} onChange={setRating} label="Rating" />
        {rating != null && (
          <Button size="sm" variant="ghost" onClick={() => setRating(null)}>
            Clear
          </Button>
        )}
      </div>

      <label className="diary-dialog__row">
        <span className="diary-dialog__label">Date</span>
        <input className="diary-dialog__date" type="date" value={day} max={today} onChange={(e) => setDay(e.target.value)} />
      </label>

      <label className="diary-dialog__note">
        <span className="diary-dialog__label">Note</span>
        <textarea
          ref={noteRef}
          className="diary-note diary-dialog__text"
          value={note}
          rows={5}
          maxLength={NOTE_MAX}
          placeholder="What stayed with you?"
          onChange={(e) => setNote(e.target.value)}
        />
        <span className={`diary-dialog__count ${length > NOTE_MAX - 40 ? "is-near" : ""}`} aria-live="polite">
          {length} / {NOTE_MAX}
        </span>
      </label>

      <div className="diary-dialog__actions">
        {confirming ? (
          <Button size="sm" variant="ghost" className="diary-dialog__delete is-confirming" disabled={busy} onClick={() => void remove()}>
            Delete for good
          </Button>
        ) : (
          <Button size="sm" variant="ghost" className="diary-dialog__delete" disabled={busy} onClick={() => setConfirming(true)}>
            Delete entry
          </Button>
        )}
        <span className="diary-dialog__spacer" />
        <Button size="sm" variant="secondary" disabled={busy} onClick={onClose}>
          Cancel
        </Button>
        <Button size="sm" variant="primary" disabled={busy || length > NOTE_MAX} onClick={() => void save()}>
          Save
        </Button>
      </div>
    </Dialog>
  );
}
