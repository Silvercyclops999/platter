/**
 * The core's lyric search (lyricsearch.rs) for the mock: three or more
 * words, or words in quotes, found in order in stored lyrics; the line that
 * matched comes back with the song.
 */
const fold = (s: string) =>
  s
    .normalize("NFD")
    .replace(/\p{M}/gu, "")
    .toLowerCase();
const words = (s: string) => fold(s).split(/[^\p{L}\p{N}]+/u).filter(Boolean);
/** Timestamps and tags off an LRC line. */
const plain = (line: string) => line.replace(/\[[^\]]*\]/g, "").trim();

export function lyricQuery(raw: string): { words: string[]; prefix: boolean } | null {
  const quotes = (raw.match(/"/g) ?? []).length;
  const quoted = quotes > 0 ? raw.split('"')[1] : undefined;
  const useQuoted = quoted !== undefined && quoted.trim() !== "";
  const w = words(useQuoted ? quoted : raw).slice(0, 12);
  if (!w.length || (!useQuoted && w.length < 3)) return null;
  return { words: w, prefix: !(useQuoted && quotes >= 2) && !/\s$/.test(raw) };
}

export function mockLyricSearch(raw: string, stored: (readonly [string, string])[]): { trackId: string; line: string }[] {
  const q = lyricQuery(raw);
  if (!q) return [];
  const out: { trackId: string; line: string }[] = [];
  for (const [trackId, text] of stored) {
    const lines = text.split("\n").map(plain).filter(Boolean);
    const all = lines.flatMap((l, i) => words(l).map((w) => [i, w] as const));
    const n = q.words.length;
    for (let s = 0; s + n <= all.length; s++) {
      const hit = q.words.every((w, k) => (k === n - 1 && q.prefix ? all[s + k][1].startsWith(w) : all[s + k][1] === w));
      if (hit) {
        out.push({ trackId, line: lines.slice(all[s][0], all[s + n - 1][0] + 1).join(" / ") });
        break;
      }
    }
  }
  return out;
}
