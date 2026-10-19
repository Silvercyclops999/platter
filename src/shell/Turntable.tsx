import { useEffect, useId, useRef, type ReactNode, type Ref } from "react";
import { type SleeveText } from "../components/Art";
import { RecordSheen, Spindle, Vinyl, type RecordLabel, type RecordSide } from "../components/Record";
import { artUrl } from "../lib/backend";
import type { Track } from "../lib/types";
import { prefersReducedMotion } from "../design/motion";
import { usePlayer } from "../stores/player";
import {
  ARM_LENGTH,
  ARM_OFFSET_DEG,
  CENTRE,
  LABEL_R,
  PIVOT,
  PLATTER_R,
  REST_DEG,
  VIEW_H,
  VIEW_W,
  armAngleForRadius,
  grooveRadius,
  sideProgress,
} from "./recordGeometry";
import { useRecordSide } from "./useRecordSide";
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

/** The arm's angle: on its rest, or over the groove the music has reached. */
export function armAngle(playing: boolean, side: RecordSide | null | undefined, positionMs: number): number {
  if (!playing || !side || side.current < 0) return REST_DEG;
  const total = side.durationsMs.reduce((a, b) => a + b, 0);
  return armAngleForRadius(grooveRadius(sideProgress(side.durationsMs, side.current, positionMs), total));
}

const rad = (d: number) => (d * Math.PI) / 180;

/** The stylus end of the arm, in the arm's own axes (pivot at 0,0, stylus at ARM_LENGTH,0). */
const HEAD_BACK = 52;
const JOINT = { x: ARM_LENGTH - HEAD_BACK * Math.cos(rad(ARM_OFFSET_DEG)), y: -HEAD_BACK * Math.sin(rad(ARM_OFFSET_DEG)) };
const TUBE_DEG = (Math.atan2(JOINT.y, JOINT.x) * 180) / Math.PI;
const TUBE_LEN = Math.hypot(JOINT.x, JOINT.y);
/** The arm rest: under the tube, 150 mm out from the pivot, with the arm parked. */
const REST_AT = { x: PIVOT.x - 150 * Math.sin(rad(TUBE_DEG)), y: PIVOT.y + 150 * Math.cos(rad(TUBE_DEG)) };

/** The moving part of the arm, drawn along +x from the pivot. `ink` draws its shadow instead. */
function ArmBody({ ids, ink }: { ids: (s: string) => string; ink?: boolean }) {
  const f = (fill: string) => (ink ? "#000" : fill);
  return (
    <>
      {/* Counterweight behind the pivot, on its stub. */}
      <rect x={-34} y={-3.2} width={28} height={6.4} fill={f(`url(#${ids("tube")})`)} />
      <rect x={-66} y={-12} width={34} height={24} rx={2} fill={f(`url(#${ids("weight")})`)} />
      {!ink && <line x1={-40} y1={-11.8} x2={-40} y2={11.8} stroke="#000" strokeOpacity="0.35" strokeWidth="0.5" />}
      {/* The tube. */}
      <g transform={`rotate(${TUBE_DEG})`}>
        <rect x={6} y={-4.25} width={TUBE_LEN - 8} height={8.5} rx={1} fill={f(`url(#${ids("tube")})`)} />
        {/* The collar at the headshell. */}
        <rect x={TUBE_LEN - 14} y={-5.4} width={13} height={10.8} rx={1.2} fill={f(`url(#${ids("collar")})`)} />
      </g>
      {/* Bearing housing over the pivot. */}
      <circle r={12.5} fill={f(`url(#${ids("housing")})`)} />
      {!ink && <circle r={6} fill={`url(#${ids("cap")})`} />}
      {/* Headshell turned in by the offset angle, with the cartridge and the finger lift. */}
      <g transform={`translate(${ARM_LENGTH} 0) rotate(${ARM_OFFSET_DEG})`}>
        <path d="M -14 -8.6 L -17 -24.5" stroke={f("#b9b6ae")} strokeWidth="2.6" strokeLinecap="round" fill="none" />
        <rect x={-HEAD_BACK} y={-9} width={HEAD_BACK + 6} height={18} rx={2.2} fill={f(`url(#${ids("shell")})`)} />
        <rect x={-31} y={-8.2} width={34} height={16.4} rx={1.3} fill={f("#141414")} />
        {!ink && (
          <>
            <rect x={-31} y={-8.2} width={34} height={16.4} rx={1.3} fill="none" stroke="#fff" strokeOpacity="0.08" strokeWidth="0.4" />
            <circle cx={-22} cy={-4.6} r={0.95} fill="#8f8c86" />
            <circle cx={-22} cy={4.6} r={0.95} fill="#8f8c86" />
          </>
        )}
      </g>
    </>
  );
}

