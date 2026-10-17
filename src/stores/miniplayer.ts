/**
 * The miniplayer window from the main window's side: whether it's open (the
 * player bar button and the app menu show it), and opening or closing it.
 * The window itself is made by the core; its page is `MiniplayerApp`.
 */
import { create } from "zustand";
import { backend, type Backend } from "../lib/backend";
import { nav } from "./router";
import { toast } from "./toasts";

/** Said when the window couldn't be made. */
export const MINIPLAYER_FAILED = "The miniplayer couldn't open. Try again, or restart Platter if it keeps happening.";

interface MiniplayerStore {
  open: boolean;
  set: (open: boolean) => void;
}

export const useMiniplayer = create<MiniplayerStore>()((set) => ({
  open: false,
  set: (open) => set({ open }),
}));

/** This page is the miniplayer window's. */
export const isMiniplayerWindow = (b: Backend = backend): boolean => {
  try {
    return b.windowLabel() === "miniplayer";
  } catch {
    return false;
  }
};

export const miniplayer = {
  /** Open it, or close it when it's open. */
  toggle: (b: Backend = backend) => miniplayer.setOpen(!useMiniplayer.getState().open, b),
  setOpen: async (open: boolean, b: Backend = backend) => {
    try {
      const st = await b.miniplayerSet(open);
      useMiniplayer.getState().set(st.open);
    } catch {
      if (open) toast.error(MINIPLAYER_FAILED);
    }
  },
};

/**
 * Follow the window opening and closing, and open the pages it asks for
 * (Go to album from its queue). Returns a cleanup.
 */
export function connectMiniplayer(b: Backend = backend): () => void {
  let alive = true;
  const uns: (() => void)[] = [];
  const keep = (u: () => void) => (alive ? uns.push(u) : u());
  b.miniplayerState().then(
    (s) => alive && useMiniplayer.getState().set(s.open),
    () => {},
  );
  b.listen("miniplayer://changed", (s) => useMiniplayer.getState().set(s.open)).then(keep);
  b.listen("miniplayer://go", (g) => nav.go({ name: g.kind, id: g.id })).then(keep);
  return () => {
    alive = false;
    uns.forEach((u) => u());
  };
}
