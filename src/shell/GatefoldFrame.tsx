/**
 * Gatefold: two pages like an open record sleeve. The left page is now
 * playing (a big cover, the song, a few lines of the record's sleeve notes,
 * and lyrics or credits); the right page is whatever you're browsing, and it
 * turns like a page when you go somewhere else. The spine between them holds
 * the transport, the volume knob and the queue handle. In a narrow window
 * the left page folds into the spine as a small now playing.
 *
 * The controls are the player bar's own, so names, keys and tab stops don't
 * change. The page turn is a slide with a soft crease shadow, or a fade with
 * reduced motion, and can be turned off in Settings.
 */
import { useEffect, useId, useLayoutEffect, useRef, useState, type RefObject } from "react";
import { Art } from "../components/Art";
import { ArtistLinks, TextLink } from "../components/ArtistLinks";
import { ExplicitMark } from "../components/FileTagBadges";
import { openContextMenu } from "../components/ContextMenu";
import { SaveButton } from "../components/SaveButton";
import { Skeleton } from "../components/Skeleton";
import { trackMenuItems } from "../components/TrackRow";
import { prefersReducedMotion } from "../design/motion";
import { trackArtistLine } from "../lib/format";
import { formatSleeveDate } from "../lib/sleeve";
import type { SleeveNotes, Track } from "../lib/types";
import { useSleeveNotes } from "../lib/useSleeveNotes";
import { endSongDrag, startSongDrag, tracksDrag } from "../stores/drag";
import { fullWindow, useFullWindow } from "../stores/fullWindow";
import { usePlayer } from "../stores/player";
import { nav, useRouter } from "../stores/router";
import { useSettings } from "../stores/settings";
import { useLayout } from "../stores/theme";
import { useUi } from "../stores/ui";
import { useCreditRows } from "./creditRows";
import { currentLineIndex, timedLines, useLyrics } from "./lyrics";
import { NowPlayingAnnouncer, PlayControls, QueueButton, SeekBar } from "./PlayerBar";
import { SignalBadge, SignalPanel } from "./SignalPath";
import { VolumeKnob } from "./DeckFrame";
import { StripExtras } from "./TransportStrip";
import "./gatefold.css";

// ---------------------------------------------------------------- the page turn

export type TurnDirection = "forward" | "back";

/** How long a page turn takes (ms): a slide, or a shorter fade with reduced motion. */
export const PAGE_TURN_MS = 360;
export const PAGE_FADE_MS = 200;

/**
 * The page turn's keyframes: the page slides in from the side it came from
 * while a soft crease shadow by the spine fades away. Reduced motion is a
 * plain fade with no movement and no crease.
 */
export function pageTurnFrames(dir: TurnDirection, reduced: boolean): { ms: number; page: Keyframe[]; crease: Keyframe[] | null } {
  if (reduced) return { ms: PAGE_FADE_MS, page: [{ opacity: 0 }, { opacity: 1 }], crease: null };
  const from = dir === "forward" ? "40px" : "-40px";
  return {
    ms: PAGE_TURN_MS,
    page: [
      { transform: `translateX(${from})`, opacity: 0.35 },
      { transform: "translateX(0)", opacity: 1 },
    ],
    crease: [{ opacity: 1 }, { opacity: 0 }],
  };
}

const TURN_EASING = "cubic-bezier(0.22, 0.8, 0.24, 1)";

/**
 * Turns `page` when you go to another page (not when the same page only
 * updates, like a search query). Back turns the other way. The direction is
 * left on the page as `data-turn` while it runs.
 */