/** The arm at the angle the music has reached. */
function LiveArm({ playing, side, ids }: { playing: boolean; side: RecordSide | null; ids: (s: string) => string }) {
  const positionMs = usePlayer((s) => s.positionMs);
  return <Arm deg={armAngle(playing, side, positionMs)} ids={ids} />;
}

/** The arm and its shadow at an angle. */
function Arm({ deg, ids }: { deg: number; ids: (s: string) => string }) {
  const down = deg !== REST_DEG;
  // The shadow falls down and right of the arm; higher up (on its rest, or lifting), it falls further and softer.
  const off = down ? { x: 2.2, y: 3.2 } : { x: 4.5, y: 6.5 };
  const turn = (dx: number, dy: number) => `translate(${(PIVOT.x + dx).toFixed(2)}px, ${(PIVOT.y + dy).toFixed(2)}px) rotate(${deg.toFixed(3)}deg)`;
  return (
    <>
      <g className="turntable__arm-shadow" style={{ transform: turn(off.x, off.y), opacity: down ? 0.3 : 0.2 }}>
        <g opacity="0.55" stroke="#000" strokeWidth="2.4" strokeLinejoin="round">
          <ArmBody ids={ids} ink />
        </g>
        <ArmBody ids={ids} ink />
      </g>
      <g className="turntable__arm" data-angle={deg.toFixed(2)} style={{ transform: turn(0, 0) }}>
        <ArmBody ids={ids} />
      </g>
    </>
  );
}

/**
 * The record player's drawing: platter, record, light, spindle and the arm's
 * fixed parts, with the arm passed in (it needs the same gradient ids).
 */
