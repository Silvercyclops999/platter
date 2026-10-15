/**
 * The player bar's two buttons while a smart shuffle suggestion plays:
 * remove it (skip it and pick new ones) or add it to the playlist (Liked
 * Songs: like it), after which it plays on as one of the playlist's songs.
 */
import { backend } from "../lib/backend";
import type { PlayerState, QueueItem } from "../lib/types";
import { setLiked } from "./likes";
import { addNow } from "./playlists";
import { toast } from "./toasts";

/** The suggestion playing now and the playlist it's in, if one is. */
export function playingSuggestion(state: PlayerState | null | undefined): { item: QueueItem; contextId: string; title: string; liked: boolean } | null {
  const item = state?.queue.current;
  const ctx = state?.queue.context;
  if (!item || item.source !== "smart" || !ctx?.id) return null;
  if (ctx.kind !== "playlist" && ctx.kind !== "liked") return null;
  return { item, contextId: ctx.id, title: ctx.kind === "liked" ? "Liked Songs" : (ctx.title ?? "playlist"), liked: ctx.kind === "liked" };
}

export async function removeSuggestion(): Promise<void> {
  try {
    await backend.smartSuggestionRemove();
  } catch {
    toast.error("Couldn't remove the suggestion");
  }
}

export async function keepSuggestion(state: PlayerState | null | undefined): Promise<void> {
  const s = playingSuggestion(state);
  if (!s) return;
  const ok = s.liked
    ? await setLiked([s.item.track], true)
    : await addNow({ id: s.contextId, title: s.title, thumb: null }, [s.item.track.id]);
  if (!ok) return;
  await backend.smartSuggestionKeep(s.item.itemId).catch(() => undefined);
}
