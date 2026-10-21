import { useEffect } from "react";
import { Button } from "../components/Button";
import type { Track, TranscribeStatus } from "../lib/types";
import { openLyricsEditor } from "../stores/lyricsEditor";
import { checkFoundNothing, transcribe, useTranscribe } from "../stores/transcribe";

/** Said while a song's words are being written down. */
export const WRITING_TEXT = "Writing down the words…";
/** Said while the speech model downloads for the first time. */
export const FIRST_TIME_TEXT = "Getting ready to write down lyrics, first time only";
/** Said for a song transcribed with no words found. */
export const NO_WORDS_TEXT = "No words were heard in this song";

/** How far along the song (or the first model download) is, 0 to 100, or null when it can't say yet. */
export function writingPercent(c: NonNullable<TranscribeStatus["current"]>, s: TranscribeStatus): number | null {
  if (c.stage === "waitingForModel") {
    const m = s.models.find((x) => x.kind === s.model);
    return m && m.sizeBytes ? Math.floor((m.doneBytes / m.sizeBytes) * 100) : null;
  }
  // The percent moves once per 30 second stretch of the song.
  return c.stage === "transcribing" && c.percent > 0 ? c.percent : null;
}

/** A thin line that fills as the work goes; one that moves on its own when the amount isn't known. */
function ProgressLine({ percent, label }: { percent: number | null; label: string }) {
  return (
    <div
      className={`lyrics-page__progress ${percent === null ? "is-unknown" : ""}`}
      role="progressbar"
      aria-label={label}
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={percent ?? undefined}
    >
      <span className="lyrics-page__progress-fill" style={percent === null ? undefined : { width: `${percent}%` }} />
    </div>
  );
}

/**
 * The lyrics view for a song with no lyrics anywhere. When this computer
 * writes lyrics down, the song is already being done (it starts as the song
 * plays): its progress shows, then the lyrics replace it. Otherwise, or when
 * it found no words: "No lyrics for this song" and a quiet Write your own.
 */
export function NoLyrics({ track }: { track: Track }) {
  const status = useTranscribe((s) => s.status);
  const empty = useTranscribe((s) => s.empty.has(track.id));
  const on = !!status?.enabled && status.available;
  useEffect(() => {
    if (on) void checkFoundNothing(track.id);
  }, [on, track.id]);

  const current = on && status.current?.trackId === track.id ? status.current : undefined;
  // Next in line counts as being written down only while the work isn't
  // held up (paused, on battery, off hours) or stopped by a failure.
  const queued = on && !current && status.upNext === track.id && !status.paused && !status.userPaused && !status.error;
  if (status && (current || queued)) {
    const first = current?.stage === "waitingForModel";
    const text = first ? FIRST_TIME_TEXT : WRITING_TEXT;
    return (
      <div className="lyrics-page__empty-wrap lyrics-page__empty-wrap--writing">
        <p className="lyrics-page__writing" role="status">
          {text}
        </p>
        <ProgressLine percent={current ? writingPercent(current, status) : null} label={text} />
      </div>
    );
  }
  // This song failed, or the model download did (it's tried again with the next song).
  const e = on ? status.error : undefined;
  const failed = e && (e.trackId === track.id || (!e.trackId && e.kind !== "transcribe")) ? e : undefined;
  return (
    <div className="lyrics-page__empty-wrap">
      <p className="lyrics-page__empty">{empty ? NO_WORDS_TEXT : "No lyrics for this song"}</p>
      {failed && (
        <p className="lyrics-page__transcribe-note" role="alert">
          {failed.message}
        </p>
      )}
      <div className="lyrics-page__empty-actions">
        {on && !empty && (
          <Button variant="primary" onClick={() => void transcribe.track(track.id)}>
            {failed ? "Try again" : "Write down the words"}
          </Button>
        )}
        <button type="button" className="lyrics__source-edit" onClick={() => openLyricsEditor(track)}>
          Write your own
        </button>
      </div>
    </div>
  );
}