export function RecordPlayerArt({
  label,
  side,
  seed,
  detail = "full",
  recordRef,
  children,
}: {
  label: RecordLabel;
  side: RecordSide | null;
  seed: string;
  detail?: "full" | "low";
  recordRef?: Ref<SVGGElement>;
  children: ReactNode | ((ids: (s: string) => string) => ReactNode);
}) {
  const raw = useId().replace(/[^a-zA-Z0-9_-]/g, "");
  const ids = (s: string) => `tt${raw}-${s}`;
  return (
    <svg className="turntable__svg" viewBox={`0 0 ${VIEW_W} ${VIEW_H}`} aria-hidden="true" focusable="false">
      <defs>
        {/* Satin aluminium across a round tube: dull at the edges, a soft band of light off centre. */}
        <linearGradient id={ids("tube")} x1="0" y1="0" x2="0" y2="1">
          <stop offset="0" stopColor="#55534f" />
          <stop offset="0.3" stopColor="#b9b6af" />
          <stop offset="0.42" stopColor="#d6d3cc" />
          <stop offset="0.62" stopColor="#97948d" />
          <stop offset="1" stopColor="#3f3d3a" />
        </linearGradient>
        <linearGradient id={ids("collar")} x1="0" y1="0" x2="0" y2="1">
          <stop offset="0" stopColor="#2b2a28" />
          <stop offset="0.4" stopColor="#6d6a65" />
          <stop offset="1" stopColor="#1f1e1c" />
        </linearGradient>
        <linearGradient id={ids("weight")} x1="0" y1="0" x2="0" y2="1">
          <stop offset="0" stopColor="#3a3936" />
          <stop offset="0.35" stopColor="#8e8b84" />
          <stop offset="0.5" stopColor="#a29f98" />
          <stop offset="0.75" stopColor="#5d5b56" />
          <stop offset="1" stopColor="#2a2927" />
        </linearGradient>
        <linearGradient id={ids("shell")} x1="0" y1="0" x2="0" y2="1">
          <stop offset="0" stopColor="#3b3a37" />
          <stop offset="0.5" stopColor="#56544f" />
          <stop offset="1" stopColor="#2d2c2a" />
        </linearGradient>
        <radialGradient id={ids("housing")} cx="0.4" cy="0.36" r="0.7">
          <stop offset="0" stopColor="#4a4945" />
          <stop offset="1" stopColor="#1f1e1c" />
        </radialGradient>
        <radialGradient id={ids("cap")} cx="0.38" cy="0.34" r="0.75">
          <stop offset="0" stopColor="#d4d1ca" />
          <stop offset="1" stopColor="#6e6b65" />
        </radialGradient>
        <radialGradient id={ids("platter")} r="0.5">
          <stop offset="0.985" stopColor="#8a8780" />
          <stop offset="1" stopColor="#4a4844" />
        </radialGradient>
        {/* The platter's shadow on the plinth: soft, without a filter. */}
        <radialGradient id={ids("drop")} r="0.5">
          <stop offset="0.9" stopColor="#000" stopOpacity="0.38" />
          <stop offset="1" stopColor="#000" stopOpacity="0" />
        </radialGradient>
        <radialGradient id={ids("base")} cx="0.42" cy="0.38" r="0.7">
          <stop offset="0" stopColor="#3a3936" />
          <stop offset="1" stopColor="#1b1a19" />
        </radialGradient>
      </defs>

      {/* Platter and its shadow. */}
      <circle cx={CENTRE.x + 2.5} cy={CENTRE.y + 4} r={PLATTER_R + 8} fill={`url(#${ids("drop")})`} />
      <circle className="turntable__platter" cx={CENTRE.x} cy={CENTRE.y} r={PLATTER_R} fill={`url(#${ids("platter")})`} />

      {/* The record: the only part that turns. */}
      {/* Placed by its attribute; while it turns, the motor's style transform takes over. */}
      <g className="turntable__record" ref={recordRef} transform={`translate(${CENTRE.x} ${CENTRE.y})`}>
        <Vinyl side={side} label={label} seed={seed} detail={detail} />
      </g>
      <g transform={`translate(${CENTRE.x} ${CENTRE.y})`}>
        <RecordSheen />
        <Spindle />
      </g>

      {/* The arm's fixed parts: base, cue lever, rest. */}
      <g className="turntable__arm-base">
        {/* The armboard the base and the rest are mounted on: a plain plate, a shade darker than the plinth. */}
        <rect x={PIVOT.x - 30} y={PIVOT.y - 34} width={74} height={REST_AT.y - PIVOT.y + 52} rx={6} fill="#000" fillOpacity="0.12" />
        <circle cx={PIVOT.x + 1.5} cy={PIVOT.y + 2.5} r={27} fill="#000" opacity="0.25" />
        <circle cx={PIVOT.x} cy={PIVOT.y} r={26} fill={`url(#${ids("base")})`} />
        <circle cx={PIVOT.x} cy={PIVOT.y} r={25.6} fill="none" stroke="#fff" strokeOpacity="0.07" strokeWidth="0.6" />
        {/* The cue lever, on the base. */}
        <g transform={`translate(${PIVOT.x} ${PIVOT.y}) rotate(-20)`}>
          <rect x={22} y={-3} width={9} height={6} rx={1.2} fill="#262523" />
          <path d="M 29 0 L 38 0" stroke="#9c9993" strokeWidth="1.8" strokeLinecap="round" />
          <circle cx={39.5} cy={0} r={2.4} fill="#1a1918" stroke="#fff" strokeOpacity="0.12" strokeWidth="0.4" />
        </g>
        {/* The rest: a post with a cradle, under the tube when the arm is parked. */}
        <g transform={`translate(${REST_AT.x} ${REST_AT.y}) rotate(${REST_DEG + TUBE_DEG})`}>
          <circle r={5.5} fill="#000" opacity="0.3" cx={1.5} cy={2} />
          <circle r={5} fill="#2a2927" />
          <rect x={-2.5} y={-8} width={5} height={16} rx={1.2} fill="#1d1c1b" stroke="#fff" strokeOpacity="0.08" strokeWidth="0.4" />
        </g>
      </g>

      {typeof children === "function" ? children(ids) : children}
    </svg>
  );
}

