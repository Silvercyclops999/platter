/**
 * Lyrics data and timing rules for the expanded view's lyrics mode,
 * kept apart from the components so they can be tested on their own: the
 * fetch (with a small cache), which line and word are current, the
 * auto-follow pause, and auto-show.
 */
import { useEffect, useState } from "react";
import { create } from "zustand";
import { backend } from "../lib/backend";
import type { LyricLine, LyricWord, Lyrics, LyricsCheckStatus } from "../lib/types";
import { useLyricsEditor } from "../stores/lyricsEditor";
import { usePlayer } from "../stores/player";
import { fullWindow, useFullWindow } from "../stores/fullWindow";
import { useSettings } from "../stores/settings";
import { useUi } from "../stores/ui";

// ---------------------------------------------------------------- fetch

/** Lyrics for the last few tracks (null: none), so skipping back doesn't refetch. */
const CACHE_SIZE = 30;
const cache = new Map<string, Lyrics | null>();
const pending = new Map<string, Promise<Lyrics | null>>();
/** Moves on each clear, so a fetch from before it can't put stale lyrics back. */
let generation = 0;

export function clearLyricsCache() {
  generation++;
  cache.clear();
  pending.clear();
}

/** Lyrics were looked up for the song this session (shown, held back or checked): its menu offers "Wrong lyrics?". */
export function lyricsSeen(trackId: string): boolean {
  return !!cache.get(trackId);
}

/** Lyrics with words to show (not just a notice saying why there are none). */
export function hasWords(l: Lyrics | null | undefined): boolean {
  return !!l && !l.notice && !!(l.lines.length || l.timed?.length);
}

/** The track's lyrics, from the cache or the core: null when there are none to show. */
export function loadLyrics(trackId: string): Promise<Lyrics | null> {
  return loadLyricsInfo(trackId).then((l) => (hasWords(l) ? l : null));
}

/**
 * The core's whole answer, notices included ("Lyrics from the released
 * version", "Checking…"). A failed fetch is "none" (and not cached).
 */
export function loadLyricsInfo(trackId: string): Promise<Lyrics | null> {
  if (cache.has(trackId)) return Promise.resolve(cache.get(trackId) ?? null);
  let p = pending.get(trackId);
  if (!p) {
    const gen = generation;
    const settle = () => {
      if (gen === generation && pending.get(trackId) === p) pending.delete(trackId);
    };
    p = backend.lyrics(trackId).then(
      (l) => {
        const v = l && (hasWords(l) || l.notice) ? l : null;
        // Cleared or asked again while asking (a setting, an edit, a check
        // or a nudge changed): don't cache the old answer over the new one.
        if (gen === generation && pending.get(trackId) === p) {
          cache.set(trackId, v);
          if (cache.size > CACHE_SIZE) cache.delete(cache.keys().next().value!);
        }
        settle();
        return v;
      },
      () => {
        settle();
        return null;
      },
    );
    pending.set(trackId, p);
  }
  return p;
}

/**
 * Lyrics for a track; `loading` until known. Turning LRCLIB on or off,
 * preferring your own lyrics, borrowed timing on live versions, and saving
 * lyrics in the editor ask again.
 */
export function useLyrics(trackId: string | null | undefined): {
  lyrics: Lyrics | null;
  loading: boolean;
  /** The whole answer: with a notice when nothing is shown and why. */
  info: Lyrics | null;
} {
  const lrclib = useSettings((s) => s.settings?.lyricsLrclib ?? false);
  const preferOwn = useSettings((s) => s.settings?.lyricsPreferOwn ?? false);
  const showAuto = useSettings((s) => s.settings?.showAutoLyrics ?? true);
  const liveTiming = useSettings((s) => s.settings?.lyricsLiveTiming ?? false);
  const version = useLyricsEditor((s) => s.version);
  const trust = useLyricsTrust((s) => s.version);
  const sources = `${lrclib}|${preferOwn}|${showAuto}|${liveTiming}|${version}|${trust}`;
  const [state, setState] = useState<{ key: string | null; trackId: string | null; lyrics: Lyrics | null; loading: boolean }>(
    () => ({ key: null, trackId: null, lyrics: null, loading: !!trackId }),
  );
  const key = trackId ? `${trackId}|${sources}` : null;
  if (key !== state.key) {
    const id = trackId ?? null;
    if (id && cache.has(id)) setState({ key, trackId: id, lyrics: cache.get(id) ?? null, loading: false });
    // The same song asked again (a check landed, a nudge, a pick): keep
    // showing what's there until the new answer arrives, so the view never
    // blanks or remounts. Only a different song shows the skeleton.
    else if (id && id === state.trackId && !state.loading) setState({ ...state, key });
    else setState({ key, trackId: id, lyrics: null, loading: !!id });
  }
  useEffect(() => {
    if (!trackId) return;
    let alive = true;
    loadLyricsInfo(trackId).then((lyrics) => alive && setState({ key: `${trackId}|${sources}`, trackId, lyrics, loading: false }));
    return () => {
      alive = false;
    };
  }, [trackId, sources]);
  return { lyrics: hasWords(state.lyrics) ? state.lyrics : null, loading: state.loading, info: state.lyrics };
}

