import { useEffect, useId, useRef, useState } from "react";
import { Badge } from "../components/Badge";
import { backend } from "../lib/backend";
import type { AlbumReleaseStatus } from "../lib/types";
import { useSettings } from "../stores/settings";

/**
 * An album's release status from the core. Reloads when the MusicBrainz
 * check changes it or the settings that feed it change. Failures show
 * nothing.
 */
export function useReleaseStatus(albumId: string | null | undefined): AlbumReleaseStatus | null {
  const [status, setStatus] = useState<AlbumReleaseStatus | null>(null);
  const deps = useSettings((s) =>
    s.settings ? `${s.settings.countUnmatchedUnreleased}:${s.settings.checkUnmatchedMusicbrainz}:${s.settings.infoMusicbrainz}` : "",
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
    const un = backend.listen("releasestatus://changed", (e) => {
      if (e.id === albumId) void load();
    });
    return () => {
      alive = false;
      un.then((f) => f());
    };
  }, [albumId, deps]);
  return status;
}

const LABEL = { probablyUnreleased: "Probably unreleased", plexMissed: "Released, Plex missed it" } as const;

/**
 * The album header's "Probably unreleased" or "Released, Plex missed it"
 * label with a "Why?" popover listing the evidence. Nothing for albums
 * that are plainly released or tagged.
 */
export function ReleaseStatusLabel({ status }: { status: AlbumReleaseStatus | null }) {
  const [open, setOpen] = useState(false);
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

  if (!status || (status.status !== "probablyUnreleased" && status.status !== "plexMissed")) return null;
  const label = LABEL[status.status];

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
          <ul className="release-status__reasons">
            {status.reasons.map((r) => (
              <li key={r}>{r}</li>
            ))}
          </ul>
          {status.status === "plexMissed" && (
            <p className="release-status__hint">You can fix the match in Plex with Fix Match on this album.</p>
          )}
        </div>
      )}
    </span>
  );
}
