/**
 * Plex players for the device picker's "Listen on" section, and moving
 * playback to them and back (`remote_players`, `remote_move`,
 * `remote_return`). The remote session itself lives in the player store.
 */
import { create } from "zustand";
import { backend, type Backend } from "../lib/backend";
import type { RemotePlayer } from "../lib/types";
import { usePlayer } from "./player";
import { toast } from "./toasts";

/** How often players refresh while the picker is open. */
export const REMOTE_REFRESH_MS = 10_000;

interface RemoteStore {
  /** null until the first load. */
  players: RemotePlayer[] | null;
  /** A move in flight (to this player id), or null while coming back here. */
  busy: string | null | undefined;
}

export const useRemote = create<RemoteStore>()(() => ({ players: null, busy: undefined }));

const message = (e: unknown) =>
  e && typeof e === "object" && "message" in e ? String((e as { message: unknown }).message) : "";

export function loadPlayers(b: Backend = backend): Promise<void> {
  return b.remotePlayers().then(
    (players) => useRemote.setState({ players }),
    () => {
      if (useRemote.getState().players === null) useRemote.setState({ players: [] });
    },
  );
}

/** Keep the list current while the picker is open. Returns a cleanup. */
export function watchPlayers(b: Backend = backend): () => void {
  let alive = true;
  let un: (() => void) | null = null;
  void loadPlayers(b);
  const t = setInterval(() => void loadPlayers(b), REMOTE_REFRESH_MS);
  b.listen("remote://players", (players) => useRemote.setState({ players })).then((u) => (alive ? (un = u) : u()));
  return () => {
    alive = false;
    clearInterval(t);
    un?.();
  };
}

/** Move playback to a player at the current position. */
export async function moveTo(player: RemotePlayer, b: Backend = backend): Promise<void> {
  if (useRemote.getState().busy !== undefined) return;
  useRemote.setState({ busy: player.id });
  try {
    usePlayer.getState().setRemote(await b.remoteMove(player.id));
  } catch (e) {
    toast.error(message(e) || `Couldn't reach ${player.name}`);
  } finally {
    useRemote.setState({ busy: undefined });
  }
}

/** Bring playback back to this computer at the player's position. */
export async function returnHere(b: Backend = backend): Promise<void> {
  if (useRemote.getState().busy !== undefined) return;
  useRemote.setState({ busy: null });
  try {
    usePlayer.getState().setRemote(await b.remoteReturn());
  } catch (e) {
    toast.error(message(e) || "Couldn't bring playback back here");
  } finally {
    useRemote.setState({ busy: undefined });
  }
}
