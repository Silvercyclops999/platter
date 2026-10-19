/**
 * Editions of one record shown once on the phone's shelves: albums that
 * share a group tag, or whose titles differ only by an edition note
 * ("Deluxe Edition", "Remastered 2011", "Expanded"), fold into the main
 * edition with a count of the rest.
 */
import type { TagIndex } from "../lib/types";

/** Words that mark a title's bracket or dash suffix as an edition note, not part of the name. */
const EDITION_WORDS =
  /\b(deluxe|edition|remaster(ed)?|expanded|anniversary|bonus|special|collector'?s|super|reissue|version|explicit|clean|mono|stereo|live edition|tour|platinum|complete|definitive|legacy|redux|\d{4} mix)\b/i;

/** A title without its edition notes, folded for comparing. */
export function baseTitle(title: string): string {
  let t = title.trim();
  for (;;) {
    const m = /\s*(?:[([]([^()[\]]*)[)\]]|[-–:]\s+([^-–:]+))$/.exec(t);
    const note = m && (m[1] ?? m[2]);
    if (!m || !note || !EDITION_WORDS.test(note)) break;
    t = t.slice(0, m.index).trim();
  }
  return t.toLowerCase().replace(/\s+/g, " ");
}

export interface EditionGroup<T> {
  main: T;
  /** Every edition, the main one first. */
  all: T[];
}

/**
 * Albums in their given order, one per record: editions fold into the first
 * that has no edition note (else the most played, else the first seen).
 */
export function groupEditions<T extends { id: string; title: string; viewCount?: number }>(albums: T[], index?: TagIndex | null): EditionGroup<T>[] {
  const groups = new Map<string, T[]>();
  const order: string[] = [];
  for (const a of albums) {
    const tag = index?.albums[a.id]?.group;
    const key = tag ? `g:${tag}` : `t:${baseTitle(a.title)}`;
    const g = groups.get(key);
    if (g) g.push(a);
    else {
      groups.set(key, [a]);
      order.push(key);
    }
  }
  return order.map((k) => {
    const all = groups.get(k)!;
    const plain = all.find((a) => baseTitle(a.title) === a.title.trim().toLowerCase().replace(/\s+/g, " "));
    const main = plain ?? [...all].sort((a, b) => (b.viewCount ?? 0) - (a.viewCount ?? 0))[0];
    return { main, all: [main, ...all.filter((a) => a !== main)] };
  });
}

/** "3 editions" under a folded record. */
export const editionsNote = (n: number) => `${n} editions`;
