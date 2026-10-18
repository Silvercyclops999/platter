/**
 * Home led by records, in Platter's own looks (Studio, Studio Black,
 * Sleeve): the album you were in the middle of as a sleeve with its record
 * half out, then shelves of records. The core works out what goes where
 * (`home_records`) and keeps the answer until the plays or the library
 * change; each shelf hides when it has nothing, the hero when nothing is
 * left to resume.
 */
import { Pause, Play } from "lucide-react";
import { useEffect, useId, useLayoutEffect, useRef, useState, type CSSProperties, type KeyboardEvent, type ReactNode } from "react";
import { Art } from "../components/Art";
import { Button } from "../components/Button";
import { identityOf } from "../design/themes";
import { backend } from "../lib/backend";
import { headerColorsFromServer } from "../lib/color";
import { timeAgo } from "../lib/relativeTime";
import type { Album, RecordHome, RecordProgress } from "../lib/types";
import { useApp } from "../stores/app";
import { player, usePlayer } from "../stores/player";
import { nav } from "../stores/router";
import { useSettings } from "../stores/settings";
import { useTheme } from "../stores/theme";
import { useAlbumActions, useContextState } from "./cards";
import "./record-home.css";

/** The look in use is one of Platter's own (Studio, Studio Black, Sleeve). */
export function useStudioLook(): boolean {
  return useTheme((s) => identityOf(s.resolved.id) !== "");
}

/** Home leads with records: one of Platter's own looks, with the setting on. */
export function useRecordsLead(): boolean {
  const studio = useStudioLook();
  const on = useSettings((s) => s.settings?.homeRecordsFirst !== false);
  return studio && on;
}

/**
 * The record shelves while `on` and the server is there; read again as the
 * library changes and as songs change (the core answers from its cache
 * when nothing moved). Null while off or loading.
 */
export function useRecordHome(on: boolean): RecordHome | null {
  const libraryVersion = useApp((s) => s.libraryVersion);
  const cachedOnly = useApp((s) => !!s.health?.cachedOnly);
  const trackId = usePlayer((s) => s.state?.track?.id ?? null);
  const live = on && !cachedOnly;
  const [data, setData] = useState<RecordHome | null>(null);
  useEffect(() => {
    if (!live) return;
    let alive = true;
    backend.homeRecords().then(
      (d) => alive && setData(d),
      () => alive && setData(null),
    );
    return () => {
      alive = false;
    };
  }, [live, libraryVersion, trackId]);
  return live && data?.shown ? data : null;
}

/** Whether a record Home has anything to show. */
export function hasRecords(d: RecordHome | null): boolean {
  return !!d && (!!d.resume || d.newArrivals.length + d.spunThisWeek.length + d.unfinished.length + d.notInAYear.length > 0);
}

/** "Picks up at song 6 of 10, Title." */
export function resumeLine(p: RecordProgress, playing: boolean): string {
  return playing ? `Song ${p.nextNumber} of ${p.total}, ${p.nextTitle}.` : `Picks up at song ${p.nextNumber} of ${p.total}, ${p.nextTitle}.`;
}

/** How far through the album the resume point is, 0 to 1. */
export const progressOf = (p: RecordProgress) => Math.min(1, Math.max(0, (p.nextNumber - 1) / Math.max(1, p.total)));

const monthYear = new Intl.DateTimeFormat(undefined, { month: "long", year: "numeric" });

/** The record's colour for its label: the server's tint, or a warm neutral. */
function labelColour(album: Album): string {
  return headerColorsFromServer(album.tint)?.vivid ?? "#8a7a62";
}

/** Play from the resume point, or pause and play again while it's the record playing. */
function useResume(p: RecordProgress) {
  const { isContext, playing } = useContextState("album", p.album.id);
  return {
    isContext,
    playing,
    resume: () => (isContext ? player.toggle() : player.playAlbum(p.album.id, p.nextTrackId)),
  };
}

