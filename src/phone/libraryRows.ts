/**
 * How the phone's Library lists rows: Bookmarks and Stats live on the You
 * tab rather than among the playlists; Liked Songs is the one list of liked
 * songs (a heart-named "Tracks" playlist made by another player is the same
 * songs, so it stays off); playlists say how many songs they hold instead
 * of your own name; and A to Z goes by the name as written, ignoring a
 * leading "The", "A" or "An".
 */
import { subtitle, type LibEntry, type LibSort } from "../stores/library";
import { countLine } from "./pages/common";

const collator = new Intl.Collator(undefined, { sensitivity: "base", numeric: true });

/** A heart (or other picture) followed by "Tracks": another player's list of liked songs. */
export function isLikedCopy(e: Pick<LibEntry, "kind" | "title">): boolean {
  if (e.kind !== "playlist") return false;
  const t = e.title.trim();
  return /^\p{Extended_Pictographic}/u.test(t) && t.replace(/[\p{Extended_Pictographic}️‍\s]+/gu, " ").trim().toLowerCase() === "tracks";
}

/** What A to Z sorts on: the name without a leading article. */
export function alphaKey(title: string): string {
  return title.trim().replace(/^(the|a|an)\s+/i, "");
}

/** The first letter a row files under ("#" for numbers and symbols). */
export function letterOf(title: string): string {
  const c = alphaKey(title).normalize("NFD").charAt(0).toUpperCase();
  return /[A-Z]/.test(c) ? c : "#";
}

/** The phone Library's rows, from the shared list model; `counts` gives playlists their song counts. */
export function phoneRows(rows: LibEntry[], sort: LibSort, counts?: ReadonlyMap<string, number>): LibEntry[] {
  const kept = rows
    .filter((e) => !e.folder && !e.bookmarks && !e.stats && !e.page && !isLikedCopy(e))
    .map((e) => (e.kind === "playlist" && !e.trackCount && counts?.has(e.id) ? { ...e, trackCount: counts.get(e.id)! } : e));
  // Liked Songs and pinned rows stay at the top (in pin order) in every
  // sort; with A to Z the rest go in order.
  const top = kept.filter((e) => e.liked || e.pinned);
  const others = kept.filter((e) => !e.liked && !e.pinned);
  if (sort !== "alpha") return [...top, ...others];
  const rest = others.sort((a, b) => collator.compare(alphaKey(a.title), alphaKey(b.title)) || collator.compare(a.title, b.title));
  return [...top, ...rest];
}

/** A row's grey line: a playlist's song count in place of its owner; a downloaded mix says Mix. */
export function phoneSubtitle(e: LibEntry, typed: boolean): string {
  if (e.mix) {
    const count = e.trackCount ? countLine(e.trackCount, "song") : null;
    return typed ? (count ?? "Mix") : ["Mix", count].filter(Boolean).join(" · ");
  }
  if (e.kind === "playlist" && !e.liked && !e.shared && !e.blend && !e.platter && !e.folder) {
    const count = e.trackCount ? countLine(e.trackCount, "song") : null;
    if (typed) return count ?? (e.smart ? "Smart playlist" : "");
    return ["Playlist", count].filter(Boolean).join(" · ");
  }
  return subtitle(e, typed);
}
