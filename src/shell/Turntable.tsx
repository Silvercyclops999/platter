import { useEffect, useRef } from "react";
import { Art } from "../components/Art";
import { prefersReducedMotion } from "../design/motion";
import "./turntable.css";

/** A record's speed at 33 1/3 rpm, in degrees a second. */
export const SPIN_DEG_PER_S = (100 / 3 / 60) * 360;
/** About how long the platter takes to come up to speed or stop, in ms. */
export const SPIN_EASE_MS = 1000;

/**
 * The record's speed one frame later: it eases towards the target like a
 * motor (most of the way there in about SPIN_EASE_MS), and snaps once it is
 * close so it settles instead of creeping.
 */
export function nextSpeed(speed: number, target: number, dtMs: number): number {
  const k = 1 - Math.exp(-dtMs / (SPIN_EASE_MS / 4));
  const next = speed + (target - speed) * k;
  return Math.abs(target - next) < 0.5 ? target : next;
}

/**
 * The cover as the label of a record on a platter, with a tonearm. The
 * record spins up when music plays and slows to a stop on pause; the arm
 * swings onto the record and lifts off it. Under reduced motion the record
 * holds still and the arm fades between its two places.
 */
export function Turntable({ thumb, playing, requestSize = 640 }: { thumb: string | null | undefined; playing: boolean; requestSize?: number }) {
  const record = useRef<HTMLDivElement>(null);
  const target = useRef(0);
  target.current = playing ? SPIN_DEG_PER_S : 0;

  useEffect(() => {
    const el = record.current;
    if (!el) return;
    let angle = 0;
    let speed = 0;
    let last = 0;
    let frame = 0;
    const step = (t: number) => {
      const dt = last ? Math.min(100, t - last) : 16;
      last = t;
      if (prefersReducedMotion()) speed = 0;
      else speed = nextSpeed(speed, target.current, dt);
      angle = (angle + (speed * dt) / 1000) % 360;
      el.style.transform = `rotate(${angle.toFixed(2)}deg)`;
      // Stopped and nothing to do: wait for the next play.
      if (speed === 0 && target.current === 0) {
        frame = 0;
        last = 0;
        return;
      }
      frame = requestAnimationFrame(step);
    };
    const kick = () => {
      if (!frame) frame = requestAnimationFrame(step);
    };
    kick();
    el.addEventListener("platter-kick", kick);
    return () => {
      el.removeEventListener("platter-kick", kick);
      cancelAnimationFrame(frame);
    };
  }, []);

  // A change of play state wakes the loop.
  useEffect(() => {
    record.current?.dispatchEvent(new Event("platter-kick"));
  }, [playing]);

  return (
    <div className="turntable" data-state={playing ? "playing" : "paused"}>
      <div className="turntable__platter" aria-hidden="true" />
      <div className="turntable__record" ref={record}>
        <div className="turntable__label">
          <Art path={thumb} fluid requestSize={requestSize} />
        </div>
      </div>
      <div className="turntable__spindle" aria-hidden="true" />
      {/* Under reduced motion the arm is drawn again in its new place and fades in. */}
      <div className="turntable__arm" key={prefersReducedMotion() ? (playing ? "down" : "up") : "arm"} aria-hidden="true">
        <svg viewBox="0 0 100 300" preserveAspectRatio="xMidYMin meet">
          <circle className="turntable__pivot" cx="50" cy="40" r="26" />
          <circle className="turntable__pivot-cap" cx="50" cy="40" r="11" />
          <path className="turntable__tube" d="M50 40 L50 230 L34 262" />
          <rect className="turntable__head" x="20" y="252" width="26" height="36" rx="3" transform="rotate(28 33 270)" />
        </svg>
      </div>
    </div>
  );
}
