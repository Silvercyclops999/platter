import { AudioLines, Image as ImageIcon, PenLine } from "lucide-react";
import { useCallback, useEffect, useLayoutEffect, useRef, useState, type KeyboardEvent, type ReactNode } from "react";
import { Button } from "../components/Button";
import { Skeleton } from "../components/Skeleton";
import { prefersReducedMotion } from "../design/motion";
import { usePageVisible } from "../design/pageVisibility";
import { NEUTRAL_HEADER } from "../lib/color";
import type { LyricLine, Lyrics, Track } from "../lib/types";
import { openLyricsEditor, useLyricsEditor } from "../stores/lyricsEditor";
import { player, usePlayer } from "../stores/player";
import { openShareCard, useLyricPick } from "../stores/shareCard";
import { LYRIC_LINES_MAX } from "../lib/shareCard";
import { trackArtistLine } from "../lib/format";
import { useSettings } from "../stores/settings";
import { LyricExtras, TranslateButton, useLyricExtras, type Extras } from "./lyricExtras";
import {
  currentLineIndex,
  currentPositionMs,
  currentWordIndex,
  followTarget,
  lineState,
  lyricsSourceLabel,
  onSeek,
  setPlayhead,
  timedLines,
  useLyrics,
} from "./lyrics";
import { useCoverColors } from "./nowPlaying";
import { TranscribeOffer } from "./TranscribeOffer";
import { LyricsNotice, LyricsTools, tapLine } from "./LyricsTrust";
import "./lyrics.css";
import "./lyrics-editor.css";

/** Said above lyrics that have no timing. */
export const UNSYNCED_NOTE = "These lyrics aren't synced to the song yet";
/** Said for a live version showing lyrics timed to another version. */
export const BORROWED_NOTE = "Timing is from another version";

/** Colour behind the lyrics: from the cover, or neutral when Artwork colours is off. */
export function useLyricsBackground(thumb: string | null | undefined): string {
  const colours = useSettings((s) => s.settings?.artworkColours !== false);
  const c = useCoverColors(colours ? thumb : null);
  return (colours && c?.vivid) || NEUTRAL_HEADER.vivid;
}

/** Keys that scroll a focused scroller by hand. */
const SCROLL_KEYS = new Set(["ArrowUp", "ArrowDown", "PageUp", "PageDown", "Home", "End"]);

function scrollEl(el: HTMLElement, top: number, smooth: boolean) {
  if (typeof el.scrollTo === "function") el.scrollTo({ top, behavior: smooth ? "smooth" : "auto" });
  else el.scrollTop = top;
}

/** A break in the singing this long or longer is read out as "Instrumental"; shorter ones are verse breaks. */
export const INSTRUMENTAL_GAP_MS = 5000;

/** Line `i` is a wordless break long enough to call instrumental (or the end of the song). */
export function isInstrumentalGap(lines: LyricLine[], i: number): boolean {
  const l = lines[i];
  if (!l || l.text) return false;
  const next = lines[i + 1];
  return !next || next.startMs - l.startMs >= INSTRUMENTAL_GAP_MS;
}

/** What a screen reader says for a line: its words, then its pronunciation and translation. */
function lineLabel(text: string, extras: Extras, i: number): string {
  return [text, extras.pron?.[i], extras.trans?.[i]].filter(Boolean).join(", ");
}

/** The line nearest `i` (forward first) a keyboard can land on. */
function nearestStop(i: number, stop: (i: number) => boolean, n: number): number {
  for (let d = 0; d < n; d++) {
    if (i + d < n && stop(i + d)) return i + d;
    if (i - d >= 0 && stop(i - d)) return i - d;
  }
  return i;
}

