/** Recents page decisions: day groups and each row's lines. */
import type { RecentsEntry } from "../lib/types";

const startOfDay = (d: Date) => new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();

/** "Today", "Yesterday", then "Mon, Oct 12" (with the year when it isn't this year). */
export function dayLabel(at: number, now = new Date()): string {
  const day = startOfDay(new Date(at));
  const today = startOfDay(now);
  if (day === today) return "Today";
  // A calendar day back, safe across daylight saving changes.
  const y = new Date(now.getFullYear(), now.getMonth(), now.getDate() - 1).getTime();
  if (day === y) return "Yesterday";
  const d = new Date(at);
  return d.toLocaleDateString("en-US", {
    weekday: "short",
    month: "short",
    day: "numeric",
    ...(d.getFullYear() === now.getFullYear() ? {} : { year: "numeric" }),
  });
}

/** Entries (newest first) in day groups, newest day first. */
export function groupByDay(entries: RecentsEntry[], now = new Date()): { label: string; entries: RecentsEntry[] }[] {
  const out: { label: string; entries: RecentsEntry[] }[] = [];
  for (const e of entries) {
    const label = dayLabel(e.at, now);
    const last = out[out.length - 1];
    if (last?.label === label) last.entries.push(e);
    else out.push({ label, entries: [e] });
  }
  return out;
}

const songs = (n: number, verb: string) => `${n} ${n === 1 ? "song" : "songs"} ${verb}`;

/** A row's title (null for loose songs) and its grey line. */
export function entryLines(e: RecentsEntry): { title: string | null; sub: string } {
  const played = songs(e.tracks.length, "played");
  switch (e.kind) {
    case "liked":
      return { title: "Liked Songs", sub: `Playlist · ${played}` };
    case "likedAdded":
      return { title: "Liked Songs", sub: songs(e.tracks.length, "added") };
    case "songs":
      return { title: null, sub: played };
    case "context": {
      const i = e.item;
      if (i?.kind === "album") return { title: i.album.title, sub: `Album · ${i.album.artistTitle} · ${played}` };
      if (i?.kind === "artist") return { title: i.artist.title, sub: `Artist · ${played}` };
      if (i?.kind === "playlist") return { title: i.playlist.title, sub: `${played} · Playlist` };
      return { title: null, sub: played };
    }
  }
}

/** Stable key for a row (the same album can show twice in a day). */
export function entryKey(e: RecentsEntry): string {
  const i = e.item;
  const id = i ? `${i.kind}:${i.kind === "album" ? i.album.id : i.kind === "artist" ? i.artist.id : i.playlist.id}` : e.kind;
  return `${id}@${e.at}`;
}
