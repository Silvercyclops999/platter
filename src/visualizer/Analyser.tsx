/**
 * Analyser: one thin spectrum line over a log frequency axis and a dB
 * scale, with a slower peak-hold line behind it, like a studio analyser.
 * The scale is drawn once (SVG, in the look's surface colours); the lines
 * are drawn on a canvas with WebGL2 when there is one, Canvas 2D otherwise
 * or after the GPU context is lost.
 */
import { useEffect, useMemo, useRef, useState } from "react";
import type { VisualizerSmoothing } from "../lib/types";
import { fitCanvas, useBoxSize, useFrameLoop, useLookColours } from "./draw";
import { latestFrame } from "./feed";
import { linesFor, type Line, type LineRenderer } from "./lines";
import { AnalyserState, BANDS, DB_TICKS, FREQ_TICKS, bandCentre, current, dbY, freqLabel, freqX } from "./scale";

/** Plot insets for the labels, in CSS pixels. */
const PAD_FULL = { l: 34, r: 14, t: 20, b: 22 };
const PAD_SLIM = { l: 0, r: 0, t: 2, b: 2 };

export interface AnalyserProps {
  fps: number;
  smoothing: VisualizerSmoothing;
  /** The slower peak line (off under reduced motion). */
  peakHold: boolean;
  /** The slim strip under the cover: no labels, no peak line. */
  slim?: boolean;
  /** The cover's colour, or null for the look's accent. */
  accent: string | null;
  /** Tests and the fallback: never try WebGL2. */
  canvas2d?: boolean;
}

export function Analyser({ fps, smoothing, peakHold, slim = false, accent, canvas2d = false }: AnalyserProps) {
  const box = useRef<HTMLDivElement>(null);
  const { w, h } = useBoxSize(box);
  const look = useLookColours(box);
  const pad = slim ? PAD_SLIM : PAD_FULL;
  const pw = Math.max(0, w - pad.l - pad.r);
  const ph = Math.max(0, h - pad.t - pad.b);
  // A lost GPU context: a fresh canvas, drawn with Canvas 2D from then on.
  const [useGl, setUseGl] = useState(!canvas2d);
  const canvas = useRef<HTMLCanvasElement>(null);
  const renderer = useRef<LineRenderer | null>(null);
  const state = useMemo(() => new AnalyserState(), []);
  const [kind, setKind] = useState<"webgl2" | "2d" | "none">("none");

  useEffect(() => {
    const c = canvas.current;
    if (!c) return;
    const r = linesFor(c, useGl);
    if (!r && useGl) {
      setUseGl(false);
      return;
    }
    renderer.current = r;
    setKind(r?.kind ?? "none");
    const lost = (e: Event) => {
      e.preventDefault();
      renderer.current = null;
      setUseGl(false);
    };
    c.addEventListener("webglcontextlost", lost);
    return () => {
      c.removeEventListener("webglcontextlost", lost);
      r?.dispose();
      renderer.current = null;
    };
  }, [useGl]);

  const pts = useMemo(() => ({ line: new Float32Array(BANDS * 2), peak: new Float32Array(BANDS * 2) }), []);
  const xs = useMemo(() => Array.from({ length: BANDS }, (_, k) => pad.l + freqX(bandCentre(k)) * pw), [pad.l, pw]);

  useFrameLoop(
    (dt, now) => {
      const c = canvas.current;
      const r = renderer.current;
      if (!c || !r || !w || !h) return;
      const { frame, at } = latestFrame();
      const f = current(frame, at, now);
      const hold = peakHold && !slim;
      state.step(f?.bands ?? null, dt, smoothing, hold);
      for (let k = 0; k < BANDS; k++) {
        pts.line[k * 2] = xs[k];
        pts.line[k * 2 + 1] = pad.t + dbY(state.shown[k]) * ph;
        pts.peak[k * 2] = xs[k];
        pts.peak[k * 2 + 1] = pad.t + dbY(state.peaks[k]) * ph;
      }
      const colour = accent ?? look.current.accent;
      const lines: Line[] = [];
      if (hold) lines.push({ points: pts.peak, colour, alpha: 0.42, width: 1 });
      lines.push({ points: pts.line, colour, alpha: 1, width: slim ? 1.25 : 1.5 });
      const dpr = fitCanvas(c, w, h);
      r.draw(lines, w, h, dpr);
    },
    fps,
    w > 0 && h > 0,
  );

  return (
    <div ref={box} className={`vis-analyser ${slim ? "vis-analyser--slim" : ""}`} data-renderer={kind} role="img" aria-label="Spectrum analyser">
      {w > 0 && h > 0 && (
        <svg className="vis-scale" width={w} height={h} aria-hidden="true">
          {!slim &&
            DB_TICKS.map((db) => {
              const y = pad.t + dbY(db) * ph;
              return (
                <g key={db}>
                  <line className={db === 0 ? "vis-scale__line vis-scale__line--ref" : "vis-scale__line"} x1={pad.l} x2={pad.l + pw} y1={y} y2={y} />
                  <text className="vis-scale__label" x={pad.l - 6} y={y} textAnchor="end" dominantBaseline="middle">
                    {db}
                  </text>
                </g>
              );
            })}
          {FREQ_TICKS.map((hz) => {
            const x = pad.l + freqX(hz) * pw;
            return (
              <g key={hz}>
                <line className="vis-scale__line" x1={x} x2={x} y1={pad.t} y2={pad.t + ph} />
                {!slim && (
                  <text className="vis-scale__label" x={x} y={pad.t + ph + 14} textAnchor="middle">
                    {freqLabel(hz)}
                  </text>
                )}
              </g>
            );
          })}
          {!slim && (
            <text className="vis-scale__unit" x={pad.l - 6} y={pad.t + ph + 14} textAnchor="end">
              Hz
            </text>
          )}
          {!slim && (
            <text className="vis-scale__unit" x={pad.l - 6} y={pad.t - 10} textAnchor="end">
              dBFS
            </text>
          )}
        </svg>
      )}
      <canvas key={useGl ? "gl" : "2d"} ref={canvas} className="vis-canvas" style={{ width: w, height: h }} />
    </div>
  );
}
