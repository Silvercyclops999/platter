/**
 * The sound map itself: a canvas with every song as a dot. Wheel or pinch
 * zooms around the pointer, dragging moves the map (or draws a lasso in
 * lasso mode, or with Shift held), clicking a dot plays it, and keys move
 * and zoom when the map has focus. Drawing happens once per frame and only
 * when something changed.
 */
import { useEffect, useRef, type MutableRefObject } from "react";
import { prefersReducedMotion } from "../../design/motion";
import {
  buildGrid,
  dotRadius,
  fitView,
  lassoSelect,
  nearest,
  toMap,
  toScreen,
  zoomAt,
  type Grid,
  type Legend,
  type MapData,
  type View,
} from "./geometry";

export type MapMode = "move" | "lasso";

/** What the page can ask of the map. */
export interface MapApi {
  /** Centre on a map point, zoomed in `zoom` times the fitted view. */
  flyTo(x: number, y: number, zoom: number): void;
  /** Back to the whole map. */
  reset(): void;
}

interface Props {
  data: MapData;
  legend: Legend;
  /** CSS colours per legend group. */
  colours: string[];
  ring: string;
  highlight: string;
  mode: MapMode;
  /** Selected points (1: selected), or null. */
  selected: Uint8Array | null;
  /** A point to mark (found by search), or -1. */
  focus: number;
  api: MutableRefObject<MapApi | null>;
  label: string;
  onHover: (i: number, sx: number, sy: number) => void;
  onPick: (i: number) => void;
  onLasso: (indices: number[]) => void;
  onEscape: () => void;
}

/** Fallback size where layout gives none (tests). */
const FALLBACK: [number, number] = [800, 600];
const HIT_PX = 8;
const DRAG_PX = 4;

