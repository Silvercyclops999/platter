/**
 * Marking an album as owned: the "I own this" menu item, the dialog that
 * edits the copies (format, pressing, colour, condition, notes) and the
 * "On vinyl" badge.
 */
import { Disc3, Plus, X } from "lucide-react";
import { useId, useRef, useState } from "react";
import { create } from "zustand";
import type { OwnedCopy, PhysicalFormat } from "../lib/types";
import { FORMATS, FORMAT_LABEL, blankCopy, saveOwned, shelfLabel, useOwned, usePhysical } from "../stores/physical";
import { useSettings } from "../stores/settings";
import type { MenuItem } from "../stores/ui";
import { Badge } from "./Badge";
import { Button, IconButton } from "./Button";
import { Dialog } from "./Dialog";
import "./physical.css";

export interface OwnTarget {
  albumId: string;
  title: string;
  artist: string;
  thumb: string | null;
}

interface DialogState {
  target: OwnTarget | null;
  open: (t: OwnTarget) => void;
  close: () => void;
}

export const usePhysicalDialog = create<DialogState>()((set) => ({
  target: null,
  open: (target) => set({ target }),
  close: () => set({ target: null }),
}));

/** The album menu's item: "I own this" or "Edit what you own". Hidden while Show physical collection is off. */
export function physicalItem(t: OwnTarget): MenuItem {
  const owned = !!usePhysical.getState().owned?.get(t.albumId);
  return {
    label: owned ? "Edit what you own" : "I own this",
    icon: Disc3,
    when: () => useSettings.getState().settings?.showPhysicalCollection !== false,
    onSelect: () => usePhysicalDialog.getState().open(t),
  };
}

/** "On vinyl" next to an album's other badges. */
export function ShelfBadge({ albumId }: { albumId: string }) {
  const o = useOwned(albumId);
  if (!o || o.formats.length === 0) return null;
  const label = shelfLabel(o.formats);
  return (
    <Badge tone="shelf" title={`${label}. ${o.copies.length === 1 ? "1 copy" : `${o.copies.length} copies`} on your shelf`}>
      <Disc3 size={12} aria-hidden="true" className="shelf-badge__icon" />
      {label}
    </Badge>
  );
}

/** The small disc on an album card, with the label for screen readers and on hover. */
export function ShelfMark({ formats }: { formats: PhysicalFormat[] }) {
  const label = shelfLabel(formats);
  if (!label) return null;
  return (
    <span className="shelf-dot" title={label} role="img" aria-label={label}>
      <Disc3 size={12} aria-hidden="true" />
    </span>
  );
}

/** Mounted once in the app shell. */
export function PhysicalDialogHost() {
  const target = usePhysicalDialog((s) => s.target);
  if (!target) return null;
  return <OwnedDialog key={target.albumId} target={target} onClose={() => usePhysicalDialog.getState().close()} />;
}

const FIELDS: { key: keyof Pick<OwnedCopy, "pressing" | "colour" | "condition" | "notes">; label: string; hint: string }[] = [
  { key: "pressing", label: "Pressing", hint: "2016 reissue, Japanese pressing" },
  { key: "colour", label: "Colour", hint: "Black, red translucent" },
  { key: "condition", label: "Condition", hint: "Near mint, very good" },
  { key: "notes", label: "Notes", hint: "Signed, bought at the show" },
];

export function OwnedDialog({ target, onClose }: { target: OwnTarget; onClose: () => void }) {
  const current = usePhysical((s) => s.owned?.get(target.albumId) ?? null);
  const [copies, setCopies] = useState<OwnedCopy[]>(() => (current?.copies.length ? current.copies.map((c) => ({ ...c })) : [blankCopy()]));
  const [busy, setBusy] = useState(false);
  const save = useRef<HTMLButtonElement>(null);
  const id = useId();
  const change = (i: number, patch: Partial<OwnedCopy>) => setCopies((cs) => cs.map((c, j) => (j === i ? { ...c, ...patch } : c)));
  const done = async (list: OwnedCopy[]) => {
    setBusy(true);
    const ok = await saveOwned(target.albumId, { copies: list, title: target.title, artist: target.artist, thumb: target.thumb });
    setBusy(false);
    if (ok) onClose();
  };
  return (
    <Dialog open onClose={onClose} label={`What you own of ${target.title}`} className="own-dialog">
      <h2 className="own-dialog__title">On your shelf</h2>
      <p className="own-dialog__album">
        {target.title} <span className="own-dialog__artist">· {target.artist}</span>
      </p>
      <form
        className="own-dialog__form"
        onSubmit={(e) => {
          e.preventDefault();
          void done(copies);
        }}
      >
        <ol className="own-copies" aria-label="Copies">
          {copies.map((c, i) => (
            <li key={i} className="own-copy" aria-label={`Copy ${i + 1}, ${FORMAT_LABEL[c.format]}`}>
              <div className="own-copy__head">
                <div className="own-formats" role="radiogroup" aria-label={`Format of copy ${i + 1}`}>
                  {FORMATS.map((f: PhysicalFormat) => (
                    <button
                      key={f}
                      type="button"
                      role="radio"
                      aria-checked={c.format === f}
                      className={`chip chip--sm ${c.format === f ? "is-selected" : ""}`}
                      onClick={() => change(i, { format: f })}
                    >
                      {FORMAT_LABEL[f]}
                    </button>
                  ))}
                </div>
                {c.discogs && <span className="own-copy__from">From Discogs</span>}
                {copies.length > 1 && (
                  <IconButton size="sm" label={`Remove copy ${i + 1}`} icon={<X size={16} />} onClick={() => setCopies((cs) => cs.filter((_, j) => j !== i))} />
                )}
              </div>
              <div className="own-copy__fields">
                {FIELDS.map((f) => (
                  <label key={f.key} className="own-field" htmlFor={`${id}-${i}-${f.key}`}>
                    <span className="own-field__label">{f.label}</span>
                    <input
                      id={`${id}-${i}-${f.key}`}
                      className="own-input"
                      value={c[f.key]}
                      maxLength={300}
                      placeholder={f.hint}
                      onChange={(e) => change(i, { [f.key]: e.target.value })}
                    />
                  </label>
                ))}
              </div>
            </li>
          ))}
        </ol>
        {copies.length < 20 && (
          <Button type="button" size="sm" variant="ghost" icon={<Plus size={16} />} onClick={() => setCopies((cs) => [...cs, blankCopy(cs[cs.length - 1]?.format ?? "vinyl")])}>
            Add another copy
          </Button>
        )}
        <div className="confirm__actions own-dialog__actions">
          {current && (
            <Button type="button" variant="ghost" className="own-dialog__remove" disabled={busy} onClick={() => void done([])}>
              I don't own it
            </Button>
          )}
          <Button type="button" variant="ghost" onClick={onClose}>
            Cancel
          </Button>
          <Button ref={save} type="submit" variant="primary" disabled={busy || copies.length === 0}>
            Save
          </Button>
        </div>
      </form>
    </Dialog>
  );
}
