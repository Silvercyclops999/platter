/**
 * Bringing the computer's setup over to the phone. After the first sign-in
 * it runs by itself in the background: nothing to choose and no screen, and
 * a short line says what came once it's done. Every Platter on the same Plex
 * account finds the others by itself, so there's no code to type; when no
 * computer turns up in time, everyday sync brings the same things over the
 * next time both are open.
 */
import { backend } from "../lib/backend";
import { toast } from "../stores/toasts";

/** How long a first copy waits in the background for a computer to turn up. */
export const BACKGROUND_WAIT_MS = 10 * 60_000;

/**
 * Copy from the computer quietly, once one turns up within a while. With a
 * computer already linked it's a sign-in again, and nothing is copied. Only a finished copy says anything; a failure
 * stays quiet because the next sync round tries again.
 */
export function copyInBackground(waitMs = BACKGROUND_WAIT_MS): void {
  let done = false;
  let unlisten: (() => void) | null = null;
  const finish = () => {
    done = true;
    window.clearTimeout(timer);
    unlisten?.();
  };
  const run = () => {
    finish();
    backend.pairCopySettings().then(
      (r) => {
        if (!r.cancelled && r.groups.some((g) => g.count > 0)) toast.info(`Brought your setup over from ${r.name}`);
      },
      (e: unknown) => console.warn("first copy", e),
    );
  };
  // The first look decides: a computer already linked means this is a sign-in
  // again, and the phone keeps its own settings (signing in changes nothing).
  let first = true;
  const look = () =>
    backend.deviceSyncStatus().then(
      (o) => {
        if (done) return;
        const wasFirst = first;
        first = false;
        if (!o.on || (wasFirst && o.devices.length > 0)) finish();
        else if (o.devices.length > 0) run();
      },
      () => {},
    );
  const timer = window.setTimeout(finish, waitMs);
  void backend
    .listen("device-sync://status", (devices) => {
      if (!done && devices.length > 0) void look();
    })
    .then((u) => {
      if (done) u();
      else unlisten = u;
      // Look once listening, so one that linked in between isn't missed.
      void look();
    });
}
