/**
 * The library doctor's count for the note under Your Library, kept current
 * from `doctor://changed` and reloaded when its settings change. Zero while
 * the doctor is off.
 */
import { useEffect, useState } from "react";
import { timeAgo } from "../lib/relativeTime";
import { create } from "zustand";
import { backend } from "../lib/backend";
import { useSettings } from "./settings";

interface DoctorState {
  count: number;
  running: boolean;
  set: (count: number, running: boolean) => void;
}

export const useDoctor = create<DoctorState>()((set) => ({
  count: 0,
  running: false,
  set: (count, running) => set({ count, running }),
}));

let started = false;

/** Follow `doctor://changed` (once). */
export function ensureDoctor() {
  if (started) return;
  started = true;
  void backend.listen("doctor://changed", (p) => useDoctor.getState().set(p.count, p.running));
}

export function reloadDoctorCount(): Promise<void> {
  return backend.doctorCount().then(
    (n) => useDoctor.getState().set(n, useDoctor.getState().running),
    () => useDoctor.getState().set(0, false),
  );
}

/** The badge count, loaded on first use and again when the doctor's settings change. */
export function useDoctorCount(): number {
  const on = useSettings((s) => s.settings?.libraryDoctor !== false);
  const checks = useSettings((s) => JSON.stringify(s.settings?.doctorChecks ?? null));
  const px = useSettings((s) => s.settings?.doctorCoverPx);
  const count = useDoctor((s) => s.count);
  useEffect(() => {
    ensureDoctor();
  }, []);
  useEffect(() => {
    void reloadDoctorCount();
  }, [on, checks, px]);
  return on ? count : 0;
}

/** Badge text: up to 99, then 99+. */
export function badgeText(n: number): string {
  return n > 99 ? "99+" : String(n);
}

/**
 * When the library was last checked and nothing needed fixing (unix
 * seconds). Null while there's something to fix, a check is going, album
 * tags are still being read (some checks wait for them), it was never
 * checked, or the doctor is off.
 */
export function useLibraryFineSince(): number | null {
  const on = useSettings((s) => s.settings?.libraryDoctor !== false);
  const count = useDoctorCount();
  const running = useDoctor((s) => s.running);
  const [at, setAt] = useState<number | null>(null);
  useEffect(() => {
    if (!on || count > 0 || running) {
      setAt(null);
      return;
    }
    let alive = true;
    backend.doctorReport().then(
      (r) => {
        if (alive) setAt(r.count === 0 && !r.running && r.tagsRead >= r.tagsTotal ? r.checkedAt : null);
      },
      () => {
        if (alive) setAt(null);
      },
    );
    return () => {
      alive = false;
    };
  }, [on, count, running]);
  return on && count === 0 && !running ? at : null;
}

/** "Your library looks fine. Checked 2 hours ago." */
export function fineLine(at: number): string {
  return `Your library looks fine. Checked ${timeAgo(at).toLowerCase()}.`;
}
