import { AudioLines } from "lucide-react";
import { useEffect } from "react";
import { Button } from "../components/Button";
import type { TranscribeStatus } from "../lib/types";
import { checkFoundNothing, modelSize, transcribe, useTranscribe } from "../stores/transcribe";

/** What the song being transcribed is doing, in words. */
export function stageText(c: NonNullable<TranscribeStatus["current"]>, s: TranscribeStatus): string {
  switch (c.stage) {
    case "waitingForModel": {
      const m = s.models.find((x) => x.kind === s.model);
      const pct = m && m.sizeBytes ? Math.floor((m.doneBytes / m.sizeBytes) * 100) : 0;
      return `Downloading the speech model… ${pct}%`;
    }
    case "reading":
      return "Reading the song…";
    case "decoding":
      return "Getting the audio ready…";
    case "transcribing":
      return `Transcribing… ${c.percent}%`;
  }
}

/**
 * The lyrics view's offer for a song without lyrics, when transcription is
 * on: "Transcribe this song", its progress, or why it didn't work.
 */
export function TranscribeOffer({ trackId }: { trackId: string }) {
  const status = useTranscribe((s) => s.status);
  const empty = useTranscribe((s) => s.empty.has(trackId));
  const on = !!status?.enabled && status.available;
  useEffect(() => {
    if (on) void checkFoundNothing(trackId);
  }, [on, trackId]);
  if (!status || !on) return null;

  const current = status.current?.trackId === trackId ? status.current : undefined;
  if (current) {
    return (
      <div className="lyrics-page__transcribe" role="status">
        <p className="lyrics-page__transcribe-note">{stageText(current, status)}</p>
        <Button variant="ghost" onClick={() => void transcribe.cancel(trackId)}>
          Cancel
        </Button>
      </div>
    );
  }
  const failed = status.error?.trackId === trackId ? status.error : undefined;
  const model = status.models.find((m) => m.kind === status.model);
  const needsModel = !!model && !model.installed;
  return (
    <div className="lyrics-page__transcribe">
      {failed && (
        <p className="lyrics-page__transcribe-note" role="alert">
          {failed.message}
        </p>
      )}
      {empty && !failed && <p className="lyrics-page__transcribe-note">No words were found when this song was transcribed.</p>}
      {!empty && (
        <>
          <Button variant="ghost" icon={<AudioLines size={16} />} onClick={() => void transcribe.track(trackId)}>
            {failed ? "Try again" : "Transcribe this song"}
          </Button>
          <p className="lyrics-page__transcribe-note">
            {needsModel ? `Downloads the ${modelSize(model.sizeBytes)} speech model first. ` : ""}Runs on this computer; the song never leaves it.
          </p>
        </>
      )}
    </div>
  );
}