/** A notice the lyrics view can act on (not "you chose none"): the lyrics button stays on. */
export function actionableNotice(l: Lyrics | null | undefined): boolean {
  return !!l?.notice && l.notice !== "chosenNone";
}

/**
 * Lyrics checked against the song, a pick or an offset changed: `version`
 * moves so the views ask again.
 */
export const useLyricsTrust = create<{ version: number; checks: Record<string, LyricsCheckStatus> }>()(() => ({ version: 0, checks: {} }));

/** A song's check moved on. */
export function setCheckStatus(s: LyricsCheckStatus) {
  useLyricsTrust.setState((st) => ({ checks: { ...st.checks, [s.trackId]: s } }));
}

/** Ask the core where a song's check is (the view opened mid-check). */
export async function loadCheckStatus(trackId: string): Promise<void> {
  try {
    const s = await backend.lyricsCheckStatus(trackId);
    if (s && !useLyricsTrust.getState().checks[trackId]) setCheckStatus(s);
  } catch {
    // Nothing known: the plain checking note shows.
  }
}

/** "Try again" after a check couldn't run: forget its status and ask for the lyrics again (which asks for a check). */
export function retryCheck(trackId: string) {
  useLyricsTrust.setState((st) => {
    const checks = { ...st.checks };
    delete checks[trackId];
    return { checks };
  });
  lyricsChanged(trackId);
}

/** Forget a song's lyrics (or every song's) and ask again. */
export function lyricsChanged(trackId?: string) {
  if (trackId) {
    cache.delete(trackId);
    pending.delete(trackId);
  } else {
    clearLyricsCache();
  }
  useLyricsTrust.setState((s) => ({ version: s.version + 1 }));
}

let trustStarted = false;

/** Follow the core's lyrics events (once, at startup). */
export async function startLyricsTrust(): Promise<void> {
  if (trustStarted) return;
  trustStarted = true;
  await backend.listen("lyrics://checked", ({ trackId }) => lyricsChanged(trackId));
  await backend.listen("lyrics://check-status", (s) => setCheckStatus(s));
  await backend.listen("lyrics://choices", () => lyricsChanged());
}

// Checking against the song, fixing its timing and released lyrics for other
// versions change what shows.
useSettings.subscribe((s, p) => {
  const k = (x: typeof s) => `${x.settings?.lyricsCheckAudio}|${x.settings?.lyricsFixTiming}|${x.settings?.lyricsReleasedForVersions}`;
  if (s.settings && p.settings && k(s) !== k(p)) lyricsChanged();
});

// Turning LRCLIB on finds lyrics Plex didn't have; off drops them. Preferring
// your own lyrics changes which source wins.
let lastSources: string | undefined;
useSettings.subscribe((s) => {
  if (!s.settings) return;
  const v = `${s.settings.lyricsLrclib}|${s.settings.lyricsPreferOwn}|${s.settings.showAutoLyrics}|${s.settings.lyricsLiveTiming}`;
  if (lastSources !== undefined && v !== lastSources) clearLyricsCache();
  lastSources = v;
});
// Saving or removing lyrics in the editor.
useLyricsEditor.subscribe((s, p) => s.version !== p.version && clearLyricsCache());

/**
 * Where the lyrics came from, shown under them. None when the credit line
 * already says (LRCLIB, licensed lyrics).
 */
export function lyricsSourceLabel(l: Lyrics): string | null {
  switch (l.source) {
    case "user":
      return "Your lyrics";
    case "auto":
      return "Auto-transcribed, may be wrong.";
    case "file":
      return "From the lyrics file next to this song";
    case "lrclib":
      return null;
    default:
      return l.credit ? null : "Lyrics from Plex";
  }
}

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

/** Where the current line sits in the view: a little above the middle. */
export const FOLLOW_ANCHOR = 0.42;

/** Scroll offset that puts a line at the anchor, clamped to the scroll range. */
export function followTarget(lineTop: number, lineHeight: number, viewHeight: number, scrollHeight: number): number {
  const top = lineTop + lineHeight / 2 - viewHeight * FOLLOW_ANCHOR;
  return Math.max(0, Math.min(top, Math.max(0, scrollHeight - viewHeight)));
}

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
 * starts while the now-playing panel is open opens the expanded view in
 * lyrics mode. An expanded view that's already open keeps the mode the user
 * picked. The session restored at launch doesn't count, and Off never opens it.
 */
export function useLyricsAutoShow() {
  useEffect(
    () =>
      usePlayer.subscribe((s, prev) => {
        const id = s.state?.track?.id;
        if (!id || !prev.state || id === prev.state.track?.id || !isPlaying(s.state?.status)) return;
        if (useSettings.getState().settings?.lyricsMode !== "autoShow") return;
        if (!useUi.getState().nowPlayingOpen || useFullWindow.getState().open) return;
        void loadLyrics(id).then((l) => {
          // Still the same song, the panel still open, and the view not opened meanwhile.
          if (!timedLines(l) || usePlayer.getState().state?.track?.id !== id || !useUi.getState().nowPlayingOpen) return;
          if (useFullWindow.getState().open) return;
          fullWindow.open("lyrics");
        });
      }),
    [],
  );
}
