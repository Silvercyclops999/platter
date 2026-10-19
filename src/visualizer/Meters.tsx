/**
 * Meters: a pair of needle meters, left and right, each with a peak lamp.
 * The needles move like a real VU (300 ms to a reading, rising and falling
 * alike) over a scale that is linear in voltage, +3 at the end. The face is
 * drawn once (SVG); the needles and lamps on a Canvas 2D.
 */
import { useMemo, useRef } from "react";
import { fitCanvas, useBoxSize, useFrameLoop, useLookColours } from "./draw";
import { latestFrame } from "./feed";
import { Needle, PEAK_LAMP_DBFS, PEAK_LAMP_MS, VU_MARKS, VU_REF_DBFS, current, vuPosition } from "./scale";

/** The arc runs this many degrees either side of upright. */
const SWEEP = 48;

interface Geometry {
  w: number;
  h: number;
  cx: number;
  cy: number;
  r: number;
}

function geometry(w: number, h: number, compact: boolean): Geometry {
  // The pivot sits near the foot of the face, the arc high over it.
  // Room either side for the scale's numbers; a tall face centres the dial.
  if (compact) return { w, h, cx: w / 2, cy: h * 0.94, r: Math.min(w * 0.55, h * 0.82) };
  const r = Math.max(0, Math.min(w * 0.5 - 16, h * 0.78));
  const cy = Math.min(h * 0.9, h / 2 + r * 0.55 + 8);
  return { w, h, cx: w / 2, cy, r };
}

const angleOf = (pos: number) => ((-SWEEP + 2 * SWEEP * Math.min(1.04, Math.max(0, pos))) * Math.PI) / 180;
const at = (g: Geometry, pos: number, rad: number): [number, number] => {
  const a = angleOf(pos);
  return [g.cx + Math.sin(a) * rad, g.cy - Math.cos(a) * rad];
};

function arcPath(g: Geometry, from: number, to: number, rad: number): string {
  const [x0, y0] = at(g, from, rad);
  const [x1, y1] = at(g, to, rad);
  return `M ${x0} ${y0} A ${rad} ${rad} 0 0 1 ${x1} ${y1}`;
}

const LABELLED = new Set([-20, -10, -5, -3, 0, 3]);
const COMPACT_LABELLED = new Set([-20, 0, 3]);
const SMALL_LABELLED = new Set([-20, -5, 0, 3]);

function Face({ g, side, compact }: { g: Geometry; side: "L" | "R"; compact: boolean }) {
  const ref = vuPosition(0);
  // A small dial keeps fewer numbers, so they never touch.
  const labelled = compact ? COMPACT_LABELLED : g.r < 110 ? SMALL_LABELLED : LABELLED;
  return (
    <svg className="vis-meter__face" width={g.w} height={g.h} aria-hidden="true">
      <path className="vis-meter__arc" d={arcPath(g, 0, ref, g.r)} />
      <path className="vis-meter__arc vis-meter__arc--over" d={arcPath(g, ref, 1, g.r)} />
      {VU_MARKS.map((m) => {
        const p = vuPosition(m);
        const long = labelled.has(m);
        const [x0, y0] = at(g, p, g.r);
        const [x1, y1] = at(g, p, g.r + (long ? 7 : 4));
        const [tx, ty] = at(g, p, g.r + (compact ? 13 : 17));
        return (
          <g key={m}>
            <line className="vis-meter__tick" x1={x0} y1={y0} x2={x1} y2={y1} />
            {long && !compact && (
              <text className="vis-meter__mark" x={tx} y={ty} textAnchor="middle" dominantBaseline="middle">
                {m > 0 ? `+${m}` : m}
              </text>
            )}
          </g>
        );
      })}
      <text className="vis-meter__side" x={compact ? 8 : 12} y={g.h - (compact ? 7 : 12)}>
        {side}
      </text>
      {!compact && (
        <text className="vis-meter__side" x={g.w - 24} y={g.h - 12} textAnchor="end">
          PEAK
        </text>
      )}
      {!compact && (
        <text className="vis-meter__unit" x={g.cx} y={g.cy - g.r * 0.42} textAnchor="middle">
          VU
        </text>
      )}
    </svg>
  );
}