/** Which line and word are playing, updated every frame but only re-rendering on a change. */
function usePlayhead(lines: LyricLine[]): { line: number; word: number; jump: (ms: number) => void } {
  const [pos, setPos] = useState({ line: -1, word: -1 });
  const measure = useCallback(() => {
    const ms = currentPositionMs();
    const line = currentLineIndex(lines, ms);
    const word = line >= 0 ? currentWordIndex(lines[line].words, ms) : -1;
    setPos((p) => (p.line === line && p.word === word ? p : { line, word }));
  }, [lines]);
  // The frame loop runs only while the song plays and the page is visible:
  // paused or hidden, the position ticks alone keep the line right.
  const playing = usePlayer((s) => s.state?.status === "playing");
  const visible = usePageVisible();
  useEffect(() => {
    // The shared playhead (lyrics.ts) has already taken the change in.
    const unsub = usePlayer.subscribe((s, p) => {
      if (s.positionMs !== p.positionMs || s.state?.status !== p.state?.status) measure();
    });
    measure();
    return unsub;
  }, [measure]);
  useEffect(() => {
    if (!playing || !visible) return;
    let raf = 0;
    const tick = () => {
      measure();
      raf = requestAnimationFrame(tick);
    };
    raf = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(raf);
  }, [measure, playing, visible]);
  const jump = useCallback(
    (ms: number) => {
      setPlayhead(ms);
      measure();
    },
    [measure],
  );
  return { ...pos, jump };
}

