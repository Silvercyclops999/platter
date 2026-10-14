/**
 * Sidebar pins. App-only state: per device for now (localStorage); the
 * optional sync file (Phase 3) will carry it across devices.
 */
import { create } from "zustand";

export interface Pin {
  kind: "album" | "artist" | "playlist";
  id: string;
  title: string;
  subtitle: string;
  thumb: string | null;
}

const KEY = "platter.pins.v1"; // gitleaks:allow (localStorage key name, not a secret)

function load(): Pin[] {
  try {
    const raw = localStorage.getItem(KEY);
    return raw ? (JSON.parse(raw) as Pin[]) : [];
  } catch {
    return [];
  }
}

function save(pins: Pin[]) {
  try {
    localStorage.setItem(KEY, JSON.stringify(pins));
  } catch {
    /* storage unavailable: pins live for this session only */
  }
}

interface PinState {
  pins: Pin[];
  isPinned: (kind: Pin["kind"], id: string) => boolean;
  toggle: (p: Pin) => void;
}

export const usePins = create<PinState>()((set, get) => ({
  pins: load(),
  isPinned: (kind, id) => get().pins.some((p) => p.kind === kind && p.id === id),
  toggle: (p) => {
    const pins = get().isPinned(p.kind, p.id)
      ? get().pins.filter((x) => !(x.kind === p.kind && x.id === p.id))
      : [p, ...get().pins].slice(0, 12);
    save(pins);
    set({ pins });
  },
}));
