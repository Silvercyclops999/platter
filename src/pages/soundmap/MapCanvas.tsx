/**
 * The sound map itself: a canvas with every song as a dot. Wheel or pinch
 * zooms around the pointer, dragging moves the map (or draws a lasso in
 * lasso mode, or with Shift held), clicking a dot plays it, and keys move,
 * zoom and step from song to song when the map has focus. Drawing happens
 * once per frame and only when something changed.
 */
import { useEffect, useLayoutEffect, useRef, type MutableRefObject } from "react";
import { prefersReducedMotion } from "../../design/motion";
import {
  buildGrid,
  dotRadius,
  fitView,
  lassoSelect,
  nearest,
  nearestToward,
  rebase,
  toMap,
  toScreen,
  wheelPixels,
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
  /** How far in the map is zoomed, relative to the fitted view. */
  zoom(): number;
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
  /** The song in focus (found, clicked or stepped to), or -1. */
  focus: number;
  api: MutableRefObject<MapApi | null>;
  label: string;
  onHover: (i: number, sx: number, sy: number) => void;
  onPick: (i: number) => void;
  onLasso: (indices: number[]) => void;
  onEscape: () => void;
  /** Keys moved the focus to another song. */
  onFocus: (i: number) => void;
  /** Enter on the song in focus. */
  onPlay: (i: number) => void;
}

/** Fallback size where layout gives none (tests). */
const FALLBACK: [number, number] = [800, 600];
const HIT_PX = 8;
const DRAG_PX = 4;
/** A song stepped to by keys is brought into view when it's nearer the edge than this. */
const EDGE_PX = 40;

const ARROWS: Record<string, [number, number]> = {
  ArrowLeft: [-1, 0],
  ArrowRight: [1, 0],
  ArrowUp: [0, 1],
  ArrowDown: [0, -1],
};

/** The dots' paths for one view, kept so a redraw for the hover ring alone doesn't rebuild them. */
interface Paths {
  key: unknown[];
  paths: Path2D[];
  dim: Path2D[];
  rings: Path2D;
  r: number;
}