export function usePageTurn(page: RefObject<HTMLElement | null>, crease: RefObject<HTMLElement | null>, on: boolean) {
  const key = useRouter((s) => s.entries[s.index]?.key);
  const index = useRouter((s) => s.index);
  const last = useRef({ key, index });
  useLayoutEffect(() => {
    const prev = last.current;
    last.current = { key, index };
    const el = page.current;
    if (!on || !el || prev.key === key) return;
    const reduced = prefersReducedMotion();
    const dir: TurnDirection = index < prev.index ? "back" : "forward";
    const plan = pageTurnFrames(dir, reduced);
    el.dataset.turn = reduced ? "fade" : dir;
    const done = () => {
      if (el.dataset.turn) delete el.dataset.turn;
    };
    if (typeof el.animate !== "function") {
      const t = window.setTimeout(done, plan.ms);
      return () => window.clearTimeout(t);
    }
    const a = el.animate(plan.page, { duration: plan.ms, easing: TURN_EASING });
    const c = plan.crease && crease.current?.animate ? crease.current.animate(plan.crease, { duration: plan.ms, easing: "ease-out" }) : null;
    a.onfinish = done;
    a.oncancel = done;
    return () => {
      a.cancel();
      c?.cancel();
    };
  }, [key, index, on, page, crease]);
}

// ---------------------------------------------------------------- the left page

/** The cover, which opens the now playing view (and the expanded view on a double click). */
function Cover({ track, size }: { track: Track; size: number }) {
  const expanded = useFullWindow((s) => s.open);
  const nowPlayingOpen = useUi((s) => s.nowPlayingOpen) && !expanded;
  return (
    <button
      type="button"
      className="gf-cover"
      aria-label="Now playing view"
      aria-pressed={nowPlayingOpen}
      onClick={(e) => e.detail < 2 && useUi.getState().toggleNowPlaying()}
      onDoubleClick={() => {
        useUi.getState().toggleNowPlaying();
        fullWindow.open();
      }}
    >
      <Art path={track.thumb} fluid requestSize={size} />
    </button>
  );
}

/** The song's title (to its album), artists and the like key. */
function SongLines({ track, big }: { track: Track; big: boolean }) {
  const like = useLayout().playerButtons.includes("like");
  return (
    <div className={`gf-song ${big ? "gf-song--big" : ""}`}>
      <div className="gf-song__text">
        <div
          className="gf-song__title truncate"
          onContextMenu={(e) => {
            e.preventDefault();
            openContextMenu(`Options for ${track.title}`, trackMenuItems(track), e);
          }}
        >
          <TextLink onActivate={() => nav.go({ name: "album", id: track.albumId })}>{track.title}</TextLink>
          <ExplicitMark trackId={track.id} />
        </div>
        <ArtistLinks track={track} className="gf-song__artist truncate" />
        {big && track.albumTitle && (
          <TextLink className="gf-song__album truncate" onActivate={() => nav.go({ name: "album", id: track.albumId })}>
            {track.albumTitle}
          </TextLink>
        )}
      </div>
      {like && <SaveButton track={track} place="bar" />}
    </div>
  );
}

/** The facts a sleeve prints first: when it came out, on what label, where it was made. */
export function sleeveLine(n: SleeveNotes): string | null {
  const parts = [
    n.released ? `Released ${formatSleeveDate(n.released.value)}` : null,
    n.label ? `on ${n.label.value}` : null,
  ].filter(Boolean);
  const first = parts.join(" ");
  const studio = n.studios.find((s) => s.role === "recorded") ?? n.studios[0];
  const second = studio ? `Recorded at ${studio.name}` : n.recorded ? `Recorded ${formatSleeveDate(n.recorded.value)}` : null;
  const out = [first || null, second].filter(Boolean).join(". ");
  return out ? `${out}.` : null;
}

/** A few lines of the record's sleeve notes, with the way to all of them. */
function SleeveSnippet({ track }: { track: Track }) {
  const on = useSettings((s) => s.settings?.showSleeveNotes !== false);
  const { data, loading } = useSleeveNotes(track.albumId, on);
  if (!on) return null;
  const line = data ? sleeveLine(data) : null;
  const notes = data?.notes?.value.trim() || null;
  return (
    <section className="gf-sleeve" aria-label="Sleeve notes">
      <h2 className="gf-label">Sleeve notes</h2>
      {loading && !data ? (
        <div className="gf-sleeve__loading" aria-hidden="true">
          <Skeleton width="90%" height={12} />
          <Skeleton width="70%" height={12} style={{ marginTop: 6 }} />
        </div>
      ) : line || notes ? (
        <>
          {line && <p className="gf-sleeve__facts">{line}</p>}
          {notes && <p className="gf-sleeve__notes">{notes}</p>}
          <button type="button" className="gf-link" onClick={() => nav.go({ name: "album", id: track.albumId, tab: "sleeve" })}>
            Read the sleeve notes
          </button>
        </>
      ) : (
        <p className="gf-empty">{data?.looking ? "Looking these up." : "No sleeve notes for this record yet."}</p>
      )}
    </section>
  );
}

