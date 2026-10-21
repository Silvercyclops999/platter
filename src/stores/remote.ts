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
/** How long a pick of a phone that's asleep waits for it to wake. */
export const WAKE_WAIT_MS = 5 * 60_000;
/** How often the list is checked while waiting for it. */
export const WAKE_POLL_MS = 4_000;

interface RemoteStore {
  /** null until the first load. */
  players: RemotePlayer[] | null;
  /** A move in flight (to this player id), or null while coming back here. */
  busy: string | null | undefined;
  /** The AirPlay receiver asking for its password, with the last error. */
  password: { player: RemotePlayer; error: string | null } | null;
  /** A phone picked while asleep: playback moves there once it answers. */
  waiting: { id: string; name: string } | null;
}

export const useRemote = create<RemoteStore>()(() => ({ players: null, busy: undefined, password: null, waiting: null }));

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

/** Stops the wait for a phone (cleanup of the running one). */
let stopWaiting: (() => void) | null = null;

/** Whether there's playback here (or elsewhere) to move. */
const somethingToMove = () => !!usePlayer.getState().state?.track || !!usePlayer.getState().remote;

/** Whether a listed player answers now. */
const awake = (p: RemotePlayer) => p.online && !p.asleep && !p.unreachable;

/** Stop waiting for a phone to wake. */
export function cancelWaiting(): void {
  stopWaiting?.();
}

/**
 * A phone can't be woken from here: iOS keeps Platter asleep until it's
 * opened on the phone. So picking one waits for it, checking the list
 * every few seconds, and moves playback the moment it answers. Picking it
 * again, or anything else, stops the wait; after a few minutes it gives up.
 */
function waitFor(player: RemotePlayer, b: Backend): void {
  stopWaiting?.();
  useRemote.setState({ waiting: { id: player.id, name: player.name } });
  const started = Date.now();
  let done = false;
  const stop = () => {
    if (done) return;
    done = true;
    clearInterval(t);
    unsub();
    if (stopWaiting === stop) stopWaiting = null;
    if (useRemote.getState().waiting?.id === player.id) useRemote.setState({ waiting: null });
  };
  const check = (players: RemotePlayer[] | null) => {
    const now = players?.find((p) => p.id === player.id);
    if (!now || !awake(now)) return;
    // Another move or a password prompt in flight: try again next time.
    if (useRemote.getState().busy !== undefined) return;
    stop();
    if (!somethingToMove()) return;
    void moveTo(now, b);
  };
  const unsub = useRemote.subscribe((s, prev) => {
    if (s.players !== prev.players) check(s.players);
  });
  const t = setInterval(() => {
    if (Date.now() - started >= WAKE_WAIT_MS) {
      stop();
      toast.error(`${player.name} didn't wake up. Open Platter on it, then pick it again.`);
      return;
    }
    if (!inflight) void loadPlayers(b);
  }, WAKE_POLL_MS);
  stopWaiting = stop;
  void loadPlayers(b, { fresh: true });
}

/** Move playback to a player at the current position. An AirPlay receiver
 * with a password Platter doesn't have asks for it first, and a phone
 * that's asleep is waited for. */
export async function moveTo(player: RemotePlayer, b: Backend = backend): Promise<void> {
  if (useRemote.getState().busy !== undefined) return;
  const waiting = useRemote.getState().waiting;
  if (waiting) {
    cancelWaiting();
    // Picking the phone again while waiting is how the wait is called off.
    if (waiting.id === player.id) return;
  }
  if (player.asleep) {
    if (!somethingToMove()) {
      toast.error("Play something first, then move it.");
      return;
    }
    waitFor(player, b);
    return;
  }
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
  cancelWaiting();
  useRemote.setState({ busy: null });
  try {
    usePlayer.getState().setRemote(await b.remoteReturn());
  } catch (e) {
    toast.error(message(e) || "Couldn't bring playback back here");
  } finally {
    useRemote.setState({ busy: undefined });
  }
}
