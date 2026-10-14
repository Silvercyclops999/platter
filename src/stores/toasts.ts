import { create } from "zustand";

export interface Toast {
  id: number;
  kind: "info" | "error";
  message: string;
  /** Optional button, e.g. Restart for a downloaded update. */
  action?: { label: string; run: () => void };
}

interface ToastState {
  toasts: Toast[];
  push: (t: Omit<Toast, "id">, ttlMs?: number) => number;
  dismiss: (id: number) => void;
}

let seq = 1;

export const useToasts = create<ToastState>()((set, get) => ({
  toasts: [],
  push: (t, ttlMs = 4000) => {
    const id = seq++;
    // Keep at most 3 on screen.
    set({ toasts: [...get().toasts, { ...t, id }].slice(-3) });
    if (ttlMs > 0) setTimeout(() => get().dismiss(id), ttlMs);
    return id;
  },
  dismiss: (id) => set({ toasts: get().toasts.filter((x) => x.id !== id) }),
}));

export const toast = {
  info: (message: string) => useToasts.getState().push({ kind: "info", message }),
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
