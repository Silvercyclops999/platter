/**
 * How the phone's lyrics follow the song: the current line flips at once,
 * then the list eases up by the finished line in a short scroll that keeps
 * the current line about 45% down the view. Nothing scrolls until the song
 * gets to that spot. A seek, playing or paused, puts both right at once.
 * Lines with word timing also say which word is playing, for the fill.
 */
import { useEffect, useRef, useState, type RefObject } from "react";
import type { LyricLine } from "../lib/types";
import { currentLineIndex, currentPositionMs, currentWordIndex, onSeek, setPlayhead } from "../shell/lyrics";
import { player, usePlayer } from "../stores/player";

/** Where the current line sits, as a share of the view's height. */
export const PHONE_ANCHOR = 0.45;
/** The eased scroll after a line flips. */
export const FOLLOW_MS = 280;
/** The scroll starts this long after the flip. */
export const FOLLOW_DELAY_MS = 30;

/** The scroll offset that puts a line's middle at the anchor (never above the top). */
export function anchorTarget(lineTop: number, lineHeight: number, viewHeight: number, scrollHeight: number): number {
  const top = lineTop + lineHeight / 2 - viewHeight * PHONE_ANCHOR;
  return Math.max(0, Math.min(top, Math.max(0, scrollHeight - viewHeight)));
}

/** Ease in and out (a smooth start and stop). */
export const easeInOut = (t: number) => (t < 0.5 ? 2 * t * t : 1 - (-2 * t + 2) ** 2 / 2);

/**
 * The current line for `lines`, kept up to date while playing (each frame)
 * and at once on any seek. Scrolls `view` to keep it at the anchor.
 */
export function useLyricFollow(lines: LyricLine[] | null, view: RefObject<HTMLElement | null>) {
  const [current, setCurrent] = useState(() => (lines ? currentLineIndex(lines, currentPositionMs()) : -1));
  const [word, setWord] = useState(-1);
  const status = usePlayer((s) => s.state?.status);
  const playing = status === "playing" || status === "buffering";
  const shown = useRef(current);
  const anim = useRef<{ frame: number; timer: ReturnType<typeof setTimeout> | null }>({ frame: 0, timer: null });

  const scrollTo = (i: number, eased: boolean) => {
    const el = view.current;
    const line = el?.querySelector<HTMLElement>(`[data-line="${i}"]`);
    if (!el || !line) return;
    const target = anchorTarget(line.offsetTop, line.offsetHeight, el.clientHeight, el.scrollHeight);
    cancelAnimationFrame(anim.current.frame);
    if (anim.current.timer) clearTimeout(anim.current.timer);
    if (!eased || Math.abs(target - el.scrollTop) < 1) {
      el.scrollTop = target;
      return;
    }
    anim.current.timer = setTimeout(() => {
      const from = el.scrollTop;
      const start = performance.now();
      const step = (t: number) => {
        const k = Math.min(1, (t - start) / FOLLOW_MS);
        el.scrollTop = from + (target - from) * easeInOut(k);
        if (k < 1) anim.current.frame = requestAnimationFrame(step);
      };
      anim.current.frame = requestAnimationFrame(step);
    }, FOLLOW_DELAY_MS);
  };

  const update = (eased: boolean, ms = currentPositionMs()) => {
    if (!lines) return;
    const i = currentLineIndex(lines, ms);
    // The word fill (lines with word timing only); React drops a repeat of the same value.
    setWord(i >= 0 ? currentWordIndex(lines[i].words, ms) : -1);
    if (i !== shown.current || !eased) {
      shown.current = i;
      setCurrent(i);
      scrollTo(Math.max(0, i), eased);
    }
  };

  // A new song or new lyrics: start from where the song is, without easing.
  useEffect(() => {
    shown.current = -2;
    update(false);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [lines]);

  // Each frame while playing.
  useEffect(() => {
    if (!playing || !lines) return;
    let frame = 0;
    const tick = () => {
      update(true);
      frame = requestAnimationFrame(tick);
    };
    frame = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(frame);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [playing, lines]);

  // Any seek, paused too: line states and scroll at once.
  useEffect(() => onSeek(() => update(false)), [lines]); // eslint-disable-line react-hooks/exhaustive-deps

  useEffect(
    () => () => {
      cancelAnimationFrame(anim.current.frame);
      if (anim.current.timer) clearTimeout(anim.current.timer);
    },
    [],
  );

  /** Tap a line: play from its start, keeping play or pause as it is. */
  const seekTo = (ms: number) => {
    setPlayhead(ms);
    void player.seek(ms);
    update(false, ms);
  };
  return { current, word, seekTo };
}
