/**
 * Scope: a stereo phase scope (goniometer). Mid runs up, side across, with
 * left on the upper-left diagonal and right on the upper-right, so a mono
 * song draws an upright line and a wide mix spreads out. The last two frames
 * stay faintly, like a phosphor. Under it, the correlation between the
 * channels, from -1 (out of phase) to +1 (mono). The frame is drawn once
 * (SVG); the trace on a Canvas 2D.
 */
import { useMemo, useRef } from "react";
import { fitCanvas, useBoxSize, useFrameLoop, useLookColours } from "./draw";
import { latestFrame } from "./feed";
import { current, scopePoint } from "./scale";

/** How the channels move together over a frame's pairs: +1 mono, 0 unrelated, -1 out of phase. */
export function correlation(pairs: ArrayLike<number>): number | null {
  let lr = 0;
  let ll = 0;
  let rr = 0;
  for (let i = 0; i + 1 < pairs.length; i += 2) {
    const l = pairs[i];
    const r = pairs[i + 1];
    lr += l * r;
    ll += l * l;
    rr += r * r;
  }
  const d = Math.sqrt(ll * rr);
  return d > 1e-9 ? Math.max(-1, Math.min(1, lr / d)) : null;
}

const TRAILS = [0.18, 0.4, 1];

export function Scope({ fps, accent }: { fps: number; accent: string | null }) {
  const box = useRef<HTMLDivElement>(null);
  const { w, h } = useBoxSize(box);
  const look = useLookColours(box);
  const canvas = useRef<HTMLCanvasElement>(null);
  const readout = useRef<HTMLSpanElement>(null);
  const bar = useRef<HTMLSpanElement>(null);
  const history = useRef<(number[] | null)[]>([]);
  const lastT = useRef<number | null>(null);
  // The plot is square, centred, with room under it for the readout.
  const size = Math.max(0, Math.min(w, h - 40));
  const ox = (w - size) / 2;
  const half = size / 2;
  const scale = half * 0.92;
  const frameD = useMemo(() => {
    const c = ox + half;
    return {
      lx: c - half * 0.7,
      rx: c + half * 0.7,
      top: half - half * 0.7,
      c,
    };
  }, [ox, half]);

  useFrameLoop(
    (_dt, now) => {
      const c = canvas.current;
      if (!c || !w || !h) return;
      const ctx = c.getContext("2d");
      if (!ctx) return;
      const { frame, at } = latestFrame();
      const f = current(frame, at, now);
      if (!f?.scope) history.current = [];
      else if (f.t !== lastT.current) {
        lastT.current = f.t;
        history.current = [...history.current, f.scope].slice(-TRAILS.length);
      }
      const dpr = fitCanvas(c, w, h);
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
      ctx.clearRect(0, 0, w, h);
      const colour = accent ?? look.current.accent;
      const traces = history.current;
      traces.forEach((pairs, i) => {
        if (!pairs) return;
        ctx.globalAlpha = TRAILS[TRAILS.length - traces.length + i];
        ctx.strokeStyle = colour;
        ctx.lineWidth = 1;
        ctx.beginPath();
        for (let p = 0; p + 1 < pairs.length; p += 2) {
          const [x, y] = scopePoint(pairs[p], pairs[p + 1]);
          const px = ox + half + x * scale;
          const py = half - y * scale;
          if (p === 0) ctx.moveTo(px, py);
          else ctx.lineTo(px, py);
        }
        ctx.stroke();
      });
      ctx.globalAlpha = 1;
      const corr = f?.scope ? correlation(f.scope) : null;
      if (readout.current) readout.current.textContent = corr === null ? "–" : `${corr >= 0 ? "+" : "−"}${Math.abs(corr).toFixed(2)}`;
      if (bar.current) bar.current.style.left = `${((corr ?? 0) + 1) * 50}%`;
    },
    fps,
    w > 0 && h > 0,
  );

  return (
    <div ref={box} className="vis-scope" role="img" aria-label="Stereo phase scope">
      {size > 0 && (
        <svg className="vis-scale" width={w} height={h} aria-hidden="true">
          <rect className="vis-scale__line" x={ox + 0.5} y={0.5} width={size - 1} height={size - 1} fill="none" />
          <line className="vis-scale__line" x1={frameD.c} x2={frameD.c} y1={0} y2={size} />
          <line className="vis-scale__line" x1={ox} x2={ox + size} y1={half} y2={half} />
          <line className="vis-scale__line vis-scale__line--ref" x1={frameD.c} x2={frameD.lx} y1={half} y2={frameD.top} />
          <line className="vis-scale__line vis-scale__line--ref" x1={frameD.c} x2={frameD.rx} y1={half} y2={frameD.top} />
          <text className="vis-scale__label" x={frameD.lx - 4} y={frameD.top - 4} textAnchor="end">
            L
          </text>
          <text className="vis-scale__label" x={frameD.rx + 4} y={frameD.top - 4}>
            R
          </text>
          <text className="vis-scale__label" x={frameD.c + 6} y={14}>
            M
          </text>
          <text className="vis-scale__label" x={ox + size - 6} y={half - 6} textAnchor="end">
            S
          </text>
        </svg>
      )}
      <canvas ref={canvas} className="vis-canvas" style={{ width: w, height: h }} />
      <div className="vis-scope__corr" style={{ width: size || undefined }}>
        <span className="vis-scope__end">{"−1"}</span>
        <span className="vis-scope__track">
          <span ref={bar} className="vis-scope__dot" />
        </span>
        <span className="vis-scope__end">+1</span>
        <span className="vis-scope__label">
          Correlation <span ref={readout}>{"–"}</span>
        </span>
      </div>
    </div>
  );
}
