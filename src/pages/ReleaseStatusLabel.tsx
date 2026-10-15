import { useEffect, useId, useRef, useState } from "react";
import { Badge } from "../components/Badge";
import { Button } from "../components/Button";
import { backend } from "../lib/backend";
import type { AlbumReleaseStatus, Evidence, ReleaseMark } from "../lib/types";
import { useSettings } from "../stores/settings";
import { toast } from "../stores/toasts";

/**
 * An album's release status from the core. Reloads when the MusicBrainz
 * check or a mark changes it, or the settings that feed it change. Failures
 * show nothing.
 */
export function useReleaseStatus(albumId: string | null | undefined): AlbumReleaseStatus | null {
  const [status, setStatus] = useState<AlbumReleaseStatus | null>(null);
  const deps = useSettings((s) =>
    s.settings
      ? `${s.settings.countUnmatchedUnreleased}:${s.settings.checkUnmatchedMusicbrainz}:${s.settings.infoMusicbrainz}:${(s.settings.archiveFolderKeywords ?? []).join("|")}`
      : "",
  );
  useEffect(() => {
    setStatus(null);
    if (!albumId) return;
    let alive = true;
    const load = () =>
      backend.albumReleaseStatus(albumId).then(
        (s) => alive && setStatus(s),
        () => {},
      );
    void load();
    const uns = [
      backend.listen("releasestatus://changed", (e) => {
        if (e.id === albumId) void load();
      }),
      backend.listen("archive://changed", () => void load()),
    ];
    return () => {
      alive = false;
      uns.forEach((un) => un.then((f) => f()));
    };
  }, [albumId, deps]);
  return status;
}

/** The label for a status, or null when the album shows none. */
export function releaseLabel(s: Pick<AlbumReleaseStatus, "status" | "mark">): string | null {
  switch (s.status) {
    case "probablyUnreleased":
      return "Probably unreleased";
    case "plexMissed":
      return "Released, Plex missed it";
    case "confirmedUnreleased":
      return "Unreleased";
    case "released":
      return s.mark === "released" ? "Marked released" : null;
    default:
      return null;
  }
}

/** "Strong evidence" once the weights add up to a tag-like amount. */
export function strengthLine(score: number | undefined): string | null {
  if (score == null) return null;
  return score >= 60 ? "Strong evidence" : score > 0 ? "Some evidence" : null;
}

function EvidenceRow({ e }: { e: Evidence }) {
  const lean = e.weight > 0 ? "unreleased" : e.weight < 0 ? "released" : "none";
  const hint = e.kind === "folder" || e.kind === "duration";
  return (
    <li className="release-status__reason" data-lean={lean} data-decides={e.decides || undefined}>
      <span className="release-status__dot" aria-hidden="true" />
      <span>
        {e.text}
        {hint && <span className="release-status__note"> Adds weight only</span>}
      </span>
    </li>
  );
}

/**
 * The album header's release label ("Probably unreleased", "Released, Plex
 * missed it", "Unreleased" once confirmed) with a "Why?" popover listing
 * every piece of evidence, and Confirm / Not unreleased to settle it with a
 * mark (or clear the mark). Nothing for albums that are plainly released or
 * tagged.
 */
export function ReleaseStatusLabel({ status }: { status: AlbumReleaseStatus | null }) {
  const [open, setOpen] = useState(false);
  const [busy, setBusy] = useState(false);
  const btn = useRef<HTMLButtonElement>(null);
  const pop = useRef<HTMLDivElement>(null);
  const id = useId();

  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      if (!pop.current?.contains(e.target as Node) && !btn.current?.contains(e.target as Node)) setOpen(false);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        setOpen(false);
        btn.current?.focus();
      }
    };
    window.addEventListener("mousedown", onDown);
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("mousedown", onDown);
      window.removeEventListener("keydown", onKey);
    };
  }, [open]);

  const label = status ? releaseLabel(status) : null;
  if (!status || !label) return null;
  const evidence: Evidence[] =
    status.evidence ?? status.reasons.map((text) => ({ kind: "unmatched", text, weight: 1, decides: false }) as Evidence);
  const strength = status.status === "probablyUnreleased" ? strengthLine(status.score) : null;

  const mark = async (m: ReleaseMark | null) => {
    setBusy(true);
    try {
      await backend.releaseMarkSet("album", status.id, m);
      toast.info(m === "unreleased" ? "Marked as unreleased" : m === "released" ? "Marked as released" : "Mark cleared");
    } catch {
      toast.error("Couldn't save that");
    } finally {
      setBusy(false);
    }
  };

  return (
    <span className="release-status">
      <Badge tone="version">{label}</Badge>
      <button
        ref={btn}
        type="button"
        className="link release-status__why"
        aria-expanded={open}
        aria-controls={open ? id : undefined}
        aria-label={`Why ${label.toLowerCase()}?`}
        onClick={() => setOpen((v) => !v)}
      >
        Why?
      </button>
      {open && (
        <div ref={pop} id={id} role="dialog" aria-label={label} className="release-status__popover">
          <strong className="release-status__title">{label}</strong>
          {strength && <span className="release-status__strength">{strength}</span>}
          <ul className="release-status__reasons">
            {evidence.map((e) => (
              <EvidenceRow key={`${e.kind}:${e.text}`} e={e} />
            ))}
          </ul>
          {status.status === "plexMissed" && (
            <p className="release-status__hint">You can fix it with Edit, then Fix match, on this album.</p>
          )}
          <div className="release-status__actions">
            {status.mark ? (
              <Button size="sm" disabled={busy} onClick={() => void mark(null)}>
                Clear my mark
              </Button>
            ) : (
              <>
                {status.status !== "plexMissed" && (
                  <Button size="sm" variant="primary" disabled={busy} onClick={() => void mark("unreleased")}>
                    Confirm
                  </Button>
                )}
                <Button size="sm" disabled={busy} onClick={() => void mark("released")}>
                  Not unreleased
                </Button>
              </>
            )}
          </div>
          <p className="release-status__fine">Your answer syncs to your other devices. Your files aren't changed.</p>
        </div>
      )}
    </span>
  );
}
