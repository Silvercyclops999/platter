/**
 * Lyrics on the phone: a tinted card under the player, and the whole screen
 * from it. Sung and current lines are bright, the ones to come are dim.
 * Tapping a timed line plays from there and keeps play or pause. Where the
 * lyrics time each word, the current line lights word by word, as on the
 * computer; otherwise the whole line lights.
 */
import { ChevronDown, Maximize2, Pause, PenLine, Play, RotateCw } from "lucide-react";
import { memo, useCallback, useEffect, useRef, useState, type CSSProperties } from "react";
import { backend } from "../lib/backend";
import { trackArtistLine } from "../lib/format";
import type { Lyrics, LyricLine, LyricWord, Track } from "../lib/types";
import { useArtColours } from "../lib/useTint";
import { currentLineIndex, lyricsChanged, timedLines, useLyrics } from "../shell/lyrics";
import { LyricWords, usePlayhead } from "../shell/lyricWords";
import { openLyricsEditor } from "../stores/lyricsEditor";
import { useSettings } from "../stores/settings";
import { toast } from "../stores/toasts";
import "./pages/parity.css";
import { player, usePlayer } from "../stores/player";
import { tap } from "./bits";
import { useLyricFollow } from "./lyricsFollow";
import { phone, usePhoneUi } from "./phoneUi";
import { PhoneSeek } from "./NowPlaying";
import { holdLightStatusBar } from "./statusBar";


/** Lines the card shows around the current one. */
export const CARD_LINES = 4;

/** A card line: its text, whether it is lit, and its words when they are timed (the current line only). */
export interface CardLine {
  text: string;
  sung: boolean;
  words?: LyricWord[];
}

/** The card's lines: from the current line on (or the first ones untimed). */
export function cardWindow(l: Lyrics, ms: number): CardLine[] {
  const timed = timedLines(l);
  return cardWindowAt(l, timed ? currentLineIndex(timed, ms) : -1);
}

/** The card's lines from line `line` (-1 before the first). */
export function cardWindowAt(l: Lyrics, line: number): CardLine[] {
  const timed = timedLines(l);
  if (!timed) return l.lines.filter((x) => x.trim()).slice(0, CARD_LINES).map((text) => ({ text, sung: true }));
  const i = Math.max(0, line);
  return timed.slice(i, i + CARD_LINES).map((x, k) => {
    const now = k === 0 && line >= 0;
    return { text: x.text.trim() || "♪", sung: now, ...(now && x.text.trim() && x.words?.length ? { words: x.words } : {}) };
  });
}

const NO_LINES: LyricLine[] = [];

export function LyricsCard({ track }: { track: Track }) {
  const { lyrics, loading } = useLyrics(track.id);
  // The same playhead as the computer's lyrics: line and word, each frame while playing.
  const head = usePlayhead(timedLines(lyrics) ?? NO_LINES);
  const colours = useArtColours(null, track.thumb);
  if (loading || !lyrics) return null;
  const lines = cardWindowAt(lyrics, head.line);
  return (
    <section className="ph-card ph-lyrics-card" style={{ "--ph-tint": colours?.vivid ?? "var(--color-bg-overlay)" } as CSSProperties} aria-label="Lyrics">
      <div className="ph-card__head">
        <h3 className="ph-card__title">Lyrics</h3>
        <button type="button" className="ph-icon-btn" aria-label="Show lyrics full screen" onClick={phone.openLyrics}>
          <Maximize2 size={18} aria-hidden="true" />
        </button>
      </div>
      <button type="button" className="ph-lyrics-card__lines" onClick={phone.openLyrics} aria-label="Open lyrics">
        {lines.map((l, i) => (
          <span key={i} className={`ph-lyrics-card__line ${l.sung ? "is-sung" : ""} ${l.words ? "is-worded" : ""}`}>
            {l.words ? <LyricWords words={l.words} word={head.word} /> : l.text}
          </span>
        ))}
      </button>
    </section>
  );
}

/** Full screen lyrics, over now playing. */
export function LyricsScreen() {
  const open = usePhoneUi((s) => s.lyrics);
  const track = usePlayer((s) => s.state?.track ?? null);
  if (!open || !track) return null;
  return <LyricsFull track={track} />;
}

