/**
 * Sidebar pins (playlists, albums, artists and folders), kept by the core (`pins_get` / `pins_toggle`) so they travel
 * in the sync file when it's on. Toggling shows at once; the core's list
 * comes back when another device changes it. Pins an older build kept in
 * the window's storage are handed to the core once.
 */
import { create } from "zustand";
import { backend, type Backend } from "../lib/backend";
import type { PinRecord } from "../lib/types";
import { toast } from "./toasts";

export interface Pin {
  kind: "album" | "artist" | "playlist" | "folder";
  id: string;
  title: string;
  subtitle: string;
  thumb: string | null;
}

/** A handful, not a second library. */
export const MAX_PINS = 4;
/** Where an older build kept pins. */
const LEGACY_KEY = "platter.pins.v1"; // gitleaks:allow (localStorage key name, not a secret)

const strip = (p: PinRecord): Pin => ({ kind: p.kind, id: p.id, title: p.title, subtitle: p.subtitle ?? "", thumb: p.thumb ?? null });

function legacyPins(): Pin[] {
  try {
    const raw = localStorage.getItem(LEGACY_KEY);
    return raw ? (JSON.parse(raw) as Pin[]) : [];
  } catch {
    return [];
  }
}

function dropLegacy() {
  try {
    localStorage.removeItem(LEGACY_KEY);
  } catch {
    /* storage unavailable */
  }
}

interface PinState {
  pins: Pin[];
  isPinned: (kind: Pin["kind"], id: string) => boolean;
  toggle: (p: Pin) => void;
}

export const usePins = create<PinState>()((set, get) => ({
  pins: [],
  isPinned: (kind, id) => get().pins.some((p) => p.kind === kind && p.id === id),
  toggle: (p) => {
    const before = get().pins;
    const pinned = get().isPinned(p.kind, p.id);
    if (!pinned && before.length >= MAX_PINS) {
      toast.error(`You can pin up to ${MAX_PINS} items. Unpin one first.`);
      return;
    }
    const pins = pinned ? before.filter((x) => !(x.kind === p.kind && x.id === p.id)) : [p, ...before];
    set({ pins });
    backend.pinsToggle(p).catch(() => {
      set({ pins: before });
      toast.error(pinned ? "Couldn't unpin that" : "Couldn't pin that");
    });
  },
}));

/** Read the pins from the core (moving an older build's pins over first). */
export async function loadPins(b: Backend = backend): Promise<void> {
  try {
    const legacy = legacyPins();
    const pins = legacy.length ? await b.pinsImport(legacy) : await b.pinsGet();
    if (legacy.length) dropLegacy();
    usePins.setState({ pins: pins.map(strip) });
  } catch {
    /* keep what's shown */
  }
}
