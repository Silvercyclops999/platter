/**
 * The visualizer: fills its box (the whole now playing screen on the phone,
 * the expanded view on the computer) with the playing cover's colours,
 * moving with the music. Three styles:
 *
 * - Grooves: a record in the cover's colours with the cover as its label,
 *   rings running out on the beat (WebGL2).
 * - Blur: the cover's colours as large soft shapes drifting round the cover
 *   and swelling with the low end; the cover small and sharp (WebGL2).
 * - Bars: one row of rounded bars where the cover sits, in its lead colour,
 *   over the cover blurred and dimmed (Canvas 2D).
 *
 * Without WebGL2 (or after the GPU context is lost) Grooves and Blur are
 * drawn with a few CSS layers instead. The sharp cover is the page's own
 * element on top: the stage measures where it sits so the art gathers round
 * it, and writes its breath (and the record's turn) as custom properties.
 * Everything per frame is imperative; React only hears about the renderer.
 */
import { useEffect, useLayoutEffect, useRef, useState, type CSSProperties } from "react";
import { Art } from "../components/Art";
import { artUrl } from "../lib/backend";
import { hslToRgb, parseColour, rgbToHsl, type RGB } from "../lib/color";
import type { VisualizerStyle } from "../lib/types";
import { useCoverColors } from "../shell/nowPlaying";
import { usePlayer } from "../stores/player";
import { useSettings } from "../stores/settings";
import { ArtGL, type ArtFrame } from "./art/ArtGL";
import { useFrameLoop, useReducedMotion } from "./draw";
import { latestFrame, useFeed, useSpectrumFeed } from "./feed";
import { paletteFromColour, paletteFromImage, type ArtPalette } from "./palette";
import { BarLevels, current } from "./scale";
import { MOTION, MusicSignals } from "./signals";
import { DESKTOP_BARS, DESKTOP_FPS, PAUSED_FPS, PHONE_BARS, PHONE_FPS, STYLES, styleLabel } from "./styles";
import "./stage.css";

/** A phone draws at most this share of its screen's pixels: the art is soft, the battery isn't free. */
export const PHONE_RENDER_CAP = 0.7;
/** And never more pixels than this, on any screen. */
export const MAX_RENDER_PIXELS = 2_600_000;
/** The record turns at 33 1/3 a minute while playing (degrees a second). */
const SPIN_DEG_S = 200;
/** How often the stage looks again at where the sharp cover sits (ms). */
const MEASURE_MS = 1_000;
/** Under reduced motion, frames are drawn for this long after something changes, then none. */
const STILL_AWAKE_MS = 1_400;
/** The cover is read at this size for its palette. */
const PALETTE_ART = 96;
/** The style's name shows this long after opening or a change. */
export const NAME_MS = 1_600;
/** The softer shapes need fewer pixels than the record's fine grooves. */
const DETAIL: Record<VisualizerStyle, number> = { grooves: 1, blur: 0.5, bars: 1 };

/** The drawing buffer's scale against CSS pixels for a style on this screen. */
export function renderScale(style: VisualizerStyle, dpr: number, phone: boolean, cssW: number, cssH: number): number {
  let s = Math.min(2, dpr) * DETAIL[style];
  if (phone) s = Math.min(s, dpr * PHONE_RENDER_CAP);
  const px = cssW * cssH * s * s;
  if (px > MAX_RENDER_PIXELS) s *= Math.sqrt(MAX_RENDER_PIXELS / px);
  return Math.max(0.25, s);
}

/** The bars' colour: the cover's lead, lifted so it reads over the dimmed cover. */
export function barColours(lead: RGB): { top: string; bottom: string } {
  const [h, s] = rgbToHsl([lead[0] * 255, lead[1] * 255, lead[2] * 255]);
  const css = (c: RGB, a: number) => `rgba(${c.map((v) => Math.round(v)).join(", ")}, ${a})`;
  const sat = Math.min(0.72, Math.max(0.3, s));
  return { top: css(hslToRgb(h, sat * 0.7, 0.86), 0.96), bottom: css(hslToRgb(h, sat, 0.64), 0.92) };
}

/** Why the art holds still, if it does. */
export function stillLine(local: boolean, reduced: boolean, resting = false, phone = false): string | null {
  if (!local) return "The visualizer moves with music playing on this device.";
  if (reduced) return "Holding still while Reduce motion is on.";
  if (resting) return phone ? "Resting while the phone is in Low Power Mode." : "Resting while on battery power.";
  return null;
}