function SyncedLyrics({
  lines,
  credit,
  className,
  footer,
  tools,
  onTap,
  extras,
}: {
  lines: LyricLine[];
  credit?: string;
  className: string;
  footer?: ReactNode;
  /** Under the lyrics: the check note, timing stepper, Tap to sync and Wrong lyrics?. */
  tools?: ReactNode;
  /** A line clicked while tapping to sync (true: used, don't seek). */
  onTap?: (line: LyricLine) => boolean;
  extras: Extras;
}) {
  const { line: current, word, jump } = usePlayhead(lines);
  const picking = useLyricPick((s) => s.picking);
  const picked = useLyricPick((s) => s.picked);
  const scroller = useRef<HTMLDivElement>(null);
  const lineEls = useRef<(HTMLElement | null)[]>([]);
  const [following, setFollowing] = useState(true);
  const first = useRef(true);
  /** The next placement jumps instead of easing (after a seek). */
  const instant = useRef(false);
  /** The line keyboard focus lands on (one tab stop for the whole list). */
  const [focusIdx, setFocusIdx] = useState<number | null>(null);

  const scrollToCurrent = useCallback(
    (smooth: boolean) => {
      const el = scroller.current;
      if (!el) return;
      const target = lineEls.current[Math.max(0, current)];
      if (!target) return;
      const top = followTarget(target.offsetTop, target.offsetHeight, el.clientHeight, el.scrollHeight);
      scrollEl(el, top, smooth && !prefersReducedMotion());
    },
    [current],
  );

  // Keep the current line a little above the middle (the first placement
  // and a seek jump; line changes ease).
  useLayoutEffect(() => {
    if (!following) return;
    scrollToCurrent(!first.current && !instant.current);
    first.current = false;
    instant.current = false;
  }, [current, following, scrollToCurrent]);

  // Any seek, playing or paused, shows the line it landed on at once.
  useEffect(
    () =>
      onSeek(() => {
        instant.current = true;
        setFollowing(true);
      }),
    [],
  );

  // The layout moved under the current line (the window resized, a
  // translation or pronunciation arrived): put it back at the anchor.
  const followingRef = useRef(following);
  followingRef.current = following;
  const recentre = useRef(scrollToCurrent);
  recentre.current = scrollToCurrent;
  useEffect(() => {
    const el = scroller.current;
    if (!el || typeof ResizeObserver === "undefined") return;
    const ro = new ResizeObserver(() => followingRef.current && recentre.current(false));
    ro.observe(el);
    if (el.firstElementChild) ro.observe(el.firstElementChild);
    return () => ro.disconnect();
  }, []);
  useLayoutEffect(() => {
    if (followingRef.current && !first.current) recentre.current(false);
  }, [extras.trans, extras.pron]);

  const resume = () => setFollowing(true);
  // Scrolling by hand stops following; the view stays put until Sync.
  const pause = () => setFollowing(false);
  /** Lines a keyboard can land on: words, and real instrumental breaks. */
  const stop = (i: number) => !!lines[i]?.text || isInstrumentalGap(lines, i);
  const moveFocus = (from: number, step: number) => {
    let i = from + step;
    while (i >= 0 && i < lines.length && !stop(i)) i += step;
    if (i < 0 || i >= lines.length) return;
    setFocusIdx(i);
    lineEls.current[i]?.focus();
  };
  const onKey = (e: KeyboardEvent) => {
    if (!(e.target instanceof Node) || !scroller.current?.contains(e.target) || !SCROLL_KEYS.has(e.key)) return;
    // From the scroller or a line in it: arrows and page keys scroll by hand.
    pause();
    // On a line, up and down move to the line before or after.
    const at = lineEls.current.indexOf(e.target as HTMLElement);
    if (at >= 0 && (e.key === "ArrowUp" || e.key === "ArrowDown")) {
      e.preventDefault();
      moveFocus(at, e.key === "ArrowUp" ? -1 : 1);
    }
  };
  const tabStop = nearestStop(focusIdx ?? Math.max(0, current), stop, lines.length);

  return (
    <div className={`lyrics lyrics--synced ${className}`}>
      <div className="lyrics__body">
        <div
          ref={scroller}
          className="lyrics__scroll"
          tabIndex={0}
          aria-label="Lyrics"
          onWheel={pause}
          onTouchMove={pause}
          onKeyDown={onKey}
          // A press on the scrollbar itself (not a line) is a hand scroll.
          onPointerDown={(e) => e.target === scroller.current && pause()}
        >
          <div
            className="lyrics__lines"
            // Leaving the list: the tab stop follows the current line again.
            onBlur={(e) => !e.currentTarget.contains(e.relatedTarget as Node | null) && setFocusIdx(null)}
          >
            {lines.map((l, i) => {
              const state = lineState(i, current);
              const words = state === "current" && l.words?.length ? l.words : null;
              // A short break between verses isn't announced; a long one is "Instrumental".
              const hidden = !l.text && !isInstrumentalGap(lines, i);
              return (
                <button
                  key={i}
                  type="button"
                  ref={(el) => {
                    lineEls.current[i] = el;
                  }}
                  dir="auto"
                  className={`lyrics__line is-${state} ${l.text ? "" : "lyrics__line--gap"} ${picking && picked.includes(i) ? "is-picked" : ""}`}
                  tabIndex={i === tabStop && !hidden ? 0 : -1}
                  aria-hidden={hidden || undefined}
                  aria-current={state === "current" ? "true" : undefined}
                  aria-label={l.text ? lineLabel(l.text, extras, i) : hidden ? undefined : "Instrumental"}
                  aria-pressed={picking && l.text ? picked.includes(i) : undefined}
                  disabled={picking && !l.text}
                  onFocus={() => setFocusIdx(i)}
                  onClick={(e) => {
                    if (picking) return useLyricPick.getState().toggle(i);
                    if (onTap?.(l)) return;
                    jump(l.startMs);
                    resume();
                    void player.seek(l.startMs);
                    // After a mouse click, focus goes back to the lyrics so
                    // Space pauses instead of seeking to the line again.
                    if (e.detail > 0) scroller.current?.focus({ preventScroll: true });
                  }}
                >
                  {l.text ? (
                    words ? (
                      words.map((w, j) => (
                        <span key={j} className={`lyrics__word ${j <= word ? "is-sung" : ""}`}>
                          {w.text}
                        </span>
                      ))
                    ) : (
                      l.text
                    )
                  ) : (
                    <span className="lyrics__dots" aria-hidden="true">
                      • • •
                    </span>
                  )}
                  {l.text && <LyricExtras extras={extras} i={i} />}
                </button>
              );
            })}
          </div>
          {credit && <p className="lyrics__credit">{credit}</p>}
          {footer}
        </div>
        {!following && (
          <button type="button" className="lyrics__sync" aria-label="Sync to the current line" onClick={resume}>
            <AudioLines size={14} aria-hidden="true" />
            Sync
          </button>
        )}
      </div>
      {tools}
    </div>
  );
}

