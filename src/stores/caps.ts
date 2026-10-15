/**
 * What the servers in use can do (`backend_caps`). Features that need a
 * capability hide when the item's server doesn't have it. Until the core
 * answers, everything counts as available, so nothing flickers away.
 */
import { create } from "zustand";
import { backend } from "../lib/backend";
import { ALL_CAPS, capFor, serverFor } from "../lib/caps";
import type { BackendCaps, CapName } from "../lib/types";

export const ASSUMED_CAPS: BackendCaps = { caps: ALL_CAPS, servers: [] };

export const useCaps = create<{ all: BackendCaps }>()(() => ({ all: ASSUMED_CAPS }));

let started = false;

function load() {
  backend.backendCaps().then(
    (all) => useCaps.setState({ all }),
    () => {},
  );
}

/** Ask the core once, then again whenever the servers or the sign-in change. */
export function ensureCaps() {
  if (started) return;
  started = true;
  load();
  void backend.listen("backends://changed", load);
  void backend.listen("auth://changed", load);
}

/** Whether the server with `id` (or, without one, the primary server) can do `cap`. */
export function hasCap(cap: CapName, id?: string | null): boolean {
  ensureCaps();
  return capFor(useCaps.getState().all, id, cap);
}

/** [`hasCap`] as a hook. */
export function useCap(cap: CapName, id?: string | null): boolean {
  ensureCaps();
  return useCaps((s) => capFor(s.all, id, cap));
}

/** The name of the server with `id` (the primary without one), when known. */
export function useServerName(id?: string | null): string | null {
  ensureCaps();
  return useCaps((s) => serverFor(s.all, id)?.name ?? null);
}