export interface StageProps {
  style: VisualizerStyle;
  phone?: boolean;
  /** The sharp cover on the page: the art gathers round it and sets its breath. */
  cover: () => HTMLElement | null;
}

type Renderer = "webgl2" | "css";
interface Box {
  /** Drawing buffer size and its scale against CSS pixels. */
  w: number;
  h: number;
  scale: number;
  /** The cover in buffer pixels: centre x, y and half its side. */
  anchor: [number, number, number];
  /** The cover's resting box in buffer pixels. */
  rect: { x: number; y: number; w: number; h: number };
}

export function Stage({ style, phone = false, cover }: StageProps) {
  const thumb = usePlayer((s) => s.state?.track?.thumb ?? null);
  const playing = usePlayer((s) => s.state?.status === "playing" || s.state?.status === "buffering" || s.state?.status === "loading");
  const local = usePlayer((s) => s.remote === null && !!s.state?.track);
  const colourOn = useSettings((s) => s.settings?.artworkColours !== false);
  const vivid = useCoverColors(colourOn ? thumb : null)?.vivid ?? null;
  const reduced = useReducedMotion();
  const fps = phone ? PHONE_FPS : DESKTOP_FPS;
  // Nothing to move with under reduced motion, or for music playing elsewhere, so nothing is asked of the core.
  useSpectrumFeed(reduced || !local ? null : { fps, scope: false });
  // On battery (a phone in Low Power Mode) the core rests: the art holds still too, and the
  // watch is still renewed so it moves again once the power comes back.
  const resting = useFeed((s) => s.resting);
  const still = reduced || !local || resting;

  const root = useRef<HTMLDivElement>(null);
  const glHost = useRef<HTMLDivElement>(null);
  const glCanvas = useRef<HTMLCanvasElement | null>(null);
  const barCanvas = useRef<HTMLCanvasElement>(null);
  const glRef = useRef<ArtGL | null>(null);
  const [renderer, setRenderer] = useState<Renderer | null>(null);
  const signals = useRef(new MusicSignals());
  const bars = useRef(new BarLevels(phone ? PHONE_BARS : DESKTOP_BARS));
  const box = useRef<Box>({ w: 1, h: 1, scale: 1, anchor: [0, 0, 0], rect: { x: 0, y: 0, w: 0, h: 0 } });
  const palette = useRef<ArtPalette>(paletteFromColour(null));
  const barFill = useRef(barColours(palette.current.colours[0]));
  const spin = useRef(0);
  const spinSpeed = useRef(0);
  const clock = useRef(0);
  const coverFn = useRef(cover);
  useLayoutEffect(() => {
    coverFn.current = cover;
  });

  // Holding still, frames are drawn only for a moment after something changes.
  const [awake, setAwake] = useState(true);
  const sleepTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const poke = useRef(() => {});
  poke.current = () => {
    if (!still) return;
    setAwake(true);
    if (sleepTimer.current) clearTimeout(sleepTimer.current);
    sleepTimer.current = setTimeout(() => setAwake(false), STILL_AWAKE_MS);
  };
  useEffect(() => {
    poke.current();
    return () => {
      if (sleepTimer.current) clearTimeout(sleepTimer.current);
    };
  }, [still, renderer, style]);

  // The GPU: both programs on one canvas of this mount's own, given back when it goes. A lost
  // context draws with the CSS layers until it returns.
  useLayoutEffect(() => {
    const host = glHost.current;
    if (!host || typeof document === "undefined") return;
    const c = document.createElement("canvas");
    c.className = "vis-stage__canvas";
    host.appendChild(c);
    glCanvas.current = c;
    const r = ArtGL.create(c);
    glRef.current = r;
    setRenderer(r ? "webgl2" : "css");
    const lost = (e: Event) => {
      e.preventDefault();
      if (glRef.current) glRef.current.lost = true;
      setRenderer("css");
    };
    const restored = () => {
      const g = glRef.current;
      if (!g) return;
      g.restore();
      // A GPU that can't build the art again stays on the CSS layers rather than going black.
      if (!g.ready()) return;
      g.setPalette(palette.current.colours, palette.current.base, performance.now(), 0);
      setRenderer("webgl2");
    };
    c.addEventListener("webglcontextlost", lost);
    c.addEventListener("webglcontextrestored", restored);
    return () => {
      c.removeEventListener("webglcontextlost", lost);
      c.removeEventListener("webglcontextrestored", restored);
      glRef.current?.dispose();
      glRef.current = null;
      c.remove();
      glCanvas.current = null;
    };
  }, []);
  const onGpu = renderer === "webgl2" && style !== "bars";
  useLayoutEffect(() => {
    if (glCanvas.current) glCanvas.current.hidden = !onGpu;
  }, [onGpu]);

  // The palette, eased toward on the GPU and shown on the CSS layers and the bars.
  const applyPalette = (p: ArtPalette) => {
    palette.current = p;
    barFill.current = barColours(p.colours[0]);
    glRef.current?.setPalette(p.colours, p.base, performance.now());
    poke.current();
    const el = root.current;
    if (!el) return;
    p.colours.forEach((c, i) => el.style.setProperty(`--vis-c${i}`, `rgb(${c.map((v) => Math.round(v * 255)).join(" ")})`));
    el.style.setProperty("--vis-base", `rgb(${p.base.map((v) => Math.round(v * 255)).join(" ")})`);
  };

  // The palette: from the cover itself once it has been read, else from now playing's own colour.
  const read = useRef<{ url: string; palette: ArtPalette | null } | null>(null);
  const fallback = () => paletteFromColour(vivid ? parseColour(vivid) : null);
  // Only this song's cover counts: a song with none (or one that won't load) never keeps the last one's colours.
  const coverPalette = () => {
    const url = thumb ? artUrl(thumb, PALETTE_ART) : null;
    return colourOn && url && read.current?.url === url ? read.current.palette : null;
  };
  useEffect(() => {
    applyPalette(coverPalette() ?? fallback());
  }, [vivid, colourOn]); // eslint-disable-line react-hooks/exhaustive-deps
  useEffect(() => {
    const url = thumb ? artUrl(thumb, PALETTE_ART) : undefined;
    if (!url || typeof Image === "undefined" || !colourOn) {
      read.current = null;
      applyPalette(fallback());
      return;
    }
    if (read.current?.url === url) {
      applyPalette(read.current.palette ?? fallback());
      return;
    }
    let alive = true;
    const img = new Image();
    img.crossOrigin = "anonymous";
    img.decoding = "async";
    img.onload = () => {
      if (!alive) return;
      const p = paletteFromImage(img);
      read.current = { url, palette: p };
      applyPalette(p ?? fallback());
    };
    img.onerror = () => {
      if (!alive) return;
      read.current = { url, palette: null };
      applyPalette(fallback());
    };
    img.src = url;
    return () => {
      alive = false;
    };
  }, [thumb, colourOn, renderer]); // eslint-disable-line react-hooks/exhaustive-deps

  // The buffers follow the box, and where the sharp cover sits is read now and then (never per frame).
  useEffect(() => {
    const el = root.current;
    if (!el) return;
    const measure = () => {
      const r = el.getBoundingClientRect();
      const dpr = typeof window !== "undefined" ? window.devicePixelRatio || 1 : 1;
      const scale = renderScale(style, dpr, phone, r.width, r.height);
      const w = Math.max(1, Math.round(r.width * scale));
      const h = Math.max(1, Math.round(r.height * scale));
      const c = style === "bars" ? barCanvas.current : glCanvas.current;
      const resized = !!c && (c.width !== w || c.height !== h);
      if (c && c.width !== w) c.width = w;
      if (c && c.height !== h) c.height = h;
      const before = box.current.anchor;
      const target = coverFn.current();
      let rect = { x: r.width * 0.15, y: r.height * 0.2, w: r.width * 0.7, h: r.width * 0.7 };
      if (target) {
        // The centre from the box on screen (turning and breathing keep it), the size from the
        // layout box, which neither the record's turn nor the breath changes.
        const t = target.getBoundingClientRect();
        const cw = target.offsetWidth || t.width;
        const ch = target.offsetHeight || t.height;
        rect = { x: t.left + t.width / 2 - cw / 2 - r.left, y: t.top + t.height / 2 - ch / 2 - r.top, w: cw, h: ch };
      }
      const anchor: Box["anchor"] = [(rect.x + rect.w / 2) * scale, (rect.y + rect.h / 2) * scale, (Math.min(rect.w, rect.h) / 2) * scale];
      box.current = { w, h, scale, anchor, rect: { x: rect.x * scale, y: rect.y * scale, w: rect.w * scale, h: rect.h * scale } };
      // Where the cover sits, for the CSS layers and the lines under it.
      el.style.setProperty("--vis-cx", `${(rect.x + rect.w / 2).toFixed(1)}px`);
      el.style.setProperty("--vis-cy", `${(rect.y + rect.h / 2).toFixed(1)}px`);
      el.style.setProperty("--vis-side", `${Math.min(rect.w, rect.h).toFixed(1)}px`);
      el.style.setProperty("--vis-under", `${(rect.y + rect.h).toFixed(1)}px`);
      if (resized || anchor.some((v, i) => Math.abs(v - before[i]) > 1)) poke.current();
    };
    measure();
    // The cover moves with layout (a skip, a pause settling it back), so it is looked at once a second too.
    const every = setInterval(measure, MEASURE_MS);
    let ro: ResizeObserver | null = null;
    if (typeof ResizeObserver !== "undefined") {
      ro = new ResizeObserver(measure);
      ro.observe(el);
      const target = coverFn.current();
      if (target) ro.observe(target);
    }
    return () => {
      clearInterval(every);
      ro?.disconnect();
    };
  }, [style, phone, renderer]);

  // The cover's breath and turn are the stage's while it shows; it gives them back when it goes
  // (or when another cover takes its place, as after a skip).
  const touched = useRef<HTMLElement | null>(null);
  useEffect(() => {
    return () => {
      giveBack(touched.current);
      touched.current = null;
    };
  }, [style]);

  const frame = useRef<ArtFrame>({
    time: 0,
    spin: 0,
    low: 0,
    mid: 0,
    high: 0,
    pulse: 0,
    drift: 0,
    flow: 0,
    label: [0, 0, 0],
    beatAt: signals.current.beatAt,
    beatStrength: signals.current.beatStrength,
  }).current;
  const fields = useRef<(HTMLElement | null)[]>([]);

  useFrameLoop(
    (dt, now) => {
      clock.current += dt / 1000;
      const sig = signals.current;
      const motion = still ? 0 : 1;
      let bands: ArrayLike<number> | null = null;
      if (!still) {
        const { frame: f, at } = latestFrame();
        const cur = current(f, at, now);
        bands = cur?.bands ?? null;
        sig.step(bands, cur, dt, clock.current * 1000);
      }
      // The record speeds up and slows down like a platter, never jumping.
      const wantSpin = playing && !still && style === "grooves" ? SPIN_DEG_S : 0;
      spinSpeed.current += (wantSpin - spinSpeed.current) * (1 - Math.exp(-dt / 700));
      spin.current = (spin.current + (spinSpeed.current * dt) / 1000) % 360;

      const el = coverFn.current();
      if (el !== touched.current) {
        giveBack(touched.current);
        touched.current = el;
      }
      if (el && style !== "bars") {
        const breath = 1 + motion * (0.03 * sig.pulse + 0.012 * sig.low * MOTION.gain);
        el.style.setProperty("--vis-breath", breath.toFixed(4));
        if (style === "grooves") el.style.setProperty("--vis-spin", `${spin.current.toFixed(2)}deg`);
      }

      const b = box.current;
      if (style === "bars") {
        bars.current.step(bands, dt);
        const c = barCanvas.current;
        const ctx = c?.getContext("2d");
        if (c && ctx) drawBars(ctx, bars.current.shown, b, barFill.current, sig.pulse * motion);
      } else if (renderer === "webgl2" && glRef.current && !glRef.current.lost) {
        frame.time = clock.current;
        frame.spin = (spin.current * Math.PI) / 180;
        frame.low = sig.low;
        frame.mid = sig.mid;
        frame.high = sig.high;
        frame.pulse = sig.pulse;
        frame.drift = sig.drift;
        frame.flow = sig.flow;
        frame.label = b.anchor;
        glRef.current.draw(style, frame, now, b.w, b.h);
      } else if (renderer === "css") {
        moveFields(fields.current, sig, motion);
      }
    },
    // Paused, the art rests at a slower rate; holding still it draws only after a change.
    playing ? fps : Math.min(fps, PAUSED_FPS),
    renderer !== null && (!still || awake),
  );

  const line = stillLine(local, reduced, resting, phone);
  const cssArt = renderer === "css" && style !== "bars";
  return (
    <div
      ref={root}
      className={`vis-stage vis-stage--${style}`}
      data-testid="vis-stage"
      data-renderer={renderer ?? undefined}
      data-style={style}
      data-still={still || undefined}
      aria-hidden="true"
    >
      {(style === "bars" || cssArt) && (
        <div className="vis-stage__backdrop" style={{ opacity: thumb ? undefined : 0 } as CSSProperties}>
          {thumb && <Art path={thumb} fluid requestSize={160} />}
        </div>
      )}
      <div ref={glHost} className="vis-stage__gl" />
      {cssArt && (
        <div className="vis-css">
          {style === "grooves" && <div className="vis-css__grooves" />}
          {[0, 1, 2, 3].map((i) => (
            <i key={i} ref={(n) => void (fields.current[i] = n)} className={`vis-css__field vis-css__field--${i}`} />
          ))}
        </div>
      )}
      {style === "bars" && <canvas ref={barCanvas} className="vis-stage__canvas vis-stage__bars" />}
      <div className="vis-stage__scrim vis-stage__scrim--top" />
      <div className="vis-stage__scrim vis-stage__scrim--bottom" />
      {line && <p className="vis-stage__line">{line}</p>}
    </div>
  );
}

