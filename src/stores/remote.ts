/**
 * Plex players, Cast devices and AirPlay receivers for the device picker's
 * "Listen on" section, and moving playback to them and back
 * (`remote_players`, `remote_move`, `remote_move_with_password`,
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
  /** The AirPlay receiver asking for its password, with the last error. */
  password: { player: RemotePlayer; error: string | null } | null;
}

export const useRemote = create<RemoteStore>()(() => ({ players: null, busy: undefined, password: null }));

const message = (e: unknown) =>
  e && typeof e === "object" && "message" in e ? String((e as { message: unknown }).message) : "";

/** The load running now, and whether another was asked for while it ran. */
let inflight: Promise<void> | null = null;
let again = false;

/** Load the players. A load already running is joined rather than doubled;
 * `fresh` asks for one more right after it (a setting changed meanwhile).
 * Rows also arrive on `remote://players` as each source answers. */
export function loadPlayers(b: Backend = backend, opts: { fresh?: boolean } = {}): Promise<void> {
  if (inflight) {
    if (opts.fresh) again = true;
    return inflight;
  }
  const run = (): Promise<void> =>
    b.remotePlayers().then(
      (players) => {
        useRemote.setState({ players });
      },
      () => {
        if (useRemote.getState().players === null) useRemote.setState({ players: [] });
      },
    ).then(() => {
      if (!again) return;
      again = false;
      return run();
    });
  inflight = run().finally(() => {
    inflight = null;
  });
  return inflight;
}

/** Whether a load is running (tests). */
export function loadingPlayers(): boolean {
  return inflight !== null;
}

/** Forget a load left running (tests). */
export function resetPlayerLoads(): void {
  inflight = null;
  again = false;
}

/** Keep the list current while the picker is open: rows as they're found,
 * and a refresh every few seconds that's skipped while one still runs.
 * Returns a cleanup. */
export function watchPlayers(b: Backend = backend): () => void {
  let alive = true;
  let un: (() => void) | null = null;
  b.listen("remote://players", (players) => useRemote.setState({ players })).then(
    (u) => (alive ? (un = u) : u()),
    () => {},
  );
  void loadPlayers(b, { fresh: true });
  const t = setInterval(() => {
    if (!inflight) void loadPlayers(b);
  }, REMOTE_REFRESH_MS);
  return () => {
    alive = false;
    clearInterval(t);
    un?.();
  };
}

const kind = (e: unknown) => (e && typeof e === "object" && "kind" in e ? String((e as { kind: unknown }).kind) : "");

/** Move playback to a player at the current position. An AirPlay receiver
 * with a password Platter doesn't have asks for it first. */
export async function moveTo(player: RemotePlayer, b: Backend = backend): Promise<void> {
  if (useRemote.getState().busy !== undefined) return;
  if (player.unsupported) {
    toast.error(`Can't play on ${player.name}: ${player.unsupported}`);
    return;
  }
  if (player.via === "airplay" && player.needsPassword) {
    useRemote.setState({ password: { player, error: null } });
    return;
  }
  useRemote.setState({ busy: player.id });
  try {
    usePlayer.getState().setRemote(await b.remoteMove(player.id));
  } catch (e) {
    if (player.via === "airplay" && kind(e) === "auth") {
      // A password is needed, or the stored one stopped working.
      useRemote.setState({ password: { player, error: null } });
    } else {
      toast.error(message(e) || `Couldn't reach ${player.name}`);
    }
  } finally {
    useRemote.setState({ busy: undefined });
  }
}

/** Answer the AirPlay password prompt. A wrong password keeps it open with the error. */
export async function submitPassword(password: string, b: Backend = backend): Promise<void> {
  const ask = useRemote.getState().password;
  if (!ask || useRemote.getState().busy !== undefined) return;
  useRemote.setState({ busy: ask.player.id });
  try {
    usePlayer.getState().setRemote(await b.remoteMoveWithPassword(ask.player.id, password));
    useRemote.setState({ password: null });
    void loadPlayers(b);
  } catch (e) {
    if (kind(e) === "auth") {
      useRemote.setState({ password: { player: ask.player, error: message(e) || "That password didn't work." } });
    } else {
      useRemote.setState({ password: null });
      toast.error(message(e) || `Couldn't reach ${ask.player.name}`);
    }
  } finally {
    useRemote.setState({ busy: undefined });
  }
}

export function cancelPassword(): void {
  useRemote.setState({ password: null });
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