function PlainLyrics({
  lines,
  credit,
  className,
  footer,
  extras,
  note = UNSYNCED_NOTE,
  tools,
}: {
  lines: string[];
  credit?: string;
  className: string;
  footer?: ReactNode;
  extras: Extras;
  note?: string;
  tools?: ReactNode;
}) {
  const picking = useLyricPick((s) => s.picking);
  const picked = useLyricPick((s) => s.picked);
  return (
    <div className={`lyrics lyrics--plain ${className}`}>
      <div className="lyrics__body">
        <div className="lyrics__scroll" tabIndex={0} aria-label="Lyrics">
          <div className="lyrics__lines">
            <p className="lyrics__unsynced">{note}</p>
            {lines.map((l, i) =>
              l && picking ? (
                <button
                  key={i}
                  type="button"
                  className={`lyrics__line lyrics__line--pick ${picked.includes(i) ? "is-picked" : ""}`}
                  aria-pressed={picked.includes(i)}
                  onClick={() => useLyricPick.getState().toggle(i)}
                >
                  {l}
                  <LyricExtras extras={extras} i={i} />
                </button>
              ) : l ? (
                <p key={i} className="lyrics__line">
                  {l}
                  <LyricExtras extras={extras} i={i} />
                </p>
              ) : (
                <span key={i} className="lyrics__break" aria-hidden="true" />
              ),
            )}
          </div>
          {credit && <p className="lyrics__credit">{credit}</p>}
          {footer}
        </div>
      </div>
      {tools}
    </div>
  );
}

/**
 * Lyrics for the playing song: synced lyrics follow playback (current line
 * bright, a little above the middle; click a line to seek there). A hand
 * scroll stops following until Sync. Unsynced lyrics
 * say so at the top and are plain text in one tint: no follow, no seeking.
 * Under them: where they came from, and Edit for lyrics the user wrote.
 * A live version showing lyrics timed to another version says so (and
 * offers to transcribe the song, when transcription is on).
 */
export function LyricsView({ lyrics, track }: { lyrics: Lyrics; track: Track }) {
  const timed = timedLines(lyrics);
  const cls = "lyrics--full";
  const texts = timed ? timed.map((l) => l.text) : lyrics.lines;
  const borrowed = !!lyrics.borrowedTiming;
  const extras = useLyricExtras(texts);
  const footer = (
    <>
      <TranslateButton extras={extras} />
      {borrowed && timed && <p className="lyrics__borrowed">{BORROWED_NOTE}</p>}
      <LyricPickBar track={track} texts={texts} />
      <LyricsSourceLine lyrics={lyrics} track={track} />
      {borrowed && <TranscribeOffer trackId={track.id} />}
    </>
  );
  const tools = <LyricsTools lyrics={lyrics} track={track} synced={!!timed} />;
  return timed ? (
    <SyncedLyrics
      key={track.id}
      lines={timed}
      credit={lyrics.credit}
      className={cls}
      footer={footer}
      tools={tools}
      onTap={(l) => tapLine(track.id, l, lyrics)}
      extras={extras}
    />
  ) : (
    <PlainLyrics
      key={track.id}
      lines={lyrics.lines}
      credit={lyrics.credit}
      className={cls}
      footer={footer}
      extras={extras}
      note={borrowed ? BORROWED_NOTE : UNSYNCED_NOTE}
      tools={tools}
    />
  );
}

/**
 * The expanded view's lyrics mode for whatever is playing: the lyrics, a
 * loading placeholder, or "No lyrics for this song" with Add lyrics and the
 * transcribe offer.
 */