/**
 * The style's name and where it sits among the three, for a moment after
 * opening or a change. The view places it over its cover (the stage is
 * behind the cover, so it can't).
 */
export function VisName({ style }: { style: VisualizerStyle }) {
  return <StyleName key={style} style={style} />;
}

function StyleName({ style }: { style: VisualizerStyle }) {
  // Keyed by the style, so a change starts it afresh.
  const [on, setOn] = useState(true);
  useEffect(() => {
    const t = setTimeout(() => setOn(false), NAME_MS);
    return () => clearTimeout(t);
  }, []);
  return (
    <div className={`vis-name ${on ? "is-on" : ""}`} data-testid="vis-name" aria-hidden="true">
      <span>{styleLabel(style)}</span>
      <span className="vis-name__dots">
        {STYLES.map((s) => (
          <i key={s.id} className={s.id === style ? "is-on" : ""} />
        ))}
      </span>
    </div>
  );
}

const COVER_PROPS = ["--vis-breath", "--vis-spin"];

/** Takes the stage's custom properties off a cover. */
function giveBack(el: HTMLElement | null) {
  if (el) for (const k of COVER_PROPS) el.style.removeProperty(k);
}

/** The CSS layers' colour fields: the Blur shader's paths, as transforms. */
function moveFields(fields: (HTMLElement | null)[], sig: MusicSignals, motion: number) {
  const dr = sig.drift;
  for (let i = 0; i < fields.length; i++) {
    const el = fields[i];
    if (!el) continue;
    const a = dr * (0.23 + 0.06 * i) * motion + i * 1.5708;
    const x = Math.cos(a) * (28 + 6 * i);
    const y = Math.sin(a * 1.17 + i) * (34 + 5 * i);
    const s = 1 + motion * (0.24 * sig.low + 0.07 * sig.pulse);
    el.style.transform = `translate(${x.toFixed(2)}%, ${y.toFixed(2)}%) scale(${s.toFixed(3)})`;
  }
}

