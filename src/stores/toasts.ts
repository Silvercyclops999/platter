import { create } from "zustand";

export interface Toast {
  id: number;
  kind: "info" | "error";
  message: string;
  /** Optional button, e.g. Restart for a downloaded update. */
  action?: { label: string; run: () => void };
  /** A small cover for a toast about one song, album, artist or playlist. */
  art?: ToastArt;
}

export interface ToastArt {
  path: string | null;
  kind?: "album" | "artist" | "playlist";
}

interface ToastState {
  toasts: Toast[];
  push: (t: Omit<Toast, "id">, ttlMs?: number) => number;
  dismiss: (id: number) => void;
}

let seq = 1;
/**
 * Toasts go away by themselves. The UI tests turn this off (src/test/setup.ts)
 * so a toast is still there when a test looks for it, however slow the run;
 * the toast tests turn it back on to check the timing.
 */
export const toastExpiry = { on: true };
/** Each toast's dismiss timer, so a repeat can restart it. */
const timers = new Map<number, ReturnType<typeof setTimeout>>();

export const useToasts = create<ToastState>()((set, get) => ({
  toasts: [],
  push: (t, ttlMs = 4000) => {
    // The same text already showing is replaced in place, not stacked.
    const same = get().toasts.find((x) => x.message === t.message);
    const id = same ? same.id : seq++;
    const old = timers.get(id);
    if (old) clearTimeout(old);
    timers.delete(id);
    const next = same
      ? get().toasts.map((x) => (x.id === id ? { ...t, id } : x))
      : // Keep at most 3 on screen.
        [...get().toasts, { ...t, id }].slice(-3);
    set({ toasts: next });
    if (ttlMs > 0 && toastExpiry.on) timers.set(id, setTimeout(() => get().dismiss(id), ttlMs));
    return id;
  },
  dismiss: (id) => {
    const timer = timers.get(id);
    if (timer) clearTimeout(timer);
    timers.delete(id);
    set({ toasts: get().toasts.filter((x) => x.id !== id) });
  },
}));

export const toast = {
  info: (message: string, art?: ToastArt) => useToasts.getState().push({ kind: "info", message, ...(art?.path ? { art } : {}) }),
  error: (message: string) => useToasts.getState().push({ kind: "error", message }, 6000),
};

/** Copy text to the clipboard and say whether it worked. */
export function copyText(text: string): void {
  if (!navigator.clipboard) {
    toast.error("Couldn't copy");
    return;
  }
  navigator.clipboard.writeText(text).then(
    () => toast.info("Copied to clipboard"),
    () => toast.error("Couldn't copy"),
  );
}