export function LyricsPane() {
  const track = usePlayer((s) => s.state?.track);
  const { lyrics, loading, info } = useLyrics(track?.id);
  const showAdd = useSettings((s) => s.settings?.lyricsShowAdd ?? true);

  if (!track) return <p className="lyrics-page__empty">Nothing playing.</p>;
  if (loading)
    return (
      <div className="lyrics-page__loading" aria-label="Loading lyrics">
        {[70, 55, 80, 45].map((w, i) => (
          <Skeleton key={i} width={`${w}%`} height={32} />
        ))}
      </div>
    );
  if (!lyrics && info?.notice)
    return (
      <>
        <LyricsNotice info={info} track={track} />
        {info.notice === "noneMatched" && <TranscribeOffer trackId={track.id} />}
      </>
    );
  if (!lyrics)
    return (
      <div className="lyrics-page__empty-wrap">
        <p className="lyrics-page__empty">No lyrics for this song</p>
        {showAdd && (
          <Button variant="primary" icon={<PenLine size={16} />} onClick={() => openLyricsEditor(track)}>
            Add lyrics
          </Button>
        )}
        <TranscribeOffer trackId={track.id} />
      </div>
    );
  return <LyricsView lyrics={lyrics} track={track} />;
}

/**
 * Share lyrics: a button that starts picking lines (clicks pick instead of
 * seeking), then a bar with how many are picked, Share lyrics and Cancel.
 */
export function LyricPickBar({ track, texts }: { track: Track; texts: string[] }) {
  const pick = useLyricPick();
  const mine = pick.picking && pick.trackId === track.id;
  // A new song ends picking.
  useEffect(() => {
    const p = useLyricPick.getState();
    if (p.picking && p.trackId !== track.id) p.cancel();
  }, [track.id]);
  useEffect(() => () => useLyricPick.getState().cancel(), []);
  if (!mine)
    return (
      <p className="lyrics__share">
        <button type="button" className="lyrics__source-edit" onClick={() => pick.start(track.id)}>
          <ImageIcon size={14} aria-hidden="true" /> Share lyrics
        </button>
      </p>
    );
  const n = pick.picked.length;
  return (
    <div className="lyrics__pickbar" role="group" aria-label="Pick lyrics to share">
      <span className="lyrics__pickcount" aria-live="polite">
        {n === 0 ? `Pick up to ${LYRIC_LINES_MAX} lines` : `${n} of ${LYRIC_LINES_MAX} lines`}
      </span>
      <Button variant="ghost" size="sm" onClick={pick.cancel}>
        Cancel
      </Button>
      <Button
        variant="primary"
        size="sm"
        disabled={n === 0}
        onClick={() => {
          const lines = pick.picked.map((i) => texts[i]).filter(Boolean);
          pick.cancel();
          openShareCard({
            kind: "track",
            id: track.id,
            title: track.title,
            subtitle: trackArtistLine(track),
            thumb: track.thumb,
            durationMs: track.durationMs,
            lyrics: lines,
          });
        }}
      >
        Share lyrics
      </Button>
    </div>
  );
}

function LyricsSourceLine({ lyrics, track }: { lyrics: Lyrics; track: Track }) {
  const label = lyricsSourceLabel(lyrics);
  const own = useLyricsEditor((s) => s.ownIds.has(track.id));
  // Your own lyrics or a transcription to fix, or ones you wrote that Plex's are showing over.
  const mine = lyrics.source === "user" || lyrics.source === "auto";
  const edit = mine || own;
  if (!label && !edit) return null;
  return (
    <p className="lyrics__source">
      {label && <span>{label}</span>}
      {edit && (
        <button type="button" className="lyrics__source-edit" onClick={() => openLyricsEditor(track)}>
          {mine ? "Edit" : "Edit your lyrics"}
        </button>
      )}
    </p>
  );
}