export function MapCanvas(p: Props) {
  const wrap = useRef<HTMLDivElement>(null);
  const canvas = useRef<HTMLCanvasElement>(null);
  const size = useRef<[number, number]>(FALLBACK);
  const view = useRef<View>({ cx: 0, cy: 0, scale: 1 });
  const base = useRef(1);
  const grid = useRef<Grid | null>(null);
  const lasso = useRef<[number, number][] | null>(null);
  const hover = useRef(-1);
  const frame = useRef(0);
  const props = useRef(p);
  props.current = p;

  const draw = () => {
    if (frame.current) return;
    frame.current = requestAnimationFrame(() => {
      frame.current = 0;
      paint();
    });
  };

  const paint = () => {
    const c = canvas.current;
    const ctx = c?.getContext?.("2d");
    if (!c || !ctx) return;
    const { data, legend, colours, ring, highlight, selected, focus } = props.current;
    const [w, h] = size.current;
    const dpr = window.devicePixelRatio || 1;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, w, h);
    const v = view.current;
    const r = dotRadius(v, base.current);
    const n = data.ids.length;
    const groups = legend.groups.length;
    // One path per colour (and per dimmed or not), so a big library is a few fills.
    const paths: Path2D[] = [];
    const dim: Path2D[] = [];
    const rings = new Path2D();
    for (let g = 0; g < groups; g++) {
      paths.push(new Path2D());
      dim.push(new Path2D());
    }
    const sel = selected && selected.some((x) => x === 1) ? selected : null;
    for (let i = 0; i < n; i++) {
      const [sx, sy] = toScreen(v, w, h, data.x[i], data.y[i]);
      if (sx < -10 || sy < -10 || sx > w + 10 || sy > h + 10) continue;
      const path = sel && !sel[i] ? dim[legend.group[i]] : paths[legend.group[i]];
      path.moveTo(sx + r, sy);
      path.arc(sx, sy, r, 0, Math.PI * 2);
      if (data.unreleased[i]) {
        rings.moveTo(sx + r + 2, sy);
        rings.arc(sx, sy, r + 2, 0, Math.PI * 2);
      }
    }
    for (let g = 0; g < groups; g++) {
      ctx.fillStyle = colours[g] ?? colours[colours.length - 1];
      ctx.globalAlpha = 0.22;
      ctx.fill(dim[g]);
      ctx.globalAlpha = 0.9;
      ctx.fill(paths[g]);
    }
    ctx.globalAlpha = sel ? 0.4 : 0.85;
    ctx.strokeStyle = ring;
    ctx.lineWidth = 1.25;
    ctx.stroke(rings);
    ctx.globalAlpha = 1;
    for (const i of [hover.current, focus]) {
      if (i < 0 || i >= n) continue;
      const [sx, sy] = toScreen(v, w, h, data.x[i], data.y[i]);
      ctx.beginPath();
      ctx.arc(sx, sy, r + 4, 0, Math.PI * 2);
      ctx.strokeStyle = highlight;
      ctx.lineWidth = 2;
      ctx.stroke();
    }
    const l = lasso.current;
    if (l && l.length > 1) {
      ctx.beginPath();
      l.forEach(([x, y], k) => (k ? ctx.lineTo(x, y) : ctx.moveTo(x, y)));
      ctx.closePath();
      ctx.fillStyle = highlight;
      ctx.globalAlpha = 0.08;
      ctx.fill();
      ctx.globalAlpha = 1;
      ctx.setLineDash([5, 4]);
      ctx.strokeStyle = highlight;
      ctx.lineWidth = 1.5;
      ctx.stroke();
      ctx.setLineDash([]);
    }
  };

  const resize = () => {
    const el = wrap.current;
    const c = canvas.current;
    if (!el || !c) return;
    const w = el.clientWidth || FALLBACK[0];
    const h = el.clientHeight || FALLBACK[1];
    const old = size.current;
    size.current = [w, h];
    const dpr = window.devicePixelRatio || 1;
    c.width = Math.round(w * dpr);
    c.height = Math.round(h * dpr);
    c.style.width = `${w}px`;
    c.style.height = `${h}px`;
    // Keep the zoom relative to the fitted view.
    const fitted = fitView(props.current.data, w, h).scale;
    const k = fitted / (base.current || fitted);
    base.current = fitted;
    if (old[0] !== w || old[1] !== h) view.current = { ...view.current, scale: view.current.scale * k };
    draw();
  };

  // New points: index them, and fit the view the first time there are some.
  const hadPoints = useRef(false);
  useEffect(() => {
    grid.current = buildGrid(p.data);
    const [w, h] = size.current;
    const fit = fitView(p.data, w, h);
    base.current = fit.scale;
    if (!hadPoints.current && p.data.ids.length > 0) {
      view.current = fit;
      hadPoints.current = true;
    }
    hover.current = -1;
    draw();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [p.data]);

  useEffect(() => {
    draw();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [p.legend, p.colours, p.selected, p.focus, p.ring, p.highlight]);

  // Size, wheel (not passive, so the page doesn't scroll) and the api.
  useEffect(() => {
    resize();
    const el = wrap.current;
    const ro = typeof ResizeObserver !== "undefined" ? new ResizeObserver(resize) : null;
    if (el) ro?.observe(el);
    const c = canvas.current;
    const onWheel = (e: WheelEvent) => {
      e.preventDefault();
      const [w, h] = size.current;
      const rect = c!.getBoundingClientRect();
      const sx = e.clientX - rect.left;
      const sy = e.clientY - rect.top;
      // A trackpad's two-finger move pans; pinches (ctrl) and wheels zoom.
      if (!e.ctrlKey && e.deltaX !== 0) {
        view.current = { ...view.current, cx: view.current.cx + e.deltaX / view.current.scale, cy: view.current.cy - e.deltaY / view.current.scale };
      } else {
        const factor = Math.exp(-e.deltaY * (e.ctrlKey ? 0.01 : 0.0015));
        view.current = zoomAt(view.current, w, h, sx, sy, factor, base.current);
      }
      draw();
    };
    c?.addEventListener("wheel", onWheel, { passive: false });
    let anim = 0;
    const animate = (to: View) => {
      cancelAnimationFrame(anim);
      if (prefersReducedMotion()) {
        view.current = to;
        draw();
        return;
      }
      const from = view.current;
      const start = performance.now();
      const step = (now: number) => {
        const t = Math.min(1, (now - start) / 450);
        const e = 1 - Math.pow(1 - t, 3);
        // Zoom moves in ratios so it feels even.
        const scale = from.scale * Math.pow(to.scale / from.scale, e);
        view.current = { cx: from.cx + (to.cx - from.cx) * e, cy: from.cy + (to.cy - from.cy) * e, scale };
        paint();
        if (t < 1) anim = requestAnimationFrame(step);
      };
      anim = requestAnimationFrame(step);
    };
    p.api.current = {
      flyTo: (x, y, zoom) => animate({ cx: x, cy: y, scale: base.current * zoom }),
      reset: () => {
        const [w, h] = size.current;
        animate(fitView(props.current.data, w, h));
      },
    };
    return () => {
      ro?.disconnect();
      c?.removeEventListener("wheel", onWheel);
      cancelAnimationFrame(anim);
      cancelAnimationFrame(frame.current);
      frame.current = 0;
      p.api.current = null;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Pointers: one drags or draws, two pinch.
  const pointers = useRef(new Map<number, [number, number]>());
  const gesture = useRef<{ kind: "pan" | "lasso" | "pinch"; x: number; y: number; moved: boolean; dist: number } | null>(null);

  const local = (e: React.PointerEvent): [number, number] => {
    const rect = canvas.current!.getBoundingClientRect();
    return [e.clientX - rect.left, e.clientY - rect.top];
  };

  const hitAt = (sx: number, sy: number): number => {
    const g = grid.current;
    if (!g) return -1;
    const [w, h] = size.current;
    const [mx, my] = toMap(view.current, w, h, sx, sy);
    return nearest(props.current.data, g, mx, my, HIT_PX / view.current.scale);
  };

  const onPointerDown = (e: React.PointerEvent<HTMLCanvasElement>) => {
    if (e.button !== 0 && e.pointerType === "mouse") return;
    const [sx, sy] = local(e);
    pointers.current.set(e.pointerId, [sx, sy]);
    try {
      e.currentTarget.setPointerCapture?.(e.pointerId);
    } catch {
      /* a pointer the browser no longer knows: the drag still works inside the map */
    }
    if (pointers.current.size === 2) {
      const [a, b] = [...pointers.current.values()];
      lasso.current = null;
      gesture.current = { kind: "pinch", x: (a[0] + b[0]) / 2, y: (a[1] + b[1]) / 2, moved: true, dist: Math.hypot(a[0] - b[0], a[1] - b[1]) };
      return;
    }
    const kind = props.current.mode === "lasso" || e.shiftKey ? "lasso" : "pan";
    gesture.current = { kind, x: sx, y: sy, moved: false, dist: 0 };
    if (kind === "lasso") lasso.current = [[sx, sy]];
  };

  const onPointerMove = (e: React.PointerEvent<HTMLCanvasElement>) => {
    const [sx, sy] = local(e);
    const g = gesture.current;
    if (!g) {
      const i = hitAt(sx, sy);
      if (i !== hover.current) {
        hover.current = i;
        draw();
      }
      props.current.onHover(i, sx, sy);
      return;
    }
    const last = pointers.current.get(e.pointerId);
    pointers.current.set(e.pointerId, [sx, sy]);
    if (g.kind === "pinch" && pointers.current.size === 2) {
      const [a, b] = [...pointers.current.values()];
      const dist = Math.hypot(a[0] - b[0], a[1] - b[1]);
      const [w, h] = size.current;
      if (g.dist > 0) view.current = zoomAt(view.current, w, h, (a[0] + b[0]) / 2, (a[1] + b[1]) / 2, dist / g.dist, base.current);
      g.dist = dist;
      draw();
      return;
    }
    if (!g.moved && Math.hypot(sx - g.x, sy - g.y) < DRAG_PX) return;
    g.moved = true;
    if (g.kind === "pan" && last) {
      const v = view.current;
      view.current = { ...v, cx: v.cx - (sx - last[0]) / v.scale, cy: v.cy + (sy - last[1]) / v.scale };
      if (hover.current !== -1) {
        hover.current = -1;
        props.current.onHover(-1, sx, sy);
      }
    } else if (g.kind === "lasso") {
      const l = lasso.current!;
      const [lx, ly] = l[l.length - 1];
      if (Math.hypot(sx - lx, sy - ly) >= 3) l.push([sx, sy]);
    }
    draw();
  };

  const onPointerUp = (e: React.PointerEvent<HTMLCanvasElement>) => {
    const [sx, sy] = local(e);
    pointers.current.delete(e.pointerId);
    const g = gesture.current;
    if (!g) return;
    if (g.kind === "pinch") {
      if (pointers.current.size === 0) gesture.current = null;
      return;
    }
    gesture.current = null;
    if (g.kind === "lasso") {
      const l = lasso.current ?? [];
      lasso.current = null;
      if (g.moved && l.length >= 3) {
        const [w, h] = size.current;
        const poly = l.map(([x, y]) => toMap(view.current, w, h, x, y));
        props.current.onLasso(lassoSelect(props.current.data, poly));
      }
      draw();
      return;
    }
    if (!g.moved) {
      const i = hitAt(sx, sy);
      if (i >= 0) props.current.onPick(i);
    }
  };

  const onKeyDown = (e: React.KeyboardEvent) => {
    const [w, h] = size.current;
    const v = view.current;
    const step = 60 / v.scale;
    const keys: Record<string, () => View> = {
      ArrowLeft: () => ({ ...v, cx: v.cx - step }),
      ArrowRight: () => ({ ...v, cx: v.cx + step }),
      ArrowUp: () => ({ ...v, cy: v.cy + step }),
      ArrowDown: () => ({ ...v, cy: v.cy - step }),
      "+": () => zoomAt(v, w, h, w / 2, h / 2, 1.4, base.current),
      "=": () => zoomAt(v, w, h, w / 2, h / 2, 1.4, base.current),
      "-": () => zoomAt(v, w, h, w / 2, h / 2, 1 / 1.4, base.current),
      "0": () => fitView(props.current.data, w, h),
    };
    if (e.key === "Escape") {
      props.current.onEscape();
      return;
    }
    const f = keys[e.key];
    if (!f) return;
    e.preventDefault();
    view.current = f();
    draw();
  };

  return (
    <div ref={wrap} className="soundmap__canvas-wrap">
      <canvas
        ref={canvas}
        className={`soundmap__canvas soundmap__canvas--${p.mode}`}
        role="img"
        aria-label={p.label}
        tabIndex={0}
        data-testid="soundmap-canvas"
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={onPointerUp}
        onPointerCancel={onPointerUp}
        onPointerLeave={() => {
          if (gesture.current) return;
          hover.current = -1;
          props.current.onHover(-1, 0, 0);
          draw();
        }}
        onKeyDown={onKeyDown}
      />
    </div>
  );
}
