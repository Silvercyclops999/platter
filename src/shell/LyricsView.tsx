import { LocateFixed } from "lucide-react";
import { useCallback, useEffect, useLayoutEffect, useRef, useState, type KeyboardEvent, type ReactNode } from "react";
import { Skeleton } from "../components/Skeleton";
import { prefersReducedMotion } from "../design/motion";
import { NEUTRAL_HEADER } from "../lib/color";
import type { LyricLine, Lyrics, Track } from "../lib/types";
import { player, usePlayer } from "../stores/player";
import { useLyricPick } from "../stores/shareCard";
import { useSettings } from "../stores/settings";
import { LyricExtras, useLyricExtras, type Extras } from "./lyricExtras";
import { followTarget, lineState, onSeek, timedLines, useLyrics } from "./lyrics";
import { useCoverColors } from "./nowPlaying";
import { NoLyrics } from "./TranscribeOffer";
import { LyricWords, usePlayhead } from "./lyricWords";
import { LyricsFooter, LyricsNotice, tapLine } from "./LyricsTrust";
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
  /** Under the lyrics: where they came from and the options menu. */
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
          data-scrollbar="off"
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
                      <LyricWords words={words} word={word} />
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
          <button type="button" className="lyrics__sync" aria-label="Follow along from the current line" onClick={resume}>
            <LocateFixed size={14} aria-hidden="true" />
            Follow along
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
        <div className="lyrics__scroll" data-scrollbar="off" tabIndex={0} aria-label="Lyrics">
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
 * Under them, one quiet row: where they came from and the options menu.
 * A live version showing lyrics timed to another version says so.
 */
export function LyricsView({ lyrics, track }: { lyrics: Lyrics; track: Track }) {
  const timed = timedLines(lyrics);
  const cls = "lyrics--full";
  const texts = timed ? timed.map((l) => l.text) : lyrics.lines;
  const borrowed = !!lyrics.borrowedTiming;
  const extras = useLyricExtras(texts);
  const footer = borrowed && timed ? <p className="lyrics__borrowed">{BORROWED_NOTE}</p> : undefined;
  const tools = <LyricsFooter lyrics={lyrics} track={track} synced={!!timed} texts={texts} extras={extras} />;
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

/** The loading placeholder: grey bars where lines go. */
function LyricsLoading() {
  return (
    <div className="lyrics-page__loading" aria-label="Loading lyrics">
      {[70, 55, 80, 45].map((w, i) => (
        <Skeleton key={i} width={`${w}%`} height={32} />
      ))}
    </div>
  );
}

/**
 * The expanded view's lyrics mode for whatever is playing: the lyrics, a
 * loading placeholder, or, for a song without any, the words being written
 * down (when this computer can) or "No lyrics for this song".
 */
export function LyricsPane() {
  const track = usePlayer((s) => s.state?.track);
  const { lyrics, loading, info } = useLyrics(track?.id);

  if (!track) return <p className="lyrics-page__empty">Nothing playing.</p>;
  if (loading || (!lyrics && info?.notice === "checking")) return <LyricsLoading />;
  if (!lyrics && info?.notice && info.notice !== "noneMatched") return <LyricsNotice info={info} track={track} />;
  if (!lyrics) return <NoLyrics track={track} />;
  return <LyricsView lyrics={lyrics} track={track} />;
}