export function RecordHero({ progress: p }: { progress: RecordProgress }) {
  const id = useId();
  const { isContext, playing, resume } = useResume(p);
  const a = useAlbumActions(p.album);
  const share = progressOf(p);
  const open = () => nav.go({ name: "album", id: p.album.id });
  return (
    <section
      className={`record-hero ${playing ? "is-spinning" : ""}`}
      aria-labelledby={id}
      data-testid="record-hero"
      style={{ "--record-label": labelColour(p.album) } as CSSProperties}
      onContextMenu={a.onContextMenu}
    >
      <div className="record-hero__sleeve">
        <span className="record-hero__disc" aria-hidden="true" />
        <button type="button" className="record-hero__cover" onClick={open} aria-label={`Open ${p.album.title}`} tabIndex={-1}>
          <Art path={p.album.thumb} fluid requestSize={320} kind="album" />
        </button>
      </div>
      <div className="record-hero__body">
        <p className="record-hero__eyebrow">{isContext ? "Now spinning" : "Pick up where you left off"}</p>
        <h2 className="record-hero__title" id={id}>
          <button type="button" className="record-hero__link" onClick={open}>
            {p.album.title}
          </button>
        </h2>
        <p className="record-hero__artist">
          <button type="button" className="record-hero__artist-link" onClick={() => nav.go({ name: "artist", id: p.album.artistId })}>
            {p.album.artistTitle}
          </button>
          {p.album.year ? <span> · {p.album.year}</span> : null}
        </p>
        <div className="record-hero__where">
          <div
            className="record-groove record-groove--hero"
            role="progressbar"
            aria-label={`Song ${p.nextNumber} of ${p.total}`}
            aria-valuemin={0}
            aria-valuemax={p.total}
            aria-valuenow={p.nextNumber - 1}
          >
            <span className="record-groove__fill" style={{ width: `${share * 100}%` }} />
          </div>
          <p className="record-hero__next">{resumeLine(p, isContext)}</p>
        </div>
        <div className="record-hero__actions">
          <Button
            variant="primary"
            icon={playing ? <Pause size={18} fill="currentColor" /> : <Play size={18} fill="currentColor" />}
            onClick={() => void resume()}
          >
            {playing ? "Pause" : isContext ? "Play" : "Resume"}
          </Button>
          <Button variant="secondary" onClick={open}>
            Open album
          </Button>
        </div>
      </div>
    </section>
  );
}

interface SleeveCardProps {
  album: Album;
  /** A short line under the artist ("6 plays this week"). */
  meta: string;
  /** Where Play starts, for records stopped partway. */
  progress?: RecordProgress;
  /** Under the meta line (a crate's "Sounds like this record"). */
  extra?: ReactNode;
}

/** An album as a record sleeve: the disc slides out a little on hover and focus. */
export function SleeveCard({ album, meta, progress, extra }: SleeveCardProps) {
  const a = useAlbumActions(album);
  const { isContext, playing } = useContextState("album", album.id);
  const play = () => (isContext ? player.toggle() : progress ? player.playAlbum(album.id, progress.nextTrackId) : player.playAlbum(album.id));
  const verb = playing ? "Pause" : progress && !isContext ? "Resume" : "Play";
  return (
    <div
      role="listitem"
      className={`sleeve-card ${playing ? "is-playing" : ""}`}
      style={{ "--record-label": labelColour(album) } as CSSProperties}
      onContextMenu={a.onContextMenu}
    >
      <div className="sleeve-card__art">
        <span className="sleeve-card__disc" aria-hidden="true" />
        <span className="sleeve-card__sleeve">
          <Art path={album.thumb} fluid requestSize={220} kind="album" />
        </span>
        <button
          type="button"
          className="sleeve-card__play"
          aria-label={`${verb} ${album.title}`}
          onClick={(e) => {
            e.stopPropagation();
            void play();
          }}
        >
          {playing ? <Pause size={18} fill="currentColor" aria-hidden="true" /> : <Play size={18} fill="currentColor" aria-hidden="true" />}
        </button>
      </div>
      <button type="button" className="sleeve-card__link" onClick={a.open} title={album.title}>
        <span className="sleeve-card__title truncate">{album.title}</span>
      </button>
      <div className="sleeve-card__artist truncate">{album.artistTitle}</div>
      <div className="sleeve-card__meta truncate">{meta}</div>
      {progress && (
        <div
          className="record-groove"
          role="progressbar"
          aria-label={`${album.title}: song ${progress.nextNumber} of ${progress.total}`}
          aria-valuemin={0}
          aria-valuemax={progress.total}
          aria-valuenow={progress.nextNumber - 1}
        >
          <span className="record-groove__fill" style={{ width: `${progressOf(progress) * 100}%` }} />
        </div>
      )}
      {extra}
    </div>
  );
}

/** Cards in a calm grid row from this width up; a scrolling row below it. */
export const GRID_FROM = 900;
const CARD_MIN = 168;
const GAP = 28;

/** Columns for the grid at a width, or 0 for the scrolling row. */
export function gridColumns(width: number): number {
  if (width < GRID_FROM) return 0;
  return Math.min(7, Math.max(4, Math.floor((width + GAP) / (CARD_MIN + GAP))));
}

