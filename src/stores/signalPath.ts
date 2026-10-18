/**
 * The signal path, mirrored from the core (`signal_path`,
 * `audio://signal-path`), and whether its panel is open.
 */
import { create } from "zustand";
import { backend, type Backend } from "../lib/backend";
import type { SignalPath } from "../lib/types";

interface SignalStore {
  path: SignalPath | null;
  open: boolean;
  set: (p: SignalPath) => void;
  setOpen: (open: boolean) => void;
}

export const useSignal = create<SignalStore>()((set) => ({
  path: null,
  open: false,
  set: (path) => set({ path }),
  setOpen: (open) => set({ open }),
}));

export function loadSignal(b: Backend = backend): Promise<void> {
  return b.signalPath().then(
    (p) => useSignal.getState().set(p),
    () => {},
  );
}

/** Opens the panel (from the badge, a menu or the expanded view) with a fresh snapshot. */
export function openSignal(b: Backend = backend) {
  useSignal.getState().setOpen(true);
  void loadSignal(b);
}

export function closeSignal() {
  useSignal.getState().setOpen(false);
}
