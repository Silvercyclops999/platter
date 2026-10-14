import { AudioLines, PenLine } from "lucide-react";
import { useCallback, useEffect, useLayoutEffect, useRef, useState, type KeyboardEvent, type ReactNode } from "react";
import { Button } from "../components/Button";
import { Skeleton } from "../components/Skeleton";
import { prefersReducedMotion } from "../design/motion";
import { NEUTRAL_HEADER } from "../lib/color";
import type { LyricLine, Lyrics, Track } from "../lib/types";
import { openLyricsEditor, useLyricsEditor } from "../stores/lyricsEditor";
import { player, usePlayer } from "../stores/player";
import { useSettings } from "../stores/settings";
import { currentLineIndex, currentWordIndex, followTarget, interpolate, lineState, lyricsSourceLabel, timedLines, useLyrics } from "./lyrics";
import { useCoverColors } from "./nowPlaying";
import { TranscribeOffer } from "./TranscribeOffer";
import "./lyrics.css";
import "./lyrics-editor.css";

/** Said above lyrics that have no timing. */
export const UNSYNCED_NOTE = "These lyrics aren't synced to the song yet";

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

/** Which line and word are playing, updated every frame but only re-rendering on a change. */
function usePlayhead(lines: LyricLine[]): { line: number; word: number; jump: (ms: number) => void } {
  const [pos, setPos] = useState({ line: -1, word: -1 });
  const base = useRef({ positionMs: usePlayer.getState().positionMs, at: performance.now() });
  const measure = useCallback(() => {
    const st = usePlayer.getState();
    const ms = interpolate(base.current, performance.now(), st.state?.status === "playing", st.durationMs);
    const line = currentLineIndex(lines, ms);
    const word = line >= 0 ? currentWordIndex(lines[line].words, ms) : -1;
    setPos((p) => (p.line === line && p.word === word ? p : { line, word }));
  }, [lines]);
  useEffect(() => {
    let raf = 0;
    const unsub = usePlayer.subscribe((s, p) => {
      if (s.positionMs !== p.positionMs || s.state?.status !== p.state?.status) {
        base.current = { positionMs: s.positionMs, at: performance.now() };
        measure();
      }
    });
    const tick = () => {
      measure();
      raf = requestAnimationFrame(tick);
    };
    tick();
    return () => {
      cancelAnimationFrame(raf);
      unsub();
    };
  }, [measure]);
  const jump = useCallback(
    (ms: number) => {
      base.current = { positionMs: ms, at: performance.now() };
      measure();
    },
    [measure],
  );
  return { ...pos, jump };
}

function SyncedLyrics({ lines, credit, className, footer }: { lines: LyricLine[]; credit?: string; className: string; footer?: ReactNode }) {
  const { line: current, word, jump } = usePlayhead(lines);
  const scroller = useRef<HTMLDivElement>(null);
  const lineEls = useRef<(HTMLElement | null)[]>([]);
  const [following, setFollowing] = useState(true);
  const first = useRef(true);

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

  // Keep the current line a little above the middle (the first placement jumps).
  useLayoutEffect(() => {
    if (!following) return;
    scrollToCurrent(!first.current);
    first.current = false;
  }, [current, following, scrollToCurrent]);

  const resume = () => setFollowing(true);
  // Scrolling by hand stops following; the view stays put until Sync.
  const pause = () => setFollowing(false);
  const onKey = (e: KeyboardEvent) => {
    if (e.target === scroller.current && SCROLL_KEYS.has(e.key)) pause();
  };

  return (
    <div className={`lyrics lyrics--synced ${className}`}>
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
        <div className="lyrics__lines">
          {lines.map((l, i) => {
            const state = lineState(i, current);
            const words = state === "current" && l.words?.length ? l.words : null;
            return (
              <button
                key={i}
                type="button"
                ref={(el) => {
                  lineEls.current[i] = el;
                }}
                className={`lyrics__line is-${state} ${l.text ? "" : "lyrics__line--gap"}`}
                aria-current={state === "current" ? "true" : undefined}
                aria-label={l.text || "Instrumental"}
                onClick={() => {
                  jump(l.startMs);
                  resume();
                  void player.seek(l.startMs);
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
              </button>
            );
          })}
        </div>
        {credit && <p className="lyrics__credit">{credit}</p>}
        {footer}
      </div>
      {!following && (
        <button type="button" className="lyrics__sync" aria-label="Sync to the current line" onClick={resume}>
          <AudioLines size={16} aria-hidden="true" />
          Sync
        </button>
      )}
    </div>
  );
}

function PlainLyrics({ lines, credit, className, footer }: { lines: string[]; credit?: string; className: string; footer?: ReactNode }) {
  return (
    <div className={`lyrics lyrics--plain ${className}`}>
      <div className="lyrics__scroll" tabIndex={0} aria-label="Lyrics">
        <div className="lyrics__lines">
          <p className="lyrics__unsynced">{UNSYNCED_NOTE}</p>
          {lines.map((l, i) =>
            l ? (
              <p key={i} className="lyrics__line">
                {l}
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
  );
}

/**
 * Lyrics for the playing song: synced lyrics follow playback (current line
 * bright, a little above the middle; click a line to seek there). A hand
 * scroll stops following until Sync. Unsynced lyrics
 * say so at the top and are plain text in one tint: no follow, no seeking.
 * Under them: where they came from, and Edit for lyrics the user wrote.
 */
export function LyricsView({ lyrics, track }: { lyrics: Lyrics; track: Track }) {
  const timed = timedLines(lyrics);
  const cls = "lyrics--full";
  const footer = <LyricsSourceLine lyrics={lyrics} track={track} />;
  return timed ? (
    <SyncedLyrics key={track.id} lines={timed} credit={lyrics.credit} className={cls} footer={footer} />
  ) : (
    <PlainLyrics key={track.id} lines={lyrics.lines} credit={lyrics.credit} className={cls} footer={footer} />
  );
}

/**
 * The expanded view's lyrics mode for whatever is playing: the lyrics, a
 * loading placeholder, or "No lyrics for this song" with Add lyrics and the
 * transcribe offer.
 */
export function LyricsPane() {
  const track = usePlayer((s) => s.state?.track);
  const { lyrics, loading } = useLyrics(track?.id);
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
