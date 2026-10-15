/**
 * The archive's dialogs, mounted once in the shell: Other versions of a
 * song (album, date, version and quality badges, play any) and Move to
 * section (pick one of the artist's own sections or name a new one).
 */
import { Pause, Play } from "lucide-react";
import { useEffect, useId, useRef, useState } from "react";
import { backend } from "../lib/backend";
import { formatDuration } from "../lib/format";
import type { VersionItem } from "../lib/types";
import { formatArchiveDate, SONG_STATUS_LABEL } from "../pages/era";
import { openLineage, useArchiveUi } from "../stores/archive";
import { useSettings } from "../stores/settings";
import { player, usePlayer } from "../stores/player";
import { nav } from "../stores/router";
import { toast } from "../stores/toasts";
import { Art } from "./Art";
import { Badge } from "./Badge";
import { Button, IconButton } from "./Button";
import { Dialog } from "./Dialog";
import { archiveQualityLabel, versionLabel } from "./FileTagBadges";
import { Skeleton } from "./Skeleton";
import { SoundsLikeHint } from "./SoundsLikeHint";
import "./archive.css";

export function ArchiveDialogs() {
  return (
    <>
      <VersionsHost />
      <MoveHost />
    </>
  );
}

function VersionsHost() {
  const t = useArchiveUi((s) => s.versionsOf);
  if (!t) return null;
  return <VersionsDialog key={t.id} trackId={t.id} title={t.title} onClose={() => useArchiveUi.getState().set({ versionsOf: null })} />;
}

/** The line under a version's title: album, then date (else year), then edition. */
function versionLine(v: VersionItem): string {
  const when = v.date ? formatArchiveDate(v.date) : v.year ? String(v.year) : null;
  return [v.track.albumTitle, when, v.edition].filter(Boolean).join(" · ");
}

export function VersionsDialog({ trackId, title, onClose }: { trackId: string; title: string; onClose: () => void }) {
  const [items, setItems] = useState<VersionItem[] | null>(null);
  const [failed, setFailed] = useState(false);
  const nowId = usePlayer((s) => s.state?.track?.id);
  const status = usePlayer((s) => s.state?.status);
  const headingId = useId();
  const showLineage = useSettings((s) => s.settings?.showLineage !== false);
  useEffect(() => {
    let alive = true;
    backend.trackVersions(trackId).then(
      (v) => alive && setItems(v),
      () => alive && setFailed(true),
    );
    return () => {
      alive = false;
    };
  }, [trackId]);

  const others = items ? items.length - 1 : 0;
  const play = (v: VersionItem) =>
    v.track.id === nowId ? player.toggle() : void player.playTracks(items!.map((x) => x.track.id), v.track.id);

  return (
    <Dialog open onClose={onClose} label={`Versions of ${title}`} className="versions">
      <header className="versions__head">
        <h2 id={headingId} className="versions__title">
          Versions of {title}
        </h2>
        <p className="versions__sub">{items ? (others === 1 ? "1 other version in your library" : `${others} other versions in your library`) : " "}</p>
        <SoundsLikeHint trackId={trackId} onOpen={onClose} />
      </header>
      {failed ? (
        <p className="versions__empty">Couldn't load the versions.</p>
      ) : (
        <ul className="versions__list" aria-labelledby={headingId} aria-busy={!items || undefined}>
          {items
            ? items.map((v) => {
                const playingThis = v.track.id === nowId && status === "playing";
                const version = versionLabel(v.version);
                const quality = archiveQualityLabel(v.quality, v.track);
                return (
                  <li key={v.track.id} className={`versions__row ${v.current ? "is-current" : ""}`} aria-current={v.current || undefined}>
                    <IconButton
                      label={playingThis ? `Pause ${v.track.title}` : `Play ${v.track.title}`}
                      size="md"
                      variant="filled"
                      icon={playingThis ? <Pause size={16} fill="currentColor" /> : <Play size={16} fill="currentColor" className="nudge-right" />}
                      onClick={() => play(v)}
                    />
                    <Art path={v.track.thumb} size={48} />
                    <div className="versions__text">
                      <div className="versions__name">
                        <span className="truncate">{v.track.title}</span>
                        {v.current && <span className="versions__here">This one</span>}
                      </div>
                      <div className="versions__meta">
                        <button
                          type="button"
                          className="link truncate"
                          onClick={() => {
                            onClose();
                            nav.go({ name: "album", id: v.track.albumId });
                          }}
                        >
                          {versionLine(v)}
                        </button>
                      </div>
                    </div>
                    <div className="versions__badges">
                      {v.status !== "released" && (
                        <Badge tone={v.status === "unreleased" ? "version" : "neutral"} title={SONG_STATUS_LABEL[v.status]}>
                          {v.status === "probablyAlternate" ? "Alternate" : "Unreleased"}
                        </Badge>
                      )}
                      {v.status === "released" && <Badge tone="neutral">Released</Badge>}
                      {version && <Badge tone="version">{version}</Badge>}
                      {quality && <Badge tone="neutral">{quality}</Badge>}
                    </div>
                    <span className="versions__duration tabular">{formatDuration(v.track.durationMs)}</span>
                  </li>
                );
              })
            : Array.from({ length: 3 }, (_, i) => (
                <li key={i} className="versions__row" aria-hidden="true">
                  <Skeleton width={32} height={32} radius="circle" />
                  <Skeleton width={48} height={48} />
                  <div className="versions__text">
                    <Skeleton width="50%" height={14} />
                    <Skeleton width="70%" height={12} style={{ marginTop: 6 }} />
                  </div>
                </li>
              ))}
        </ul>
      )}
      <div className="versions__actions">
        {showLineage && items && items.length > 1 && (
          <Button
            variant="ghost"
            onClick={() => {
              onClose();
              openLineage(trackId);
            }}
          >
            See lineage
          </Button>
        )}
        <Button variant="secondary" onClick={onClose}>
          Close
        </Button>
      </div>
    </Dialog>
  );
}

