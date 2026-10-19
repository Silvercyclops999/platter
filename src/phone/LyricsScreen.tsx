/**
 * Lyrics on the phone: a tinted card under the player, and the whole screen
 * from it. Sung and current lines are bright, the ones to come are dim.
 * Tapping a timed line plays from there and keeps play or pause.
 */
import { ChevronDown, Maximize2, Pause, PenLine, Play, RotateCw } from "lucide-react";
import { useRef, useState, type CSSProperties } from "react";
import { backend } from "../lib/backend";
import { trackArtistLine } from "../lib/format";
import type { Lyrics, LyricLine, Track } from "../lib/types";
import { useArtColours } from "../lib/useTint";
import { currentLineIndex, lyricsChanged, timedLines, useLyrics } from "../shell/lyrics";
import { openLyricsEditor } from "../stores/lyricsEditor";
import { useSettings } from "../stores/settings";
import { toast } from "../stores/toasts";
import "./pages/parity.css";
import { player, usePlayer } from "../stores/player";
import { tap } from "./bits";
import { useLyricFollow } from "./lyricsFollow";
import { phone, usePhoneUi } from "./phoneUi";
import { PhoneSeek } from "./NowPlaying";

/** Lines the card shows around the current one. */
export const CARD_LINES = 4;

/** The card's lines: from the current line on (or the first ones untimed). */
export function cardWindow(l: Lyrics, ms: number): { text: string; sung: boolean }[] {
  const timed = timedLines(l);
  if (!timed) return l.lines.filter((x) => x.trim()).slice(0, CARD_LINES).map((text) => ({ text, sung: true }));
  const i = Math.max(0, currentLineIndex(timed, ms));
  return timed
    .slice(i, i + CARD_LINES)
    .map((line, k) => ({ text: line.text.trim() || "♪", sung: k === 0 && currentLineIndex(timed, ms) >= 0 }));
}

export function LyricsCard({ track }: { track: Track }) {
  const { lyrics, loading } = useLyrics(track.id);
  const positionMs = usePlayer((s) => s.positionMs);
  const colours = useArtColours(null, track.thumb);
  if (loading || !lyrics) return null;
  const lines = cardWindow(lyrics, positionMs);
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
          <span key={i} className={`ph-lyrics-card__line ${l.sung ? "is-sung" : ""}`}>
            {l.text}
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
  const { lyrics } = useLyrics(track.id);
  const colours = useArtColours(null, track.thumb);
  const status = usePlayer((s) => s.state?.status ?? "idle");
  const playing = status === "playing" || status === "loading" || status === "buffering";
  const timed = timedLines(lyrics);
  const view = useRef<HTMLDivElement>(null);
  const { current, seekTo } = useLyricFollow(timed, view);
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
          <TimedLines lines={timed} current={current} onSeek={seekTo} />
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

function TimedLines({ lines, current, onSeek }: { lines: LyricLine[]; current: number; onSeek: (ms: number) => void }) {
  return (
    <div className="ph-lyrics__lines">
      {lines.map((l, i) => {
        const gap = !l.text.trim();
        const sung = i <= current;
        return (
          <button
            key={i}
            type="button"
            data-line={i}
            className={`ph-lyric ${sung ? "is-sung" : ""} ${i === current ? "is-current" : ""} ${gap ? "ph-lyric--gap" : ""}`}
            aria-current={i === current ? "true" : undefined}
            onClick={() => onSeek(l.startMs)}
          >
            {gap ? <span aria-label="Instrumental">♪</span> : l.text}
          </button>
        );
      })}
    </div>
  );
}
