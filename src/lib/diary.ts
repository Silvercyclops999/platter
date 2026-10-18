/**
 * The listening diary's words and dates, shared by the Diary page, the
 * album page's First heard line and its Log this album item. Dates are
 * this computer's local ones.
 */
import { useEffect, useState } from "react";
import { backend } from "./backend";
import { errorMessage } from "./errorText";
import type { DiaryItem } from "./types";
import { nav } from "../stores/router";
import { useSettings } from "../stores/settings";
import { toast, useToasts } from "../stores/toasts";

export const MONTHS = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
export const WEEKDAYS = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
export const NOTE_MAX = 500;

/** "March 3, 2021". */
export function longDate(ms: number): string {
  const d = new Date(ms);
  return `${MONTHS[d.getMonth()]} ${d.getDate()}, ${d.getFullYear()}`;
}

/** "2026-10" for the month an entry falls in. */
export function monthKey(ms: number): string {
  const d = new Date(ms);
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}`;
}

/** "October 2026" for a month key. */
export function monthLabel(key: string): string {
  const [y, m] = key.split("-").map(Number);
  return `${MONTHS[m - 1]} ${y}`;
}

/** Entries (newest first) grouped by month, newest month first. */
export function byMonth(entries: DiaryItem[]): { key: string; entries: DiaryItem[] }[] {
  const out: { key: string; entries: DiaryItem[] }[] = [];
  for (const e of entries) {
    const key = monthKey(e.at);
    const last = out[out.length - 1];
    if (last?.key === key) last.entries.push(e);
    else out.push({ key, entries: [e] });
  }
  return out;
}

/** Local start of a year and of the next, in unix ms. */
export function yearRange(year: number): { from: number; until: number } {
  return { from: new Date(year, 0, 1).getTime(), until: new Date(year + 1, 0, 1).getTime() };
}

/** Years from the newest entry's back to the oldest's. */
export function diaryYears(firstAt: number | null, lastAt: number | null): number[] {
  if (firstAt == null || lastAt == null) return [];
  const out: number[] = [];
  for (let y = new Date(lastAt).getFullYear(); y >= new Date(firstAt).getFullYear(); y--) out.push(y);
  return out;
}

/** "4.5 stars", "1 star". */
export function starsText(r: number): string {
  return r === 1 ? "1 star" : `${r} stars`;
}

/** "Played all 10 songs, 41 min" or "Played 9 of 10 songs", or "Logged by hand". */
export function playedLine(e: DiaryItem): string {
  if (!e.auto) return "Logged by hand";
  const songs = e.songsPlayed >= e.songsTotal ? `Played all ${e.songsTotal} songs` : `Played ${e.songsPlayed} of ${e.songsTotal} songs`;
  return e.minutes > 0 ? `${songs}, ${e.minutes} min` : songs;
}

/** "On headphones, Den PC" from the output and computer, or "". */
export function whereLine(e: DiaryItem): string {
  const kind = e.listenedOn === "headphones" ? "headphones" : e.listenedOn === "speakers" ? "speakers" : null;
  const out = kind ? `On ${kind}` : e.output ? `On ${e.output}` : null;
  if (out && e.device) return `${out}, ${e.device}`;
  if (out) return out;
  return e.device ? `On ${e.device}` : "";
}

/** Whether the diary is kept (the setting). */
export function useDiaryOn(): boolean {
  return useSettings((s) => s.settings?.keepDiary !== false);
}

/** A number that goes up whenever diary entries change, here or on another device. */
export function useDiaryVersion(): number {
  const [v, setV] = useState(0);
  useEffect(() => {
    let alive = true;
    const offs: (() => void)[] = [];
    const bump = () => alive && setV((n) => n + 1);
    void backend.listen("diary://changed", bump).then((off) => (alive ? offs.push(off) : off()));
    void backend
      .listen("syncstore://changed", (c) => {
        if (c.namespaces.includes("diary")) bump();
      })
      .then((off) => (alive ? offs.push(off) : off()));
    return () => {
      alive = false;
      offs.forEach((off) => off());
    };
  }, []);
  return v;
}

/** Log an album by hand; the toast offers to rate it and add a note. */
export async function logAlbum(albumId: string, title: string): Promise<DiaryItem | null> {
  try {
    const e = await backend.diaryLogAlbum(albumId);
    useToasts.getState().push(
      { kind: "info", message: `Logged ${title} in your diary`, action: { label: "Add a note", run: () => nav.go({ name: "diary", edit: e.id }) } },
      6000,
    );
    return e;
  } catch (err) {
    toast.error(errorMessage(err));
    return null;
  }
}

/** The album ⋯ menu item, while the diary is kept. */
export function diaryLogItems(on: boolean, album: { id: string; title: string }) {
  return on ? [{ label: "Log this album", onSelect: () => void logAlbum(album.id, album.title) }] : [];
}