function MoveHost() {
  const a = useArchiveUi((s) => s.moving);
  if (!a) return null;
  return (
    <MoveToSectionDialog
      key={a.id}
      albumId={a.id}
      artistId={a.artistId}
      albumTitle={a.title}
      onClose={() => useArchiveUi.getState().set({ moving: null })}
    />
  );
}

export const SECTION_NAME_MAX = 40;

/** Pick one of the artist's own sections, name a new one, or send the album back to its own section. */
export function MoveToSectionDialog({
  albumId,
  artistId,
  albumTitle,
  onClose,
}: {
  albumId: string;
  artistId: string;
  albumTitle: string;
  onClose: () => void;
}) {
  const [names, setNames] = useState<string[] | null>(null);
  const [name, setName] = useState("");
  const [error, setError] = useState<string | null>(null);
  const input = useRef<HTMLInputElement>(null);
  const inputId = useId();
  useEffect(() => {
    let alive = true;
    backend.artistSections(artistId).then(
      (n) => alive && setNames(n),
      () => alive && setNames([]),
    );
    return () => {
      alive = false;
    };
  }, [artistId]);

  const save = async (to: string | null) => {
    if (to !== null && !to.trim()) {
      setError("Give the section a name.");
      return;
    }
    try {
      await backend.albumSectionSet(albumId, artistId, to === null ? null : to.trim());
      toast.info(to === null ? `${albumTitle} is back in its own section` : `Moved to ${to.trim()}`);
      onClose();
    } catch (e) {
      setError((e as { message?: string }).message ?? "Couldn't move it.");
    }
  };

  return (
    <Dialog open onClose={onClose} label="Move to section" className="move-section" initialFocus={input}>
      <h2 className="move-section__title">Move to section</h2>
      <p className="move-section__body">"{albumTitle}" shows in this section on the artist page and in its discography. Your files aren't changed.</p>
      {names && names.length > 0 && (
        <div className="move-section__names" role="group" aria-label="Your sections">
          {names.map((n) => (
            <button key={n} type="button" className="chip" onClick={() => void save(n)}>
              {n}
            </button>
          ))}
        </div>
      )}
      <form
        className="move-section__new"
        onSubmit={(e) => {
          e.preventDefault();
          void save(name);
        }}
      >
        <label htmlFor={inputId} className="move-section__label">
          New section
        </label>
        <input
          id={inputId}
          ref={input}
          className="pl-edit__input"
          value={name}
          maxLength={SECTION_NAME_MAX}
          placeholder="Name it, like Vault picks"
          onChange={(e) => {
            setName(e.target.value);
            setError(null);
          }}
        />
        {error && (
          <p className="move-section__error" role="alert">
            {error}
          </p>
        )}
        <div className="confirm__actions">
          <Button variant="ghost" onClick={() => void save(null)}>
            Back to its own section
          </Button>
          <Button type="submit" variant="primary" disabled={!name.trim()}>
            Move
          </Button>
        </div>
      </form>
    </Dialog>
  );
}
