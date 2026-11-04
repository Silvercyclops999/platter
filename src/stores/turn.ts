/**
 * Turn it: one tap moves the next songs toward a different mood from your
 * own library, then the queue carries on as it was. The songs go in as
 * "play next", with Undo on the toast.
 */
import { useEffect, useState } from "react";
import { backend } from "../lib/backend";
import type { QueueView, TurnChoice } from "../lib/types";
import { player, usePlayer } from "./player";
import { guestLive } from "./social";
import { toast } from "./toasts";

/** Every song id the queue already holds (played, playing and to come). */
export function queuedIds(q: QueueView | null | undefined): string[] {
  if (!q) return [];
  return [q.current, ...q.history, ...q.userQueue, ...q.nextUp].flatMap((i) => (i ? [i.track.id] : []));
}

/** The moods to offer for the playing song; empty when there's nothing to turn. */
export async function turnChoices(trackId: string): Promise<TurnChoice[]> {
  if (guestLive()) return [];
  try {
    return await backend.turnChoices(trackId);
  } catch {
    return [];
  }
}

/** Take back a turn: the songs it put up next, if they're still waiting. */
export function undoTurn(ids: string[]): void {
  const left = new Set(ids);
  const q = usePlayer.getState().state?.queue;
  for (const item of q?.userQueue ?? []) {
    if (!left.has(item.track.id)) continue;
    left.delete(item.track.id);
    void player.queueRemove(item.itemId);
  }
}

/** Put the turn's songs up next. False when nothing could be added (a toast says why). */
export async function turnTo(mood: string): Promise<boolean> {
  const st = usePlayer.getState().state;
  const trackId = st?.track?.id;
  if (!trackId) return false;
  let ids: string[];
  try {
    ids = await backend.turnSongs(trackId, mood, queuedIds(st?.queue));
  } catch {
    toast.error("Couldn't turn the music just now. Try again in a moment.");
    return false;
  }
  if (!ids.length) {
    toast.info(`Every ${mood.toLowerCase()} song is already in your queue`);
    return false;
  }
  try {
    await backend.queueAdd(ids, "next");
  } catch {
    toast.error("Couldn't add the songs to your queue. Try again in a moment.");
    return false;
  }
  toast.undo(`Turning ${mood.toLowerCase()}: ${ids.length === 1 ? "1 song" : `${ids.length} songs`} up next`, () => undoTurn(ids));
  return true;
}

/** The moods for the playing song, looked up once per song. */
export function useTurnChoices(trackId: string | null): TurnChoice[] {
  const [got, setGot] = useState<{ id: string; list: TurnChoice[] } | null>(null);
  useEffect(() => {
    if (!trackId) return;
    let alive = true;
    void turnChoices(trackId).then((list) => alive && setGot({ id: trackId, list }));
    return () => {
      alive = false;
    };
  }, [trackId]);
  return got && got.id === trackId ? got.list : [];
}