/** The record player held still, for the look previews: the arm on the groove at `progress` through the side. */
export function RecordPlayerStill({ label, progress = 0.3, seed }: { label: RecordLabel; progress?: number; seed: string }) {
  const side: RecordSide = { durationsMs: [262_000, 228_000, 301_000, 245_000, 190_000], current: 1 };
  const total = side.durationsMs.reduce((a, b) => a + b, 0);
  const deg = armAngleForRadius(grooveRadius(progress, total));
  return <RecordPlayerArt label={label} side={side} seed={seed} detail="low">{(ids) => <Arm deg={deg} ids={ids} />}</RecordPlayerArt>;
}

/**
 * A record player drawn to scale from above: the record on its platter with
 * the cover printed on the label, and a 9-inch arm. The record spins up at
 * 33 1/3 rpm when music plays and coasts to a stop on pause (when `spin`);
 * the arm swings from its rest onto the groove the music has reached and
 * follows it in. Under reduced motion nothing turns and the arm is simply
 * in its place.
 */
export function Turntable({
  thumb,
  sleeve,
  playing,
  spin = true,
  track,
  requestSize = 640,
  children,
}: {
  thumb: string | null | undefined;
  /** The record's title and artist, printed on the label when it has no cover. */
  sleeve?: SleeveText | null;
  playing: boolean;
  /** Turn the record (the record player setting). Off, it holds still and only the arm moves. */
  spin?: boolean;
  /** The playing song, to place the grooves and the arm. */
  track?: Pick<Track, "id" | "albumId" | "durationMs"> | null;
  requestSize?: number;
  /** Laid over the record (the Flip button at the end of a side). */
  children?: ReactNode;
}) {
  const record = useRef<SVGGElement>(null);
  const target = useRef(0);
  target.current = playing && spin ? SPIN_DEG_PER_S : 0;
  const side = useRecordSide(track);

  useEffect(() => {
    const el = record.current;
    if (!el) return;
    let angle = 0;
    let speed = 0;
    let last = 0;
    let frame = 0;
    const put = () => {
      el.style.transform = `translate(${CENTRE.x}px, ${CENTRE.y}px) rotate(${angle.toFixed(2)}deg)`;
    };
    put();
    const step = (t: number) => {
      const dt = last ? Math.min(100, t - last) : 16;
      last = t;
      if (prefersReducedMotion()) speed = 0;
      else speed = nextSpeed(speed, target.current, dt);
      angle = (angle + (speed * dt) / 1000) % 360;
      put();
      // Stopped and nothing to do: wait for the next play.
      if (speed === 0) {
        frame = 0;
        last = 0;
        return;
      }
      frame = requestAnimationFrame(step);
    };
    const kick = () => {
      if (!frame && target.current !== 0 && !prefersReducedMotion()) frame = requestAnimationFrame(step);
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
  }, [playing, spin]);

  const url = artUrl(thumb, Math.round(requestSize * ((LABEL_R * 2) / VIEW_W)) + 24);

  return (
    <div className="turntable" data-state={playing ? "playing" : "paused"}>
      <RecordPlayerArt recordRef={record} label={{ url, sleeve }} side={side} seed={track?.albumId ?? sleeve?.title ?? "record"}>
        {(ids) => <LiveArm playing={playing} side={side} ids={ids} />}
      </RecordPlayerArt>
      {children && <div className="turntable__over">{children}</div>}
    </div>
  );
}