function LyricsFull({ track }: { track: Track }) {
  // Full-screen lyrics are dark on every look: light status bar text while they show.
  useEffect(() => holdLightStatusBar(), []);
  const { lyrics } = useLyrics(track.id);
  const colours = useArtColours(null, track.thumb);
  const status = usePlayer((s) => s.state?.status ?? "idle");
  const playing = status === "playing" || status === "loading" || status === "buffering";
  const timed = timedLines(lyrics);
  const view = useRef<HTMLDivElement>(null);
  const { current, word, seekTo } = useLyricFollow(timed, view);
  return (
    <section
      className="ph-lyrics"
      role="dialog"
      aria-modal="true"
      aria-label="Lyrics"
      style={{ "--ph-tint": colours?.vivid ?? "var(--color-bg-overlay)" } as CSSProperties}
    >
      <header className="ph-lyrics__top">
        <button type="button" className="ph-icon-btn" aria-label="Close lyrics" onClick={phone.closeLyrics}>
          <ChevronDown size={26} aria-hidden="true" />
        </button>
        <div className="ph-lyrics__who">
          <span className="ph-lyrics__title">{track.title}</span>
          <span className="ph-lyrics__artist">{trackArtistLine(track)}</span>
        </div>
        <span className="ph-icon-btn" aria-hidden="true" />
      </header>
      <div className="ph-lyrics__view" ref={view} data-testid="lyrics-view">
        {timed ? (
          <TimedLines lines={timed} current={current} word={word} onSeek={seekTo} />
        ) : lyrics ? (
          <div className="ph-lyrics__lines">
            {lyrics.lines.map((l, i) => (
              <p key={i} className={`ph-lyric is-sung ${l.trim() ? "" : "ph-lyric--gap"}`}>
                {l}
              </p>
            ))}
          </div>
        ) : (
          <NoLyrics track={track} />
        )}
        {lyrics?.credit && <p className="ph-lyrics__credit">{lyrics.credit}</p>}
      </div>
      <footer className="ph-lyrics__bottom">
        <PhoneSeek />
        <button
          type="button"
          className="ph-play ph-play--small"
          aria-label={playing ? "Pause" : "Play"}
          onClick={() => {
            tap();
            void player.toggle();
          }}
        >
          {playing ? <Pause size={26} fill="currentColor" aria-hidden="true" /> : <Play size={26} fill="currentColor" aria-hidden="true" />}
        </button>
      </footer>
    </section>
  );
}

/**
 * A song with no lyrics: say so plainly, and offer to look again (the
 * sources may have it now) or to write them (while that's on in Settings).
 */
export function NoLyrics({ track }: { track: Track }) {
  const showAdd = useSettings((s) => s.settings?.lyricsShowAdd ?? true);
  const [looking, setLooking] = useState(false);
  const lookAgain = async () => {
    setLooking(true);
    try {
      await backend.lyricsLookAgain(track.id);
      lyricsChanged(track.id);
    } catch {
      toast.error("Couldn't look for lyrics again");
    } finally {
      setLooking(false);
    }
  };
  return (
    <div className="ph-lyrics__empty" role="status">
      <p className="ph-lyrics__empty-title">No lyrics for this song</p>
      <p className="ph-lyrics__empty-text">None of your lyrics sources have it yet.</p>
      <div className="ph-lyrics__empty-actions">
        {showAdd && (
          <button
            type="button"
            className="ph-pill is-primary"
            onClick={() => {
              phone.closeLyrics();
              openLyricsEditor(track);
            }}
          >
            <PenLine size={16} aria-hidden="true" /> Add lyrics
          </button>
        )}
        <button type="button" className="ph-pill" disabled={looking} onClick={() => void lookAgain()}>
          <RotateCw size={16} aria-hidden="true" className={looking ? "ph-spin" : undefined} /> {looking ? "Looking…" : "Look again"}
        </button>
      </div>
    </div>
  );
}

function TimedLines({ lines, current, word, onSeek }: { lines: LyricLine[]; current: number; word: number; onSeek: (ms: number) => void }) {
  // One seek for every row, so a row only draws again when its own state changes.
  const seekRef = useRef(onSeek);
  seekRef.current = onSeek;
  const seek = useCallback((ms: number) => seekRef.current(ms), []);
  return (
    <div className="ph-lyrics__lines">
      {lines.map((l, i) => (
        <TimedLine key={i} line={l} i={i} sung={i <= current} now={i === current} word={i === current ? word : -1} onSeek={seek} />
      ))}
    </div>
  );
}

/** One timed line; only the current one is given the word index. */
const TimedLine = memo(function TimedLine({ line, i, sung, now, word, onSeek }: { line: LyricLine; i: number; sung: boolean; now: boolean; word: number; onSeek: (ms: number) => void }) {
  const gap = !line.text.trim();
  // The current line lights word by word when its words are timed.
  const words = now && !gap && line.words?.length ? line.words : null;
  return (
    <button
      type="button"
      data-line={i}
      className={`ph-lyric ${sung ? "is-sung" : ""} ${now ? "is-current" : ""} ${words ? "is-worded" : ""} ${gap ? "ph-lyric--gap" : ""}`}
      aria-current={now ? "true" : undefined}
      onClick={() => onSeek(line.startMs)}
    >
      {gap ? <span aria-label="Instrumental">♪</span> : words ? <LyricWords words={words} word={word} /> : line.text}
    </button>
  );
});