/** The song's lyrics as text, the line playing marked when they're timed. */
function LyricsText({ track }: { track: Track }) {
  const { lyrics, loading } = useLyrics(track.id);
  const timed = timedLines(lyrics);
  const position = usePlayer((s) => s.positionMs);
  const at = timed ? currentLineIndex(timed, position) : -1;
  const box = useRef<HTMLDivElement>(null);
  // Keep the playing line in view, inside the box only (the page doesn't move).
  useEffect(() => {
    const el = box.current?.querySelector<HTMLElement>(".gf-lyrics__line.is-now");
    const b = box.current;
    if (!el || !b) return;
    const top = el.offsetTop - b.clientHeight / 3;
    b.scrollTo?.({ top: Math.max(0, top), behavior: prefersReducedMotion() ? "auto" : "smooth" });
  }, [at]);
  if (loading && !lyrics) {
    return (
      <div className="gf-lyrics" aria-hidden="true">
        <Skeleton width="80%" height={14} />
        <Skeleton width="60%" height={14} style={{ marginTop: 8 }} />
        <Skeleton width="70%" height={14} style={{ marginTop: 8 }} />
      </div>
    );
  }
  const lines = timed ? timed.map((l) => l.text) : (lyrics?.lines ?? []);
  if (!lines.some((l) => l.trim())) return <p className="gf-empty">No lyrics for this song.</p>;
  return (
    <>
      <div className="gf-lyrics" ref={box} tabIndex={0} aria-label={`Lyrics for ${track.title}`}>
        {lines.map((l, i) => (
          <p key={i} className={`gf-lyrics__line ${i === at ? "is-now" : ""} ${l.trim() ? "" : "is-gap"}`}>
            {l}
          </p>
        ))}
      </div>
      <button type="button" className="gf-link" onClick={() => fullWindow.open("lyrics")}>
        Open the lyrics view
      </button>
    </>
  );
}

/** Who made the song. */
function CreditsText({ track }: { track: Track }) {
  const { rows, loading } = useCreditRows(track);
  if (!rows.length && loading) {
    return (
      <div aria-hidden="true">
        <Skeleton width="70%" height={13} />
        <Skeleton width="50%" height={13} style={{ marginTop: 8 }} />
      </div>
    );
  }
  if (!rows.length) return <p className="gf-empty">No credits for this song yet.</p>;
  return (
    <ul className="gf-credits">
      {rows.map((r) => (
        <li key={r.name} className="gf-credits__row">
          <span className="gf-credits__name truncate">{r.id ? <TextLink onActivate={() => nav.go({ name: "artist", id: r.id! })}>{r.name}</TextLink> : r.name}</span>
          <span className="gf-credits__roles">{r.roles.join(", ")}</span>
        </li>
      ))}
    </ul>
  );
}

type LeafTab = "lyrics" | "credits";

/** Lyrics or credits, as two tabs. */
function LyricsOrCredits({ track }: { track: Track }) {
  const [tab, setTab] = useState<LeafTab>("lyrics");
  const id = useId();
  const tabs: { id: LeafTab; label: string }[] = [
    { id: "lyrics", label: "Lyrics" },
    { id: "credits", label: "Credits" },
  ];
  const onKey = (e: React.KeyboardEvent) => {
    if (e.key !== "ArrowLeft" && e.key !== "ArrowRight") return;
    e.preventDefault();
    const next: LeafTab = tab === "lyrics" ? "credits" : "lyrics";
    setTab(next);
    document.getElementById(`${id}-${next}`)?.focus();
  };
  return (
    <section className="gf-words">
      <div className="gf-tabs" role="tablist" aria-label="Lyrics or credits" onKeyDown={onKey}>
        {tabs.map((t) => (
          <button
            key={t.id}
            id={`${id}-${t.id}`}
            type="button"
            role="tab"
            className={`gf-tab ${tab === t.id ? "is-on" : ""}`}
            aria-selected={tab === t.id}
            aria-controls={`${id}-panel`}
            tabIndex={tab === t.id ? 0 : -1}
            onClick={() => setTab(t.id)}
          >
            {t.label}
          </button>
        ))}
      </div>
      <div id={`${id}-panel`} className="gf-words__panel" role="tabpanel" aria-labelledby={`${id}-${tab}`}>
        {tab === "lyrics" ? <LyricsText track={track} /> : <CreditsText track={track} />}
      </div>
    </section>
  );
}

