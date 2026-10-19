/**
 * Now playing's cover as a row of three: the previous song's cover, this
 * one and the next one, side by side at the same size. A sideways drag
 * slides the row under the finger. Letting go past the distance carries it
 * on until the neighbour sits where the cover was, and that song plays;
 * short of it the row springs back. The covers never change size on the
 * way: the small settle a pause gives the cover waits until the row is
 * still, and a pause that only flickers between songs is ignored.
 *
 * A pull down moves the whole screen instead and closes it past its
 * distance. Nothing here redraws while a finger moves: the moves are
 * transforms written straight to the row or the screen.
 */
import { memo, useEffect, useLayoutEffect, useRef, useState, type RefObject } from "react";
import { Art } from "../components/Art";
import type { PlayerState, Track } from "../lib/types";
import { nextInQueue } from "../shell/nowPlaying";
import { player, usePlayer } from "../stores/player";
import { tap } from "./bits";
import { useArtGestures, type Axis, type Outcome } from "./gestures";
import { EASE, reduced, release as dropAnimations, run } from "./motion";

/** Now playing's cover is asked for at this CSS size (the phone's ladder caps it at 1200 px). */
export const NP_ART_CSS = 400;
/** Previous restarts the song from this far in, as the core does; the cover beside it is then this song's. */
export const PREVIOUS_RESTART_MS = 3000;
/** A pause has to last this long before the cover settles back (a pause between two songs doesn't). */
export const PAUSE_SETTLE_MS = 300;
/** A short drag springs back in this long. */
export const SNAP_MS = 220;
/** The longest the row takes to carry on to the neighbour. */
export const SKIP_SETTLE_MAX_MS = 300;
/** The row lets go of the old song after this long, even if the player never answered. */
export const SKIP_HOLD_MS = 1500;

type Cover = Pick<Track, "id" | "thumb" | "albumTitle" | "artistTitle"> | null;
interface Deck {
  prev: Cover;
  cur: Cover;
  next: Cover;
}

const isPlaying = (s: PlayerState | null) => s?.status === "playing" || s?.status === "loading" || s?.status === "buffering";

/** The cover to the left: this song's own when Previous would restart it, else the last other song played. */
export function previousCover(s: PlayerState | null | undefined, positionMs: number): Track | null {
  const cur = s?.track;
  if (!cur) return null;
  if (positionMs >= PREVIOUS_RESTART_MS) return cur;
  const h = s.queue.history;
  for (let i = h.length - 1; i >= 0; i--) if (h[i].track.id !== cur.id) return h[i].track;
  return cur;
}

/** How long the row takes to cover `left` px at the speed it was let go (px per ms). */
export function settleMs(left: number, speed: number): number {
  return Math.round(Math.min(SKIP_SETTLE_MAX_MS, Math.max(160, Math.abs(left) / Math.max(Math.abs(speed), 0.9))));
}