/**
 * Bars: one row across the cover's place, rising from its bottom edge.
 * Rounded, a little gap between each, never quite empty (a dot at rest).
 */
export function drawBars(ctx: CanvasRenderingContext2D, levels: Float32Array, b: Box, fill: { top: string; bottom: string }, pulse: number) {
  const { w, h, rect } = b;
  ctx.clearRect(0, 0, w, h);
  const n = levels.length;
  if (!n || rect.w <= 0) return;
  const gapShare = 0.45;
  const bw = rect.w / (n + (n - 1) * gapShare);
  const step = bw * (1 + gapShare);
  const floor = rect.y + rect.h;
  const grad = ctx.createLinearGradient(0, rect.y, 0, floor);
  grad.addColorStop(0, fill.top);
  grad.addColorStop(1, fill.bottom);
  ctx.fillStyle = grad;
  ctx.globalAlpha = Math.min(1, 0.86 + 0.14 * pulse);
  ctx.beginPath();
  for (let i = 0; i < n; i++) {
    const bh = Math.max(bw, levels[i] * rect.h);
    const x = rect.x + i * step;
    const y = floor - bh;
    const r = bw / 2;
    if (typeof ctx.roundRect === "function") ctx.roundRect(x, y, bw, bh, r);
    else ctx.rect(x, y, bw, bh);
  }
  ctx.fill();
  ctx.globalAlpha = 1;
}
