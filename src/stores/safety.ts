/**
 * The library line: "Your library is all here, checked 5 minutes ago", or
 * what left the server this week, with the list of songs that went.
 */
import { useEffect } from "react";
import { create } from "zustand";
import { backend } from "../lib/backend";
import { timeAgo } from "../lib/relativeTime";
import type { GoneSong, LibrarySafety } from "../lib/types";
import { useApp } from "./app";

interface SafetyState {
  safety: LibrarySafety | null;
  gone: GoneSong[] | null;
  /** The list couldn't be read. */
  goneFailed: boolean;
}

export const useSafety = create<SafetyState>(() => ({ safety: null, gone: null, goneFailed: false }));

export async function loadSafety(): Promise<void> {
  try {
    useSafety.setState({ safety: await backend.librarySafety() });
  } catch {
    // The line just stays as it was.
  }
}

export async function loadGone(): Promise<void> {
  try {
    useSafety.setState({ gone: await backend.libraryGone(), goneFailed: false });
  } catch {
    useSafety.setState({ gone: [], goneFailed: true });
  }
}

/** The list was looked at: the line says all is fine again. */
export async function markGoneSeen(): Promise<void> {
  if (useSafety.getState().goneFailed) return;
  const s = useSafety.getState().safety;
  if (s && s.songs === 0 && s.playlists === 0) return;
  try {
    await backend.libraryGoneSeen();
    useSafety.setState({ safety: { songs: 0, playlists: 0 } });
  } catch {
    // Still counted; the next look tries again.
  }
}

/** Keeps the line current: loads now and after every library change. */
export function useLibrarySafety(): LibrarySafety | null {
  const version = useApp((s) => s.libraryVersion);
  useEffect(() => {
    void loadSafety();
  }, [version]);
  return useSafety((s) => s.safety);
}

export interface SafetyLine {
  /** Something left the server: the line leads to the list. */
  alert: boolean;
  title: string;
  sub: string;
}

const count = (n: number, one: string) => `${n} ${one}${n === 1 ? "" : "s"}`;

/**
 * What the line says. Null before the first sync has finished (nothing has
 * been checked yet).
 */
export function safetyLine(s: LibrarySafety | null, lastSyncAt: number | null | undefined, now = Date.now() / 1000): SafetyLine | null {
  if (!lastSyncAt) return null;
  const checked = `Checked ${timeAgo(lastSyncAt, now).toLowerCase()}`;
  if (s && (s.songs > 0 || s.playlists > 0)) {
    const what = [s.songs > 0 ? count(s.songs, "song") : "", s.playlists > 0 ? count(s.playlists, "playlist") : ""].filter(Boolean).join(" and ");
    return { alert: true, title: `${what} left your server`, sub: "See which" };
  }
  return { alert: false, title: "Your library is all here", sub: checked };
}

/** "Today", "Yesterday" or the date, for grouping the list. */
export function goneDay(at: number, now = Date.now() / 1000): string {
  const d = new Date(at * 1000);
  const n = new Date(now * 1000);
  const day = (x: Date) => new Date(x.getFullYear(), x.getMonth(), x.getDate()).getTime();
  const diff = Math.round((day(n) - day(d)) / 86_400_000);
  if (diff <= 0) return "Today";
  if (diff === 1) return "Yesterday";
  return d.toLocaleDateString(undefined, { month: "short", day: "numeric" });
}

/** The songs grouped by day they went, newest day first. */
export function goneByDay(list: GoneSong[], now = Date.now() / 1000): { day: string; songs: GoneSong[] }[] {
  const out: { day: string; songs: GoneSong[] }[] = [];
  for (const g of list) {
    const day = goneDay(g.goneAt, now);
    const last = out[out.length - 1];
    if (last && last.day === day) last.songs.push(g);
    else out.push({ day, songs: [g] });
  }
  return out;
}
