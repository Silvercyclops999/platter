import { useEffect, useRef, useState } from "react";
import { sideOf, useAlbumSides } from "../lib/useAlbumSides";
import { player, usePlayer } from "../stores/player";
import { useSettings } from "../stores/settings";

export interface Flip {
  /** The side that just ended. */
  from: string;
  /** The side waiting ("B"). */
  to: string;
  trackId: string;
}

/**
 * "Pause to flip": while the record player shows (`shown`) and the setting
 * is on, the end of a side pauses playback at the start of the next side's
 * first song, so the tonearm lifts, until Flip plays on. Only a move from
 * one side to the very next of the same album counts; skipping around,
 * other albums and anything else play as always.
 */
export function useFlipPause(shown: boolean): { flip: Flip | null; play: () => void } {
  const on = useSettings((s) => s.settings?.flipPause === true && s.settings?.showVinylSides !== false) && shown;
  const track = usePlayer((s) => s.state?.track);
  const status = usePlayer((s) => s.state?.status);
  const [sides] = useAlbumSides(on ? track?.albumId : null);
  const [flip, setFlip] = useState<Flip | null>(null);
  const prev = useRef<{ id: string; albumId: string } | null>(null);
  const sawPause = useRef(false);

  useEffect(() => {
    const before = prev.current;
    prev.current = track ? { id: track.id, albumId: track.albumId } : null;
    if (!track) return;
    setFlip((f) => (f && f.trackId !== track.id ? null : f));
    if (!on || !before || before.id === track.id || before.albumId !== track.albumId || !sides?.sides.length) return;
    const list = sides.sides;
    const a = sideOf(list, before.id);
    const b = sideOf(list, track.id);
    if (!a || !b || list.indexOf(b) !== list.indexOf(a) + 1 || b.trackIds[0] !== track.id) return;
    const st = usePlayer.getState().state?.status;
    if (st !== "playing" && st !== "buffering" && st !== "loading") return;
    sawPause.current = false;
    setFlip({ from: a.label, to: b.label, trackId: track.id });
    void player.pause();
    void player.seek(0);
    // Only the track change matters here; the rest is read as it is then.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [track?.id]);

  // Playing on some other way (the play button, a key) puts Flip away.
  useEffect(() => {
    if (!flip) return;
    if (status === "paused") sawPause.current = true;
    else if (status === "playing" && sawPause.current) setFlip(null);
  }, [status, flip]);

  // Turning it off, or leaving the record player, puts Flip away too.
  useEffect(() => {
    if (!on) setFlip(null);
  }, [on]);

  return {
    flip,
    play: () => {
      setFlip(null);
      void player.resume();
    },
  };
}