export function MapCanvas(p: Props) {
  const wrap = useRef<HTMLDivElement>(null);
  const canvas = useRef<HTMLCanvasElement>(null);
  const size = useRef<[number, number]>(FALLBACK);
  const dprRef = useRef(0);
  const view = useRef<View>({ cx: 0, cy: 0, scale: 1 });
  const base = useRef(1);
  const grid = useRef<Grid | null>(null);
  const lasso = useRef<[number, number][] | null>(null);
  const hover = useRef(-1);
  const frame = useRef(0);
  const flight = useRef(0);
  const cache = useRef<Paths | null>(null);
  const props = useRef(p);
  props.current = p;

  const draw = () => {
    if (frame.current) return;
    frame.current = requestAnimationFrame(() => {
      frame.current = 0;
      paint();
    });
  };

  /** Any input of the user's stops a flight under way. */
  const stopFlight = () => {
    if (flight.current) cancelAnimationFrame(flight.current);
    flight.current = 0;
  };

  const buildPaths = (w: number, h: number): Paths => {
    const { data, legend, selected } = props.current;
    const v = view.current;
    const key = [data, legend, selected, v.cx, v.cy, v.scale, w, h, base.current];
    const old = cache.current;
    if (old && old.key.length === key.length && old.key.every((k, i) => k === key[i])) return old;
    const r = dotRadius(v, base.current);
    const groups = legend.groups.length;
    // One path per colour (and per dimmed or not), so a big library is a few fills.
    const paths: Path2D[] = [];
    const dim: Path2D[] = [];
    const rings = new Path2D();
    for (let g = 0; g < groups; g++) {
      paths.push(new Path2D());
      dim.push(new Path2D());
    }
    const { cx, cy, scale } = v;
    const hw = w / 2;
    const hh = h / 2;
    // What's on screen (with a margin), in map units, so the rest is skipped before any sums.
    const x0 = cx - (hw + 10) / scale;
    const x1 = cx + (hw + 10) / scale;
    const y0 = cy - (hh + 10) / scale;
    const y1 = cy + (hh + 10) / scale;
    const xs = data.x;
    const ys = data.y;
    const group = legend.group;
    const unreleased = data.unreleased;
    const n = data.ids.length;
    const tau = Math.PI * 2;
    for (let i = 0; i < n; i++) {
      const x = xs[i];
      if (x < x0 || x > x1) continue;
      const y = ys[i];
      if (y < y0 || y > y1) continue;
      const sx = hw + (x - cx) * scale;
      const sy = hh - (y - cy) * scale;
      const path = selected && !selected[i] ? dim[group[i]] : paths[group[i]];
      path.moveTo(sx + r, sy);
      path.arc(sx, sy, r, 0, tau);
      if (unreleased[i]) {
        rings.moveTo(sx + r + 2, sy);
        rings.arc(sx, sy, r + 2, 0, tau);
      }
    }
    const built = { key, paths, dim, rings, r };
    cache.current = built;
    return built;
  };

  const paint = () => {
    const c = canvas.current;
    const ctx = c?.getContext?.("2d");
    if (!c || !ctx) return;
    const { data, legend, colours, ring, highlight, selected, focus } = props.current;
    const [w, h] = size.current;
    const dpr = dprRef.current || window.devicePixelRatio || 1;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, w, h);
    const v = view.current;
    const n = data.ids.length;
    const { paths, dim, rings, r } = buildPaths(w, h);
    for (let g = 0; g < legend.groups.length; g++) {
      ctx.fillStyle = colours[g] ?? colours[colours.length - 1];
      ctx.globalAlpha = 0.22;
      ctx.fill(dim[g]);
      ctx.globalAlpha = 0.9;
      ctx.fill(paths[g]);
    }
    ctx.globalAlpha = selected ? 0.4 : 0.85;
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
    const dpr = window.devicePixelRatio || 1;
    const [ow, oh] = size.current;
    const sameSize = ow === w && oh === h;
    // Nothing moved: keep the canvas (setting its size clears and reallocates it).
    if (sameSize && dprRef.current === dpr) return;
    size.current = [w, h];
    dprRef.current = dpr;
    c.width = Math.round(w * dpr);
    c.height = Math.round(h * dpr);
    c.style.width = `${w}px`;
    c.style.height = `${h}px`;
    // Keep the zoom relative to the fitted view.
    const fitted = fitView(props.current.data, w, h).scale;
    if (!sameSize) view.current = rebase(view.current, base.current, fitted);
    base.current = fitted;
    cache.current = null;
    paint();
  };

  // New points: index them, fit the view the first time there are some,
  // and after that keep the zoom where it was relative to the fitted view.
  // Before the points are on screen, so a key pressed as they appear finds them.
  const hadPoints = useRef(false);
  useLayoutEffect(() => {
    grid.current = buildGrid(p.data);
    const [w, h] = size.current;
    const fit = fitView(p.data, w, h);
    if (!hadPoints.current && p.data.ids.length > 0) {
      view.current = fit;
      hadPoints.current = true;
    } else {
      view.current = rebase(view.current, base.current, fit.scale);
    }
    base.current = fit.scale;
    hover.current = -1;
    draw();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [p.data]);

  useEffect(() => {
    draw();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [p.legend, p.colours, p.selected, p.focus, p.ring, p.highlight]);

  // Size, screen density, wheel (not passive, so the page doesn't scroll) and the api,
  // also in place before the map is on screen.
  useLayoutEffect(() => {
    resize();
    const el = wrap.current;
    const ro = typeof ResizeObserver !== "undefined" ? new ResizeObserver(resize) : null;
    if (el) ro?.observe(el);
    // Moving to a screen with another density: a query for the density now, asked again each time it changes.
    let mq: MediaQueryList | null = null;
    const onDensity = () => {
      resize();
      watchDensity();
    };
    const watchDensity = () => {
      mq?.removeEventListener?.("change", onDensity);
      mq = typeof window.matchMedia === "function" ? window.matchMedia(`(resolution: ${window.devicePixelRatio || 1}dppx)`) : null;
      mq?.addEventListener?.("change", onDensity);
    };
    watchDensity();
    const c = canvas.current;
    const onWheel = (e: WheelEvent) => {
      e.preventDefault();
      stopFlight();
      const [w, h] = size.current;
      const rect = c!.getBoundingClientRect();
      const sx = e.clientX - rect.left;
      const sy = e.clientY - rect.top;
      const dx = wheelPixels(e.deltaX, e.deltaMode, w);
      const dy = wheelPixels(e.deltaY, e.deltaMode, h);
      // A trackpad's two-finger move pans; pinches (ctrl) and wheels zoom.
      if (!e.ctrlKey && dx !== 0) {
        view.current = { ...view.current, cx: view.current.cx + dx / view.current.scale, cy: view.current.cy - dy / view.current.scale };
      } else {
        const factor = Math.exp(-dy * (e.ctrlKey ? 0.01 : 0.0015));
        view.current = zoomAt(view.current, w, h, sx, sy, factor, base.current);
      }
      draw();
    };
    c?.addEventListener("wheel", onWheel, { passive: false });
    const animate = (to: View) => {
      stopFlight();
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
        flight.current = t < 1 ? requestAnimationFrame(step) : 0;
      };
      flight.current = requestAnimationFrame(step);
    };
    p.api.current = {
      flyTo: (x, y, zoom) => animate({ cx: x, cy: y, scale: base.current * zoom }),
      reset: () => {
        const [w, h] = size.current;
        animate(fitView(props.current.data, w, h));
      },
      zoom: () => view.current.scale / (base.current || 1),
    };
    return () => {
      ro?.disconnect();
      mq?.removeEventListener?.("change", onDensity);
      c?.removeEventListener("wheel", onWheel);
      stopFlight();
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
    stopFlight();
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
    } else if (g.kind === "lasso" && lasso.current) {
      const l = lasso.current;
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

  // The system took the pointer away (a scroll, a gesture, the window losing it): drop what was under way.
  const onPointerCancel = (e: React.PointerEvent<HTMLCanvasElement>) => {
    pointers.current.clear();
    gesture.current = null;
    lasso.current = null;
    try {
      e.currentTarget.releasePointerCapture?.(e.pointerId);
    } catch {
      /* already let go */
    }
    draw();
  };

  /** Brings a song stepped to near the edge (or off it) to the middle. */
  const keepInView = (i: number) => {
    const { data } = props.current;
    const [w, h] = size.current;
    const [sx, sy] = toScreen(view.current, w, h, data.x[i], data.y[i]);
    if (sx < EDGE_PX || sy < EDGE_PX || sx > w - EDGE_PX || sy > h - EDGE_PX) view.current = { ...view.current, cx: data.x[i], cy: data.y[i] };
  };

  const onKeyDown = (e: React.KeyboardEvent) => {
    const { data, focus } = props.current;
    const [w, h] = size.current;
    const v = view.current;
    if (e.key === "Escape") {
      props.current.onEscape();
      return;
    }
    if (e.key === "Enter") {
      e.preventDefault();
      if (focus >= 0 && focus < data.ids.length) {
        props.current.onPlay(focus);
        return;
      }
      // Nothing in focus yet: the song nearest the middle comes into focus.
      const g = grid.current;
      if (!g || data.ids.length === 0) return;
      let i = -1;
      for (let radius = 20; i < 0 && radius <= Math.max(w, h) * 2; radius *= 2) i = nearest(data, g, v.cx, v.cy, radius / v.scale);
      if (i >= 0) {
        stopFlight();
        keepInView(i);
        props.current.onFocus(i);
        draw();
      }
      return;
    }
    // Zoom keys with Ctrl or Cmd belong to the app (or the system), not the map.
    if ((e.ctrlKey || e.metaKey) && ["+", "=", "-", "0"].includes(e.key)) return;
    const arrow = ARROWS[e.key];
    if (arrow && focus >= 0 && focus < data.ids.length && !e.shiftKey) {
      // Step to the nearest song that way.
      e.preventDefault();
      stopFlight();
      const j = nearestToward(data, focus, arrow[0], arrow[1]);
      if (j >= 0) {
        keepInView(j);
        props.current.onFocus(j);
        draw();
      }
      return;
    }
    const step = 60 / v.scale;
    const keys: Record<string, () => View> = {
      ArrowLeft: () => ({ ...v, cx: v.cx - step }),
      ArrowRight: () => ({ ...v, cx: v.cx + step }),
      ArrowUp: () => ({ ...v, cy: v.cy + step }),
      ArrowDown: () => ({ ...v, cy: v.cy - step }),
      "+": () => zoomAt(v, w, h, w / 2, h / 2, 1.4, base.current),
      "=": () => zoomAt(v, w, h, w / 2, h / 2, 1.4, base.current),
      "-": () => zoomAt(v, w, h, w / 2, h / 2, 1 / 1.4, base.current),
      "0": () => fitView(data, w, h),
    };
    const f = keys[e.key];
    if (!f) return;
    e.preventDefault();
    stopFlight();
    view.current = f();
    draw();
  };

  return (
    <div ref={wrap} className="soundmap__canvas-wrap">
      <canvas
        ref={canvas}
        className={`soundmap__canvas soundmap__canvas--${p.mode}`}
        role="application"
        aria-roledescription="map"
        aria-label={p.label}
        tabIndex={0}
        data-testid="soundmap-canvas"
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={onPointerUp}
        onPointerCancel={onPointerCancel}
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
