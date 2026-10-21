import { Loader2 } from "lucide-react";
import { useEffect, useState, type ReactNode } from "react";
import { Button } from "../components/Button";
import { Dialog } from "../components/Dialog";
import { backend } from "../lib/backend";
import { formatDuration } from "../lib/format";
import type { LyricsShareInfo, Lyrics, Track } from "../lib/types";
import { lyricsShareChanged, markReviewed, PLAYED_THROUGH, useLyricsShare } from "../stores/lyricsShare";
import { usePlayer } from "../stores/player";
import { toast } from "../stores/toasts";
import "./share-transcription.css";

export const SHARE_LABEL = "Share these lyrics publicly";
export const SHARE_NOTE = "Lyrics made on this computer can have mistakes. Check them first. Once shared, anyone can get them.";
export const UNRELEASED_NOTE = "This song is unreleased. Sharing publishes its title and these lyrics for anyone to find.";
export const SHARED_TOAST = "Lyrics shared to LRCLIB";

function errText(e: unknown): string {
  const m = (e as { message?: unknown } | null)?.message;
  return typeof m === "string" && m ? m : "Couldn't share these lyrics. Nothing was shared.";
}

/** Counts the song as reviewed once it plays through with its lyrics showing. */
function useWatchPlayThrough(track: Track, watching: boolean) {
  useEffect(() => {
    if (!watching) return;
    let heard = 0;
    let last = -1;
    const unsub = usePlayer.subscribe((s) => {
      if (s.state?.track?.id !== track.id) {
        last = -1;
        return;
      }
      const pos = s.positionMs;
      // Moving forward while playing, a little at a time (a seek doesn't count).
      if (last >= 0 && s.state.status === "playing" && pos > last && pos - last < 5000) heard += pos - last;
      last = pos;
      const dur = s.durationMs || track.durationMs;
      if (dur > 0 && heard >= dur * PLAYED_THROUGH) {
        unsub();
        void markReviewed(track.id);
      }
    });
    return unsub;
  }, [track.id, track.durationMs, watching]);
}

/**
 * For a song showing a transcription: whether "Share these lyrics publicly"
 * is offered (once the user reviewed them and the song qualifies), the
 * action that opens its dialog, and the dialog to render. The dialog shows
 * exactly what LRCLIB gets; nothing is sent until the user confirms there.
 */
export function useShareTranscription(lyrics: Lyrics, track: Track): { offer: boolean; open: () => void; dialog: ReactNode } {
  const auto = lyrics.source === "auto";
  const version = useLyricsShare((s) => s.version);
  const [info, setInfo] = useState<LyricsShareInfo | null>(null);
  const [open, setOpen] = useState(false);

  useEffect(() => {
    setInfo(null);
    if (!auto) return;
    let live = true;
    backend.lyricsShareInfo(track.id).then(
      (i) => live && setInfo(i),
      () => live && setInfo(null),
    );
    return () => {
      live = false;
    };
  }, [auto, track.id, version]);

  useWatchPlayThrough(track, auto && info?.whyNot === "notReviewed");

  const offer = auto && !!info?.offer && !!info.payload;
  return {
    offer,
    open: () => setOpen(true),
    dialog: offer && open && info ? <ShareDialog info={info} track={track} onClose={() => setOpen(false)} /> : null,
  };
}

function ShareDialog({ info, track, onClose }: { info: LyricsShareInfo; track: Track; onClose: () => void }) {
  const p = info.payload!;
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const share = async () => {
    setBusy(true);
    setError(null);
    try {
      await backend.lyricsSharePublish(track.id);
      onClose();
      lyricsShareChanged();
      void toast.info(SHARED_TOAST);
    } catch (e) {
      setError(errText(e));
    } finally {
      setBusy(false);
    }
  };

  const close = () => {
    if (busy) void backend.lyricsShareCancel(track.id);
    onClose();
  };

  return (
    <Dialog open onClose={close} label="Share lyrics publicly" className="confirm share-lyrics">
      <h2 className="confirm__title">Share lyrics publicly?</h2>
      <p className="confirm__body">{SHARE_NOTE}</p>
      {info.unreleased && (
        <p className="share-lyrics__warning" role="note">
          {UNRELEASED_NOTE}
        </p>
      )}
      <p className="share-lyrics__what">These go to LRCLIB, a free public lyrics library:</p>
      <dl className="share-lyrics__details">
        <dt>Title</dt>
        <dd>{p.trackName}</dd>
        <dt>Artist</dt>
        <dd>{p.artistName}</dd>
        <dt>Album</dt>
        <dd>{p.albumName}</dd>
        <dt>Length</dt>
        <dd>{formatDuration(p.duration * 1000)}</dd>
      </dl>
      <pre className="share-lyrics__lyrics" tabIndex={0} aria-label="Synced lyrics to send">
        {p.syncedLyrics}
      </pre>
      {error && (
        <p className="confirm__error" role="alert">
          {error}
        </p>
      )}
      <div className="confirm__actions">
        <Button variant="ghost" onClick={close}>
          Cancel
        </Button>
        <Button variant="primary" disabled={busy} onClick={() => void share()}>
          {busy ? (
            <>
              <Loader2 size={14} aria-hidden="true" className="lyrics-trust__spin" /> Sharing
            </>
          ) : (
            "Share"
          )}
        </Button>
      </div>
    </Dialog>
  );
}