/** The left page: now playing, with sleeve notes and lyrics or credits. */
export function GatefoldLeaf() {
  const state = usePlayer((s) => s.state);
  const songDrags = useSettings((s) => s.settings?.dragAndDrop !== false);
  const track = state?.track;
  const nowText = track ? `Now playing: ${track.title} by ${trackArtistLine(track)}` : "Nothing playing";
  const dragNow =
    track && songDrags ? { draggable: true, onDragStart: (e: React.DragEvent) => void startSongDrag(e, tracksDrag([track])), onDragEnd: endSongDrag } : null;
  return (
    <section className="gf-leaf" aria-label="Now playing" data-testid="gatefold-leaf">
      <div className="gf-leaf__scroll">
        {track ? (
          <>
            <div className="gf-leaf__now" role="region" aria-label={nowText} {...dragNow}>
              <Cover track={track} size={480} />
              <SongLines track={track} big />
              <div className="gf-leaf__line">
                <SeekBar strip />
              </div>
              <div className="gf-leaf__plates">
                <SignalBadge />
              </div>
            </div>
            <SleeveSnippet track={track} />
            <LyricsOrCredits track={track} />
          </>
        ) : state === null ? (
          <div aria-hidden="true">
            <Skeleton width="100%" height={280} />
            <Skeleton width="70%" height={22} style={{ marginTop: 16 }} />
            <Skeleton width="45%" height={14} style={{ marginTop: 8 }} />
          </div>
        ) : (
          <div className="gf-leaf__idle">
            <span className="gf-leaf__sleeve" aria-hidden="true" />
            <p className="gf-leaf__idle-title">Nothing on the turntable</p>
            <p className="gf-empty">Pick a record on the right and its sleeve opens here.</p>
          </div>
        )}
      </div>
    </section>
  );
}

// ---------------------------------------------------------------- the spine

/**
 * The spine: transport keys, the volume knob, the other player keys and the
 * queue handle. Folded, it also carries a small now playing (cover, song,
 * tonearm line) in place of the left page. Its label and controls match the
 * player bar's.
 */
export function GatefoldSpine({ folded }: { folded: boolean }) {
  const state = usePlayer((s) => s.state);
  const track = state?.track;
  const queueOpen = useUi((s) => s.queueOpen);
  const nowText = track ? `Now playing: ${track.title} by ${trackArtistLine(track)}` : null;
  return (
    <aside className={`gf-spine ${folded ? "is-folded" : ""}`} aria-label="Player" data-testid="gatefold-spine">
      <NowPlayingAnnouncer text={nowText} />
      {folded && (
        <div className="gf-spine__now" role="region" aria-label={nowText ?? "Nothing playing"}>
          {track ? (
            <>
              <Cover track={track} size={240} />
              <SongLines track={track} big={false} />
            </>
          ) : state === null ? (
            <Skeleton width="100%" height={160} />
          ) : (
            <p className="gf-empty">Nothing playing. Pick a record.</p>
          )}
          <div className="gf-leaf__line">
            <SeekBar strip />
          </div>
        </div>
      )}
      <div className="gf-spine__keys">
        <PlayControls />
      </div>
      <div className="gf-spine__knob">
        <VolumeKnob />
      </div>
      <div className="gf-spine__extras">
        <StripExtras skip={["volume", "queue"]} />
      </div>
      {/* The queue handle: a tab on the spine, always there, that opens the queue drawer. */}
      <div className={`gf-handle ${queueOpen ? "is-open" : ""}`}>
        <QueueButton />
        <span className="gf-handle__label" aria-hidden="true">
          Queue
        </span>
      </div>
      <SignalPanel />
    </aside>
  );
}
