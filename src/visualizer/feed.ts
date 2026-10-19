/**
 * One feed of visualizer frames for every view that shows one (the expanded
 * view, the panel strip, the Deck's meters, the phone). While at least one
 * is mounted and the page can be seen, the core is asked to watch, and the
 * watch is renewed every second; the page hidden or the last view gone ends
 * it straight away. Frames go into a plain variable the views read from
 * their own animation frames, so a frame never re-renders React.
 */
import { useEffect } from "react";
import { create } from "zustand";
import { backend, type Unlisten } from "../lib/backend";
import type { SpectrumFrame } from "../lib/types";
import { useSettings } from "../stores/settings";
import { RENEW_MS } from "./scale";

export interface Want {
  fps: number;
  scope: boolean;
}

/** What the views say about the feed. */
interface FeedState {
  /** The core is watching (false while the page is hidden, or resting on battery). */
  running: boolean;
  /** A renewal came back refused while the page was on screen: resting on battery. */
  resting: boolean;
}

export const useFeed = create<FeedState>()(() => ({ running: false, resting: false }));

let latest: SpectrumFrame | null = null;
let latestAt = 0;
const wants = new Map<number, Want>();
let nextId = 1;
let unlisten: Unlisten | null = null;
/** Which listen attempt is current: a stop, then a start, makes a new one. */
let listenToken = 0;
let listening = false;
/** A watch was asked for since the last end, so an end must be sent. */
let asked = false;
let timer: ReturnType<typeof setInterval> | null = null;
let last: string | null = null;

/** The latest frame and when it came (performance.now()). */
export function latestFrame(): { frame: SpectrumFrame | null; at: number } {
  return { frame: latest, at: latestAt };
}

/** Tests: drop everything. */
export function resetFeed() {
  wants.clear();
  stop();
  latest = null;
  latestAt = 0;
  last = null;
  useFeed.setState({ running: false, resting: false });
}

function combined(): Want | null {
  if (!wants.size) return null;
  let fps = 1;
  let scope = false;
  for (const w of wants.values()) {
    fps = Math.max(fps, w.fps);
    scope ||= w.scope;
  }
  return { fps, scope };
}

const hidden = () => typeof document !== "undefined" && (document.visibilityState === "hidden" || document.hidden === true);
const batteryPause = () => useSettings.getState().settings?.visualizerBatteryPause !== false;

async function renew() {
  const want = combined();
  if (!want || hidden()) {
    pause();
    return;
  }
  try {
    asked = true;
    const running = await backend.visualizerWatch(true, want.fps, want.scope, batteryPause());
    // Gone or hidden while the call was out: end it now.
    if (!combined() || hidden()) {
      pause();
      return;
    }
    useFeed.setState({ running, resting: !running });
  } catch {
    useFeed.setState({ running: false, resting: false });
  }
}

function startTimer() {
  if (timer) return;
  timer = setInterval(() => void renew(), RENEW_MS);
}

function pause() {
  if (timer) clearInterval(timer);
  timer = null;
  // Sent whenever one was asked for, even if the view went before the
  // first answer came.
  if (asked) void backend.visualizerWatch(false, 0, false, batteryPause()).catch(() => {});
  asked = false;
  useFeed.setState({ running: false });
}

function stop() {
  pause();
  last = null;
  const un = unlisten;
  unlisten = null;
  listening = false;
  listenToken++;
  un?.();
}

function onVisibility() {
  if (hidden()) pause();
  else if (wants.size) {
    void renew();
    startTimer();
  }
}

function ensureListening() {
  if (listening) return;
  listening = true;
  // Each attempt has its own token: one that resolves after a stop (or
  // after a stop and a new start) drops its own listener.
  const token = ++listenToken;
  void backend
    .listen("player://spectrum", (f) => {
      latest = f;
      latestAt = performance.now();
    })
    .then((un) => {
      if (token === listenToken) unlisten = un;
      else un();
    });
  document.addEventListener("visibilitychange", onVisibility);
}

function changed() {
  const want = combined();
  if (!want) {
    document.removeEventListener("visibilitychange", onVisibility);
    stop();
    return;
  }
  ensureListening();
  const key = JSON.stringify(want);
  if (key !== last) {
    last = key;
    void renew();
  }
  if (!hidden()) startTimer();
}

/** Adds a view's wish; returns how to take it back. */
export function watchFeed(want: Want): () => void {
  const id = nextId++;
  wants.set(id, want);
  changed();
  return () => {
    wants.delete(id);
    changed();
  };
}

/** Keeps the feed going while the calling view is mounted and `want` is set. */
export function useSpectrumFeed(want: Want | null): void {
  const fps = want?.fps ?? 0;
  const scope = want?.scope ?? false;
  const on = !!want;
  useEffect(() => {
    if (!on) return;
    return watchFeed({ fps, scope });
  }, [on, fps, scope]);
}
