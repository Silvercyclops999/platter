/**
 * The library doctor's count for the badge in the app menu, kept current
 * from `doctor://changed` and reloaded when its settings change. Zero while
 * the doctor is off.
 */
import { useEffect } from "react";
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