export const CoverDeck = memo(function CoverDeck({ screen, onClose }: { screen: RefObject<HTMLElement | null>; onClose: () => void }) {
  const cur = usePlayer((s) => s.state?.track ?? null);
  const next = usePlayer((s) => nextInQueue(s.state?.queue)?.track ?? null);
  const prev = usePlayer((s) => previousCover(s.state, s.positionMs));
  const playing = usePlayer((s) => isPlaying(s.state));
  // While the row carries on to a neighbour, it keeps showing the covers it set out with.
  const [held, setHeld] = useState<Deck | null>(null);
  const deck: Deck = held ?? { prev, cur, next };
  const art = useRef<HTMLDivElement>(null);
  const strip = useRef<HTMLDivElement>(null);
  const busy = useRef(false);
  const settle = useRef<{ cancel: () => void } | null>(null);
  const closeRef = useRef(onClose);
  closeRef.current = onClose;

  // A cover that is paused sits a little smaller, but only once the row is still and the pause is real.
  const [shrunk, setShrunk] = useState(!playing);
  useEffect(() => {
    if (held) return;
    if (playing) return setShrunk(false);
    const t = setTimeout(() => !busy.current && setShrunk(true), PAUSE_SETTLE_MS);
    return () => clearTimeout(t);
  }, [playing, held]);

  // The row has carried on and the player has answered: the neighbour is the cover now, in one frame.
  useLayoutEffect(() => {
    if (held) return;
    const el = strip.current;
    if (!el) return;
    dropAnimations(el);
    el.style.transform = "";
  }, [held]);

  // Let go of the old song once the player says what plays now (or gives up waiting).
  useEffect(() => {
    if (!held) return;
    let unsub = () => {};
    const t = setTimeout(() => setHeld(null), SKIP_HOLD_MS);
    unsub = usePlayer.subscribe((s, p) => {
      if (s.state !== p.state) setHeld(null);
    });
    return () => {
      clearTimeout(t);
      unsub();
    };
  }, [held]);

  const handlers = useArtGestures({
    size: () => ({ width: art.current?.clientWidth || 360, height: screen.current?.clientHeight || 800 }),
    start: () => {
      settle.current?.cancel();
      busy.current = true;
    },
    place: (at: { x: number; y: number }, axis: Axis) => {
      if (axis === "x") {
        const el = strip.current;
        if (!el) return;
        el.style.willChange = "transform";
        el.style.transform = `translateX(${at.x}px)`;
      } else {
        const sec = screen.current;
        if (!sec) return;
        sec.classList.add("is-dragging");
        sec.style.transform = `translateY(${at.y}px)`;
      }
    },
    done: (outcome: Outcome, g) => {
      busy.current = false;
      const el = strip.current;
      const sec = screen.current;
      if (g.axis === "y") {
        if (!sec) return;
        if (outcome === "close") {
          // The close carries on from where the finger left the screen.
          sec.classList.remove("is-dragging");
          closeRef.current();
          return;
        }
        const from = sec.style.transform;
        sec.classList.remove("is-dragging");
        sec.style.transform = "";
        if (from && !reduced()) {
          const a = run(sec, [{ transform: from }, { transform: "none" }], SNAP_MS);
          settle.current = a;
          void a.done.then(() => dropAnimations(sec));
        }
        return;
      }
      if (!el) return;
      const skip = outcome === "next" || outcome === "previous";
      const neighbour = outcome === "next" ? deck.next : outcome === "previous" ? deck.prev : null;
      if (skip) tap();
      const go = () => void (outcome === "next" ? player.next() : player.previous());
      // Short of the distance, or nothing beside it to carry on to: back to the middle (and the skip still happens).
      if (!skip || !neighbour || reduced()) {
        el.style.transform = "";
        el.style.willChange = "";
        if (g.at.x && !reduced()) {
          const a = run(el, [{ transform: `translateX(${g.at.x}px)` }, { transform: "none" }], SNAP_MS);
          settle.current = a;
          void a.done.then(() => dropAnimations(el));
        }
        if (skip) go();
        return;
      }
      // Carry on until the neighbour sits where the cover was, then play it.
      const step = (el.firstElementChild as HTMLElement | null)?.offsetWidth || el.offsetWidth || art.current?.clientWidth || 360;
      const gap = parseFloat(getComputedStyle(el).getPropertyValue("--np-deck-gap")) || 18;
      const to = (outcome === "next" ? -1 : 1) * (step + gap);
      setHeld(deck);
      el.style.transform = `translateX(${to}px)`;
      const a = run(el, [{ transform: `translateX(${g.at.x}px)` }, { transform: `translateX(${to}px)` }], settleMs(to - g.at.x, g.vx), { easing: EASE });
      settle.current = a;
      void a.done.then(go);
    },
  });

  return (
    <div ref={art} className="ph-np__art" data-testid="np-art" {...handlers}>
      <div ref={strip} className="ph-np__deck-row" data-testid="np-deck">
        <Slide cover={deck.prev} side="prev" shrunk={shrunk} />
        <Slide cover={deck.cur} side="cur" shrunk={shrunk} />
        <Slide cover={deck.next} side="next" shrunk={shrunk} />
      </div>
    </div>
  );
});

function Slide({ cover, side, shrunk }: { cover: Cover; side: "prev" | "cur" | "next"; shrunk: boolean }) {
  return (
    <div className={`ph-np__slide ph-np__slide--${side}`} aria-hidden={side !== "cur" || undefined}>
      {cover && (
        <div className={`ph-np__cover ${shrunk ? "" : "is-playing"}`}>
          <Art path={cover.thumb} fluid requestSize={NP_ART_CSS} sleeve={{ title: cover.albumTitle, artist: cover.artistTitle }} />
        </div>
      )}
    </div>
  );
}