export interface MetersProps {
  fps: number;
  /** The cover's colour, or null for the look's accent. */
  accent: string | null;
  /** The Deck faceplate's small pair. */
  compact?: boolean;
}

export function Meters({ fps, accent, compact = false }: MetersProps) {
  const box = useRef<HTMLDivElement>(null);
  const { w, h } = useBoxSize(box);
  const look = useLookColours(box);
  const canvas = useRef<HTMLCanvasElement>(null);
  const needles = useMemo(() => [new Needle(), new Needle()], []);
  const lamps = useRef([0, 0]);
  const gap = compact ? 6 : 16;
  const faceW = Math.max(0, (w - gap) / 2);
  const g = useMemo(() => geometry(faceW, h, compact), [faceW, h, compact]);

  useFrameLoop(
    (dt, now) => {
      const c = canvas.current;
      if (!c || !w || !h) return;
      const ctx = c.getContext("2d");
      if (!ctx) return;
      const { frame, at: arrived } = latestFrame();
      const f = current(frame, arrived, now);
      const dpr = fitCanvas(c, w, h);
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
      ctx.clearRect(0, 0, w, h);
      const colour = accent ?? look.current.accent;
      const dim = look.current.muted;
      for (let ch = 0; ch < 2; ch++) {
        needles[ch].step(f ? f.rms[ch] : -Infinity, dt);
        if (f && f.peak[ch] >= PEAK_LAMP_DBFS) lamps.current[ch] = now + PEAK_LAMP_MS;
        const ox = ch * (faceW + gap);
        ctx.save();
        ctx.beginPath();
        ctx.rect(ox, 0, faceW, h);
        ctx.clip();
        const a = angleOf(needles[ch].arc);
        ctx.strokeStyle = colour;
        ctx.lineWidth = compact ? 1.25 : 1.5;
        ctx.lineCap = "round";
        ctx.beginPath();
        ctx.moveTo(ox + g.cx, g.cy);
        ctx.lineTo(ox + g.cx + Math.sin(a) * (g.r + 4), g.cy - Math.cos(a) * (g.r + 4));
        ctx.stroke();
        // The pivot's cap.
        ctx.beginPath();
        ctx.arc(ox + g.cx, g.cy, compact ? 2 : 3.5, 0, Math.PI * 2);
        ctx.fillStyle = dim;
        ctx.fill();
        // The peak lamp: lit in the accent for a moment after a peak near full scale.
        const lit = now < lamps.current[ch];
        const lx = ox + faceW - (compact ? 9 : 14);
        const ly = h - (compact ? 9 : 16);
        ctx.beginPath();
        ctx.arc(lx, ly, compact ? 2.5 : 3.5, 0, Math.PI * 2);
        if (lit) {
          ctx.fillStyle = colour;
          ctx.fill();
        } else {
          ctx.strokeStyle = dim;
          ctx.lineWidth = 1;
          ctx.stroke();
        }
        ctx.restore();
      }
    },
    fps,
    w > 0 && h > 0,
  );

  const minus = (n: number) => String(n).replace("-", "−");
  return (
    <div className={`vis-meters ${compact ? "vis-meters--compact" : ""}`} role="img" aria-label="Level meters, left and right">
      <div ref={box} className="vis-meters__dials">
        {faceW > 0 && h > 0 && (
          <>
            <div className="vis-meter" style={{ left: 0, width: faceW }}>
              <Face g={g} side="L" compact={compact} />
            </div>
            <div className="vis-meter" style={{ left: faceW + gap, width: faceW }}>
              <Face g={g} side="R" compact={compact} />
            </div>
          </>
        )}
        <canvas ref={canvas} className="vis-canvas" style={{ width: w, height: h }} />
      </div>
      {!compact && (
        <p className="vis-meters__note">
          0 VU = {minus(VU_REF_DBFS)} dBFS, peak lamp at {minus(PEAK_LAMP_DBFS)} dBFS
        </p>
      )}
    </div>
  );
}
