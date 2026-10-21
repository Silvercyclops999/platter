/**
 * Back to the playing song: open the list that is playing (its album when
 * the list has no page of its own, or is an artist's), scroll the playing row into view and
 * pulse it once. Ctrl/Cmd+Shift+J and the command palette.
 */
import { usePlayer } from "../stores/player";
import { nav, useRouter } from "../stores/router";
import { toast } from "../stores/toasts";
import { contextRoute } from "./nowPlaying";
import { pageReveal } from "./pageFind";

/** How long to wait for the page's rows to load. */
export const FIND_ROW_MS = 5_000;
const POLL_MS = 100;
/** The pulse class stays this long (the animation is shorter). */
const PULSE_MS = 1_400;

let pending: ReturnType<typeof setInterval> | null = null;

export function goToPlaying(): void {
  const s = usePlayer.getState().state;
  const track = s?.track;
  if (!track) {
    toast.info("Nothing is playing.");
    return;
  }
  // An artist's page shows only the popular songs, so their album it is.
  const ctx = s.queue.context;
  nav.go((ctx?.kind === "artist" ? null : contextRoute(ctx)) ?? { name: "album", id: track.albumId });
  revealRow(track.id);
}

/**
 * Scroll the row for `trackId` in the main view into view and pulse it, once
 * it shows. The page may need to scroll a long list or clear its finder
 * first. Leaving the page stops the search; a row that never shows says so.
 */
export function revealRow(trackId: string): void {
  stop();
  const started = Date.now();
  const router = useRouter.getState();
  const entry = router.entries[router.index];
  const sel = `.main__scroll .track-row[data-track-id="${CSS.escape(trackId)}"]`;
  const tryNow = () => {
    const r = useRouter.getState();
    if (r.entries[r.index] !== entry) return stop();
    const row = document.querySelector<HTMLElement>(sel);
    if (row) {
      stop();
      const still = window.matchMedia?.("(prefers-reduced-motion: reduce)").matches;
      row.scrollIntoView?.({ block: "center", behavior: still ? "auto" : "smooth" });
      row.focus({ preventScroll: true });
      row.classList.remove("track-row--found");
      // Restart the pulse when it's asked for twice in a row.
      void row.offsetWidth;
      row.classList.add("track-row--found");
      setTimeout(() => row.classList.remove("track-row--found"), PULSE_MS);
    } else if (Date.now() - started > FIND_ROW_MS) {
      stop();
      toast.info("The playing song isn't on this page.");
    } else {
      pageReveal(trackId);
    }
  };
  pending = setInterval(tryNow, POLL_MS);
  tryNow();
}

function stop() {
  if (pending) clearInterval(pending);
  pending = null;
}