/** Left, Right, Home and End move between a shelf's cards. */
function moveBetweenCards(e: KeyboardEvent<HTMLElement>) {
  const links = Array.from(e.currentTarget.querySelectorAll<HTMLElement>(".sleeve-card__link"));
  const card = (e.target as HTMLElement).closest(".sleeve-card");
  const at = links.findIndex((l) => l.closest(".sleeve-card") === card);
  if (at < 0) return;
  const next = e.key === "ArrowRight" ? at + 1 : e.key === "ArrowLeft" ? at - 1 : e.key === "Home" ? 0 : e.key === "End" ? links.length - 1 : -1;
  if (next < 0 || next >= links.length || (next === at && e.key !== "Home" && e.key !== "End")) return;
  e.preventDefault();
  links[next].focus();
  links[next].scrollIntoView?.({ block: "nearest", inline: "nearest" });
}

export function RecordShelf({
  title,
  line,
  testId,
  open,
  children,
}: {
  title: string;
  line: string;
  testId: string;
  /** A link to the shelf's own page, in place of Show all. */
  open?: { label: string; run: () => void };
  children: ReactNode[];
}) {
  const id = useId();
  const box = useRef<HTMLElement>(null);
  const [cols, setCols] = useState(0);
  const [all, setAll] = useState(false);
  useLayoutEffect(() => {
    const el = box.current;
    if (!el) return;
    const measure = () => setCols(gridColumns(el.clientWidth));
    measure();
    if (typeof ResizeObserver === "undefined") return;
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    return () => ro.disconnect();
  }, []);
  if (!children.length) return null;
  const grid = cols > 0;
  const more = grid && children.length > cols;
  const shown = grid && !all ? children.slice(0, cols) : children;
  return (
    <section className="record-shelf" aria-labelledby={id} ref={box} data-testid={testId}>
      <header className="record-shelf__head">
        <div>
          <h2 className="record-shelf__title" id={id}>
            {title}
          </h2>
          <p className="record-shelf__line">{line}</p>
        </div>
        {open ? (
          <button type="button" className="shelf__all" onClick={open.run}>
            {open.label}
          </button>
        ) : more && (
          <button type="button" className="shelf__all" aria-expanded={all} onClick={() => setAll((v) => !v)}>
            {all ? "Show less" : "Show all"}
          </button>
        )}
      </header>
      <div
        className={`record-shelf__items ${grid ? "is-grid" : "is-row"}`}
        role="list"
        aria-label={title}
        style={grid ? ({ "--record-cols": cols } as CSSProperties) : undefined}
        onKeyDown={moveBetweenCards}
      >
        {shown}
      </div>
    </section>
  );
}

const plays = (n: number, what: string) => `${n} ${n === 1 ? "play" : "plays"} ${what}`;

/** The four record shelves, in order. */
export function RecordShelves({ data }: { data: RecordHome }) {
  const nowS = Date.now() / 1000;
  return (
    <>
      <RecordShelf title="New arrivals" line="Added to your library in the last three months." testId="records-new">
        {data.newArrivals.map((album) => (
          <SleeveCard key={album.id} album={album} meta={`Added ${timeAgo(album.addedAt, nowS).toLowerCase()}`} />
        ))}
      </RecordShelf>
      <RecordShelf title="Spun this week" line="The records you played most in the last seven days." testId="records-spun">
        {data.spunThisWeek.map((s) => (
          <SleeveCard key={s.album.id} album={s.album} meta={plays(s.plays, "this week")} />
        ))}
      </RecordShelf>
      <RecordShelf title="Finish what you started" line="Records you stopped partway through. Each one picks up where you left off." testId="records-unfinished">
        {data.unfinished.map((p) => (
          <SleeveCard key={p.album.id} album={p.album} meta={`Song ${p.nextNumber} of ${p.total}`} progress={p} />
        ))}
      </RecordShelf>
      <RecordShelf title="Not played in a year" line="Records you used to play a lot and haven't put on since." testId="records-lapsed">
        {data.notInAYear.map((l) => (
          <SleeveCard key={l.album.id} album={l.album} meta={`Last played ${monthYear.format(new Date(l.lastPlayed))}`} />
        ))}
      </RecordShelf>
    </>
  );
}

/** Under the record shelves: the way into the crates (while Show crates is on). */
export function CratesLink() {
  const on = useSettings((s) => s.settings?.showCrates !== false);
  if (!on) return null;
  return (
    <div className="record-crates" data-testid="home-crates">
      <p className="record-crates__line">Dig through the rest by label, producer, studio and era.</p>
      <Button variant="secondary" size="sm" onClick={() => nav.go({ name: "crates" })}>
        Open the crates
      </Button>
    </div>
  );
}
