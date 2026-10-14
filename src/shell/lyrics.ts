/**
 * Lyrics data and timing rules for the lyrics view and the full-window view,
 * kept apart from the components so they can be tested on their own: the
 * fetch (with a small cache), which line and word are current, the
 * auto-follow pause, and auto-show.
 */
import { useEffect, useState } from "react";
import { backend } from "../lib/backend";
import type { LyricLine, LyricWord, Lyrics } from "../lib/types";
import { usePlayer } from "../stores/player";
import { nav, useRouter } from "../stores/router";
import { useSettings } from "../stores/settings";
import { useUi } from "../stores/ui";

// ---------------------------------------------------------------- fetch

/** Lyrics for the last few tracks (null: none), so skipping back doesn't refetch. */
const CACHE_SIZE = 30;
const cache = new Map<string, Lyrics | null>();
const pending = new Map<string, Promise<Lyrics | null>>();

export function clearLyricsCache() {
  cache.clear();
  pending.clear();
}

/** The track's lyrics, from the cache or the core. A failed fetch is "none" (and not cached). */
export function loadLyrics(trackId: string): Promise<Lyrics | null> {
  if (cache.has(trackId)) return Promise.resolve(cache.get(trackId) ?? null);
  let p = pending.get(trackId);
  if (!p) {
    p = backend.lyrics(trackId).then(
      (l) => {
        const v = l && (l.lines.length || l.timed?.length) ? l : null;
        cache.set(trackId, v);
        if (cache.size > CACHE_SIZE) cache.delete(cache.keys().next().value!);
        pending.delete(trackId);
        return v;
      },
      () => {
        pending.delete(trackId);
        return null;
      },
    );
    pending.set(trackId, p);
  }
  return p;
}

/** Lyrics for a track; `loading` until known. Turning LRCLIB on or off asks again. */
export function useLyrics(trackId: string | null | undefined): { lyrics: Lyrics | null; loading: boolean } {
  const lrclib = useSettings((s) => s.settings?.lyricsLrclib ?? false);
  const [state, setState] = useState<{ key: string | null; lyrics: Lyrics | null; loading: boolean }>(() => ({
    key: null,
    lyrics: null,
    loading: !!trackId,
  }));
  const key = trackId ? `${trackId}|${lrclib}` : null;
  if (key !== state.key) {
    const known = trackId ? cache.has(trackId) : true;
    setState({ key, lyrics: trackId ? (cache.get(trackId) ?? null) : null, loading: !known });
  }
  useEffect(() => {
    if (!trackId) return;
    let alive = true;
    loadLyrics(trackId).then((lyrics) => alive && setState({ key: `${trackId}|${lrclib}`, lyrics, loading: false }));
    return () => {
      alive = false;
    };
  }, [trackId, lrclib]);
  return { lyrics: state.lyrics, loading: state.loading };
}

// Turning LRCLIB on finds lyrics Plex didn't have; off drops them.
let lastLrclib: boolean | undefined;
useSettings.subscribe((s) => {
  const v = s.settings?.lyricsLrclib;
  if (v === undefined) return;
  if (lastLrclib !== undefined && v !== lastLrclib) clearLyricsCache();
  lastLrclib = v;
});

// ---------------------------------------------------------------- timing

/** The timed lines to show, or null when the lyrics aren't synced. */
export function timedLines(l: Lyrics | null | undefined): LyricLine[] | null {
  return l?.synced && l.timed?.length ? l.timed : null;
}

/** Index of the line playing at `ms` (the last one started), -1 before the first. */
export function currentLineIndex(lines: { startMs: number }[], ms: number): number {
  let lo = 0;
  let hi = lines.length - 1;
  let found = -1;
  while (lo <= hi) {
    const mid = (lo + hi) >> 1;
    if (lines[mid].startMs <= ms) {
      found = mid;
      lo = mid + 1;
    } else {
      hi = mid - 1;
    }
  }
  return found;
}

/** Index of the word playing at `ms` in a line, -1 before its first word. */
export function currentWordIndex(words: LyricWord[] | undefined, ms: number): number {
  return words?.length ? currentLineIndex(words, ms) : -1;
}

/** Where a line stands against the current one. */
export function lineState(i: number, current: number): "past" | "current" | "future" {
  return i < current ? "past" : i === current ? "current" : "future";
}

/** Where the current line sits in the view: about a third from the top. */
export const FOLLOW_ANCHOR = 1 / 3;

/** Scroll offset that puts a line at the anchor, clamped to the scroll range. */
export function followTarget(lineTop: number, lineHeight: number, viewHeight: number, scrollHeight: number): number {
  const top = lineTop + lineHeight / 2 - viewHeight * FOLLOW_ANCHOR;
  return Math.max(0, Math.min(top, Math.max(0, scrollHeight - viewHeight)));
}

/** Scrolling by hand pauses auto-follow for this long after the last scroll. */
export const FOLLOW_RESUME_MS = 5_000;

/**
 * Playback position between the core's updates (4 a second): the last
 * update plus the time since, while playing. Word timing needs better than
 * 250 ms.
 */
export function interpolate(base: { positionMs: number; at: number }, now: number, playing: boolean, durationMs: number): number {
  const ms = playing ? base.positionMs + Math.max(0, now - base.at) : base.positionMs;
  return durationMs > 0 ? Math.min(ms, durationMs) : ms;
}

// ---------------------------------------------------------------- auto-show

const isPlaying = (s: string | undefined) => s === "playing" || s === "loading" || s === "buffering";

/**
 * "Lyrics: Auto-show when available": a new song with synced lyrics that
 * starts while the now-playing view is open opens the lyrics view. The
 * session restored at launch doesn't count, and Off never opens it.
 */
export function useLyricsAutoShow() {
  useEffect(
    () =>
      usePlayer.subscribe((s, prev) => {
        const id = s.state?.track?.id;
        if (!id || !prev.state || id === prev.state.track?.id || !isPlaying(s.state?.status)) return;
        if (useSettings.getState().settings?.lyricsMode !== "autoShow") return;
        if (!useUi.getState().nowPlayingOpen) return;
        void loadLyrics(id).then((l) => {
          // Still the same song, the view still open, and not there already.
          if (!timedLines(l) || usePlayer.getState().state?.track?.id !== id || !useUi.getState().nowPlayingOpen) return;
          const r = useRouter.getState();
          if (r.entries[r.index].route.name === "lyrics") return;
          useUi.getState().setNowPlayingExpanded(false);
          nav.go({ name: "lyrics" });
        });
      }),
    [],
  );
}

/** The lyrics button: open the lyrics view, or go back from it. */
export function toggleLyricsView() {
  const r = useRouter.getState();
  if (r.entries[r.index].route.name === "lyrics") {
    if (r.index > 0) r.back();
    else r.replace({ name: "home" });
  } else {
    useUi.getState().setNowPlayingExpanded(false);
    r.navigate({ name: "lyrics" });
  }
}
