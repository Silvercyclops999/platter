/**
 * LRC text for the lyrics editor: reading saved lyrics back into lines with
 * start times, and writing them out again. One time per line (the editor
 * doesn't do word timing); `[mm:ss.xx]` tags, header tags skipped.
 */
import type { Lyrics } from "./types";

/** One line in the editor. `startMs` null: not synced yet. "" text: a break between verses. */
export interface EditLine {
  text: string;
  startMs: number | null;
}

const TIME = /^\[(\d{1,4}):(\d{1,2})(?:[.:](\d{1,3}))?\]/;
const HEADER = /^\[[a-zA-Z#]+:[^\]]*\]\s*$/;
const WORD_TAG = /<\d{1,4}:\d{1,2}(?:[.:]\d{1,3})?>/g;

function tagMs(m: RegExpMatchArray): number {
  const frac = m[3] ?? "";
  const fracMs = frac ? Math.round(Number(frac) * 10 ** (3 - frac.length)) : 0;
  return Number(m[1]) * 60_000 + Number(m[2]) * 1000 + fracMs;
}

/** `[mm:ss.xx]` for a time in ms (hundredths, rounded). */
export function lrcTag(ms: number): string {
  const cs = Math.max(0, Math.round(ms / 10));
  const min = Math.floor(cs / 6000);
  const sec = Math.floor((cs % 6000) / 100);
  const hund = cs % 100;
  return `[${String(min).padStart(2, "0")}:${String(sec).padStart(2, "0")}.${String(hund).padStart(2, "0")}]`;
}

/** A time for people: `1:02.3`. */
export function clockTenths(ms: number): string {
  const t = Math.max(0, Math.round(ms / 100));
  const min = Math.floor(t / 600);
  const sec = Math.floor((t % 600) / 10);
  return `${min}:${String(sec).padStart(2, "0")}.${t % 10}`;
}

/** Plain text (one lyric per line) to editor lines; outer blank lines dropped, runs of blanks kept as one break. */
export function linesFromText(text: string): EditLine[] {
  const out: EditLine[] = [];
  for (const raw of text.replace(/\r\n?/g, "\n").split("\n")) {
    const t = raw.trim();
    if (!t && (!out.length || out[out.length - 1].text === "")) continue;
    out.push({ text: t, startMs: null });
  }
  while (out.length && out[out.length - 1].text === "") out.pop();
  return out;
}

/** Saved lyrics (LRC or plain) to editor lines. */
export function parseLrc(text: string): EditLine[] {
  const out: EditLine[] = [];
  for (const raw of text.replace(/^﻿/, "").replace(/\r\n?/g, "\n").split("\n")) {
    let line = raw.trim();
    if (HEADER.test(line) && !TIME.test(line)) continue;
    let startMs: number | null = null;
    let m: RegExpMatchArray | null;
    while ((m = line.match(TIME))) {
      startMs ??= tagMs(m);
      line = line.slice(m[0].length).trimStart();
    }
    line = line.replace(WORD_TAG, "").replace(/\s+/g, " ").trim();
    if (!line && startMs === null && (!out.length || out[out.length - 1].text === "")) continue;
    out.push({ text: line, startMs });
  }
  while (out.length && out[out.length - 1].text === "" && out[out.length - 1].startMs === null) out.pop();
  return out;
}

/** The lines as text for the Write step (times dropped). */
export function textFromLines(lines: EditLine[]): string {
  return lines.map((l) => l.text).join("\n");
}

/** Lines that take a time (the ones with words). */
export const stampable = (l: EditLine) => l.text !== "";

/** Synced when every line with words has a time. */
export function isSynced(lines: EditLine[]): boolean {
  const words = lines.filter(stampable);
  return words.length > 0 && words.every((l) => l.startMs !== null);
}

/** How many lines with words still have no time (0 when none has one: plain lyrics). */
export function unsyncedCount(lines: EditLine[]): number {
  const words = lines.filter(stampable);
  if (!words.some((l) => l.startMs !== null)) return 0;
  return words.filter((l) => l.startMs === null).length;
}

/**
 * The lines as saved text: LRC when synced, plain lines otherwise. Breaks
 * between verses stay as blank lines either way.
 */
export function toLrc(lines: EditLine[]): string {
  const synced = isSynced(lines);
  return lines
    .map((l) => (synced && l.text && l.startMs !== null ? `${lrcTag(l.startMs)}${l.text}` : l.text))
    .join("\n")
    .trim();
}

/** Saved text as lyrics for the views (the mock backend's own-lyrics source). */
export function lyricsFromText(text: string): Lyrics | null {
  const lines = parseLrc(text);
  if (!lines.some(stampable)) return null;
  const synced = isSynced(lines);
  const timed = synced
    ? lines.filter((l) => l.startMs !== null).map((l) => ({ startMs: l.startMs!, text: l.text })).sort((a, b) => a.startMs - b.startMs)
    : undefined;
  return { synced, lines: lines.map((l) => l.text), timed, source: "user" };
}
