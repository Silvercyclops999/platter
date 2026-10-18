/**
 * "Split into sides" from an album's ⋯ menu: pick the songs where side B
 * (and C, D...) start. Kept per album and synced. Sides label the tracklist
 * and let one side play; the album itself still plays in order.
 */
import { useState } from "react";
import { Button } from "../components/Button";
import { Dialog } from "../components/Dialog";
import { backend } from "../lib/backend";
import { errorMessage } from "../lib/errorText";
import type { AlbumSides, Track } from "../lib/types";
import { toast } from "../stores/toasts";

const letter = (i: number) => String.fromCharCode(65 + i);

/** Where sides start now (every side's first song after side A's). */
function startsOf(sides: AlbumSides | null): Set<string> {
  return new Set((sides?.sides ?? []).slice(1).map((s) => s.trackIds[0]).filter(Boolean));
}

export function SidesDialog({
  albumId,
  title,
  tracks,
  sides,
  onClose,
  onSaved,
}: {
  albumId: string;
  title: string;
  tracks: Track[];
  sides: AlbumSides | null;
  onClose: () => void;
  onSaved: (s: AlbumSides) => void;
}) {
  const [starts, setStarts] = useState<Set<string>>(() => startsOf(sides));
  const [busy, setBusy] = useState(false);

  const save = async (value: string[] | null) => {
    setBusy(true);
    try {
      const s = await backend.setAlbumSides(albumId, value);
      onSaved(s);
      onClose();
    } catch (e) {
      toast.error(errorMessage(e));
      setBusy(false);
    }
  };

  const toggle = (id: string) =>
    setStarts((cur) => {
      const next = new Set(cur);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });

  // The side each song lands on with the starts picked now.
  let side = 0;
  const rows = tracks.map((t, i) => {
    if (i > 0 && starts.has(t.id)) side += 1;
    return { t, side, start: i > 0 && starts.has(t.id) };
  });
  const count = side + 1;

  return (
    <Dialog open onClose={onClose} label={`Split ${title} into sides`} className="sides-dialog">
      <h2 className="sides-dialog__title">Split into sides</h2>
      <p className="sides-dialog__sub">
        Tick the song each new side starts on. Sides label the songs and let you play one side; the album still plays in order.
      </p>
      <ol className="sides-dialog__list" aria-label={`${title} songs`}>
        {rows.map(({ t, side: s, start }, i) => (
          <li key={t.id} className={`sides-dialog__row ${start ? "is-start" : ""}`}>
            <span className="sides-dialog__side" aria-hidden="true">
              {start || i === 0 ? letter(s) : ""}
            </span>
            <span className="sides-dialog__n">{t.trackNumber || i + 1}</span>
            <span className="sides-dialog__song truncate">{t.title}</span>
            {i > 0 && (
              <label className="sides-dialog__start">
                <input type="checkbox" checked={start} onChange={() => toggle(t.id)} aria-label={`A new side starts on ${t.title}`} />
                <span aria-hidden="true">New side</span>
              </label>
            )}
          </li>
        ))}
      </ol>
      <p className="sides-dialog__count" role="status">
        {count < 2 ? "No sides yet." : `Sides A to ${letter(count - 1)}.`}
      </p>
      <div className="sides-dialog__actions">
        <div className="sides-dialog__quick">
          {tracks.length >= 2 && (
            <Button size="sm" variant="ghost" disabled={busy} onClick={() => setStarts(new Set([tracks[Math.ceil(tracks.length / 2)].id]))}>
              Split in half
            </Button>
          )}
          {sides?.detected && sides.source === "manual" && (
            <Button size="sm" variant="ghost" disabled={busy} onClick={() => void save(null)}>
              Use the record's sides
            </Button>
          )}
          {(sides?.sides.length ?? 0) > 0 && (
            <Button size="sm" variant="ghost" disabled={busy} onClick={() => void save([])}>
              No sides
            </Button>
          )}
        </div>
        <Button size="sm" variant="secondary" disabled={busy} onClick={onClose}>
          Cancel
        </Button>
        <Button size="sm" variant="primary" disabled={busy} onClick={() => void save([...starts])}>
          Save
        </Button>
      </div>
    </Dialog>
  );
}
