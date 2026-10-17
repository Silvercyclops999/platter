/**
 * Switching Plex Home users restarts the app as the other user. The
 * "Switching to…" screen covers the window, in the current look, from the
 * moment a user is picked until the window goes away for the restart, so
 * the page never vanishes or flashes mid-switch.
 *
 * Just before restarting the core sends `home://changed` with
 * `restarting: true` (also when signing out as another Home user) and waits
 * a moment. When the switch fails (a wrong PIN, no network) nothing has
 * changed: the screen goes and a toast says why. The screen is never left up:
 * if the restart doesn't happen within `STUCK_AFTER_MS`, it goes too.
 */
import { create } from "zustand";
import { backend, type Backend } from "../lib/backend";
import { toast } from "./toasts";

/** Who the switch is to. */
export interface SwitchTarget {
  name: string;
  /** Profile picture as an art path, or null. */
  avatar: string | null;
}

interface SwitchingState {
  /** Who the switch is to (null when not known, as after signing out). */
  to: SwitchTarget | null;
  /** The switching screen is up. */
  shown: boolean;
  /** The core said it's restarting. */
  restarting: boolean;
}

const IDLE: SwitchingState = { to: null, shown: false, restarting: false };

export const useSwitching = create<SwitchingState>()(() => ({ ...IDLE }));

/** A switch that hasn't restarted the app after this long has failed. */
export const STUCK_AFTER_MS = 20_000;

let watchdog: ReturnType<typeof setTimeout> | null = null;

function watch() {
  if (watchdog) clearTimeout(watchdog);
  watchdog = setTimeout(() => {
    watchdog = null;
    if (!useSwitching.getState().shown) return;
    const to = useSwitching.getState().to;
    hide();
    toast.error(to ? `Couldn't switch to ${to.name}. You're still using Platter as before` : "Couldn't switch users");
  }, STUCK_AFTER_MS);
}

function hide() {
  if (watchdog) clearTimeout(watchdog);
  watchdog = null;
  useSwitching.setState({ ...IDLE });
}

/** A user was picked: cover the window until the restart. */
export function beginSwitch(to: SwitchTarget): void {
  useSwitching.setState({ to: { name: to.name, avatar: to.avatar }, shown: true, restarting: false });
  watch();
}

/**
 * The switch call came back. Without a restart (nothing to reload) the
 * screen goes; with one, the window is about to close as it is.
 */
export function switchDone(): void {
  if (!useSwitching.getState().restarting) hide();
}

/** The switch failed: back to the page as it was, with a toast saying why. */
export function switchFailed(message: string | null | undefined): void {
  const to = useSwitching.getState().to;
  hide();
  const why = message?.trim();
  toast.error(to ? `Couldn't switch to ${to.name}${why ? `: ${why}` : "."}` : why || "Couldn't switch users");
}

/** What the switching screen says. */
export function switchingText(to: SwitchTarget | null): string {
  return to ? `Switching to ${to.name}` : "Switching users";
}

/** Follow the core: up when it's restarting, down when it says it's not. */
export function connectSwitching(b: Backend = backend): () => void {
  let alive = true;
  let un: (() => void) | null = null;
  b.listen("home://changed", (p) => {
    if (p.restarting) {
      useSwitching.setState({ shown: true, restarting: true });
      watch();
    } else hide();
  }).then((u) => (alive ? (un = u) : u()));
  return () => {
    alive = false;
    un?.();
  };
}

/** Tests: back to no switch. */
export function resetSwitchingForTests(): void {
  hide();
}
