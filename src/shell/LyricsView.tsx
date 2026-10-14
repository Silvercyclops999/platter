import { ArrowDown } from "lucide-react";
import { useCallback, useEffect, useLayoutEffect, useRef, useState, type KeyboardEvent } from "react";
import { prefersReducedMotion } from "../design/motion";
import type { LyricLine, Lyrics } from "../lib/types";
import { player, usePlayer } from "../stores/player";
import { currentLineIndex, currentWordIndex, followTarget, FOLLOW_RESUME_MS, interpolate, lineState, timedLines } from "./lyrics";
import "./lyrics.css";

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

function SyncedLyrics({ lines, credit, className }: { lines: LyricLine[]; credit?: string; className: string }) {
  const { line: current, word, jump } = usePlayhead(lines);
  const scroller = useRef<HTMLDivElement>(null);
  const lineEls = useRef<(HTMLElement | null)[]>([]);
  const [following, setFollowing] = useState(true);
  const resumeTimer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
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

  // Keep the current line about a third from the top (the first placement jumps).
  useLayoutEffect(() => {
    if (!following) return;
    scrollToCurrent(!first.current);
    first.current = false;
  }, [current, following, scrollToCurrent]);

  useEffect(() => () => clearTimeout(resumeTimer.current), []);

  const resume = () => {
    clearTimeout(resumeTimer.current);
    setFollowing(true);
  };
  // Scrolling by hand pauses following until 5 s pass with no scrolling.
  const pause = () => {
    setFollowing(false);
    clearTimeout(resumeTimer.current);
    resumeTimer.current = setTimeout(resume, FOLLOW_RESUME_MS);
  };
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
      </div>
      {!following && (
        <button type="button" className="lyrics__back" onClick={resume}>
          <ArrowDown size={16} aria-hidden="true" />
          Back to current line
        </button>
      )}
    </div>
  );
}

function PlainLyrics({ lines, credit, className }: { lines: string[]; credit?: string; className: string }) {
  return (
    <div className={`lyrics lyrics--plain ${className}`}>
      <div className="lyrics__scroll" tabIndex={0} aria-label="Lyrics">
        <div className="lyrics__lines">
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
      </div>
    </div>
  );
}

/**
 * Lyrics for the playing song: synced lyrics follow playback (current line
 * bright, a third from the top; click a line to seek there), plain lyrics are
 * just text. `size` picks the type scale (page or full window).
 */
export function LyricsView({ lyrics, size = "page", trackId }: { lyrics: Lyrics; size?: "page" | "full"; trackId: string }) {
  const timed = timedLines(lyrics);
  const cls = `lyrics--${size}`;
  return timed ? (
    <SyncedLyrics key={trackId} lines={timed} credit={lyrics.credit} className={cls} />
  ) : (
    <PlainLyrics key={trackId} lines={lyrics.lines} credit={lyrics.credit} className={cls} />
  );
}
