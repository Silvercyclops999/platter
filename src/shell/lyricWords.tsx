/**
 * Word by word lyrics, shared by the computer's lyrics view and the
 * phone's: which line and word are playing (every frame while the song
 * plays), and the current line drawn word by word with the sung words lit.
 * Lines without word timing show whole, lit by line.
 */
import { useCallback, useEffect, useState } from "react";
import { usePageVisible } from "../design/pageVisibility";
import type { LyricLine, LyricWord } from "../lib/types";
import { usePlayer } from "../stores/player";
import { currentLineIndex, currentPositionMs, currentWordIndex, setPlayhead } from "./lyrics";

/** Which line and word are playing, updated every frame but only re-rendering on a change. */
export function usePlayhead(lines: LyricLine[]): { line: number; word: number; jump: (ms: number) => void } {
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

/** A line's words, the ones up to `word` lit (`is-sung`). */
export function LyricWords({ words, word }: { words: LyricWord[]; word: number }) {
  return (
    <>
      {words.map((w, j) => (
        <span key={j} className={`lyrics__word ${j <= word ? "is-sung" : ""}`}>
          {w.text}
        </span>
      ))}
    </>
  );
}
