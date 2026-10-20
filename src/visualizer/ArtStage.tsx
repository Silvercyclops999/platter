/**
 * The art styles' stage: fills its box (the whole now playing screen on
 * the phone, the expanded view on the computer) with the playing cover and
 * its colours, moving with the music. Drawn by WebGL2 on one quad; without
 * it (or after the GPU context is lost) Sleeve is drawn with a few
 * composited layers instead. The sharp cover is the page's own element on
 * top: the stage measures where it sits so the art gathers round it, and
 * writes its breath (and for Light, the light on it) as custom properties.
 * Everything per frame is imperative; React only hears about the renderer.
 */
import { useEffect, useLayoutEffect, useRef, useState, type CSSProperties } from "react";
import { Art } from "../components/Art";
import { artUrl } from "../lib/backend";
import { parseColour } from "../lib/color";
import type { VisualizerIntensity } from "../lib/types";
import { useCoverColors } from "../shell/nowPlaying";
import { usePlayer } from "../stores/player";
import { useSettings } from "../stores/settings";
import { ArtGL, TEXTURE_SIZE, type ArtFrame } from "./art/ArtGL";
import { DETAIL, leakAt } from "./art/shaders";
import { useFrameLoop } from "./draw";
import { latestFrame, useSpectrumFeed } from "./feed";
import { paletteFromColour, paletteFromImage, type ArtPalette } from "./palette";
import { current } from "./scale";
import { INTENSITY, MusicSignals } from "./signals";
import type { ArtStyle } from "./styles";
import "./art.css";

/** A phone draws at most this share of its screen's pixels: the art is soft, the battery isn't free. */
export const PHONE_RENDER_CAP = 0.7;
/** And never more pixels than this, on any screen. */
export const MAX_RENDER_PIXELS = 2_600_000;
/** A new cover fades in over this long. */
export const COVER_FADE_MS = 1_100;
/** The record turns at 33 1/3 a minute while playing (degrees a second). */
const SPIN_DEG_S = 200;
/** A paused cover on the phone sits this much smaller (as now playing has it without the art). */
const PAUSED_REST = 0.94;

let grainTile: string | null | undefined;
/** A small tile of film grain, made once, for the sharp cover under Light. */
function grain(): string | null {
  if (grainTile !== undefined) return grainTile;
  grainTile = null;
  try {
    const c = document.createElement("canvas");
    c.width = 128;
    c.height = 128;
    const ctx = c.getContext("2d");
    if (!ctx) return null;
    const img = ctx.createImageData(128, 128);
    for (let i = 0; i < img.data.length; i += 4) {
      const v = Math.round(128 + (Math.random() + Math.random() - 1) * 110);
      img.data[i] = img.data[i + 1] = img.data[i + 2] = v;
      img.data[i + 3] = 255;
    }
    ctx.putImageData(img, 0, 0);
    grainTile = `url(${c.toDataURL("image/png")})`;
  } catch {
    grainTile = null;
  }
  return grainTile;
}

/** The drawing buffer's scale against CSS pixels for a style on this screen. */
export function renderScale(style: ArtStyle, dpr: number, phone: boolean, cssW: number, cssH: number): number {
  let s = dpr * DETAIL[style];
  if (phone) s = Math.min(s, dpr * PHONE_RENDER_CAP);
  const px = cssW * cssH * s * s;
  if (px > MAX_RENDER_PIXELS) s *= Math.sqrt(MAX_RENDER_PIXELS / px);
  return Math.max(0.25, s);
}

export interface ArtStageProps {
  style: ArtStyle;
  fps: number;
  reduced: boolean;
  intensity: VisualizerIntensity;
  phone?: boolean;
  /** The sharp cover on the page: the art gathers round it and sets its breath. */
  cover: () => HTMLElement | null;
}

type Renderer = "webgl2" | "css";

export function ArtStage({ style, fps, reduced, intensity, phone = false, cover }: ArtStageProps) {
  const thumb = usePlayer((s) => s.state?.track?.thumb ?? null);
  const playing = usePlayer((s) => s.state?.status === "playing" || s.state?.status === "buffering" || s.state?.status === "loading");
  const colourOn = useSettings((s) => s.settings?.artworkColours !== false);
  const vivid = useCoverColors(colourOn ? thumb : null)?.vivid ?? null;
  const root = useRef<HTMLDivElement>(null);
  const canvas = useRef<HTMLCanvasElement>(null);
  const glRef = useRef<ArtGL | null>(null);
  const [renderer, setRenderer] = useState<Renderer | null>(null);
  const signals = useRef(new MusicSignals());
  const size = useRef({ w: 0, h: 0, scale: 1 });
  const anchor = useRef<[number, number, number, number]>([0, 0, 0, 0]);
  const palette = useRef<ArtPalette>(paletteFromColour(null));
  const spin = useRef(0);
  const spinSpeed = useRef(0);
  const rest = useRef(phone && !playing ? PAUSED_REST : 1);
  const clock = useRef(0);
  const coverFn = useRef(cover);
  useLayoutEffect(() => {
    coverFn.current = cover;
  });
  // Nothing to move with under reduced motion, or for music playing elsewhere, so nothing is asked of the core.
  const local = usePlayer((s) => s.remote === null && !!s.state?.track);
  useSpectrumFeed(reduced || !local ? null : { fps, scope: false });

  // The renderer: WebGL2 when it builds, else the composited Sleeve; a lost context falls back until it returns.
  useLayoutEffect(() => {
    const c = canvas.current;
    if (!c) return;
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
    };
  }, []);

  // The palette shown on the CSS layers, and eased toward on the GPU.
  const applyPalette = (p: ArtPalette) => {
    palette.current = p;
    glRef.current?.setPalette(p.colours, p.base, performance.now());
    const el = root.current;
    if (!el) return;
    p.colours.forEach((c, i) => el.style.setProperty(`--art-c${i}`, `rgb(${c.map((v) => Math.round(v * 255)).join(" ")})`));
    el.style.setProperty("--art-base", `rgb(${p.base.map((v) => Math.round(v * 255)).join(" ")})`);
  };

  // Before the cover is read (or when it can't be), the palette from now playing's own colour.
  useEffect(() => {
    applyPalette(paletteFromColour(vivid ? parseColour(vivid) : null));
  }, [vivid, renderer]); // eslint-disable-line react-hooks/exhaustive-deps

  // The cover itself: drawn into a square once, uploaded as the texture, its palette read.
  useEffect(() => {
    const url = thumb ? artUrl(thumb, TEXTURE_SIZE / (typeof window !== "undefined" ? Math.min(2, window.devicePixelRatio || 1) : 1)) : undefined;
    if (!url || typeof Image === "undefined") return;
    let alive = true;
    const img = new Image();
    img.crossOrigin = "anonymous";
    img.decoding = "async";
    img.onload = () => {
      if (!alive) return;
      let src: TexImageSource = img;
      try {
        const sq = document.createElement("canvas");
        sq.width = TEXTURE_SIZE;
        sq.height = TEXTURE_SIZE;
        const ctx = sq.getContext("2d");
        if (ctx) {
          ctx.drawImage(img, 0, 0, TEXTURE_SIZE, TEXTURE_SIZE);
          src = sq;
        }
      } catch {
        /* drawn straight from the image instead */
      }
      try {
        glRef.current?.setCover(src, performance.now(), COVER_FADE_MS);
      } catch {
        /* a cover from another origin can't be a texture: the palette still shows */
      }
      const p = colourOn ? paletteFromImage(src as CanvasImageSource) : null;
      if (p) applyPalette(p);
    };
    img.src = url;
    return () => {
      alive = false;
    };
  }, [thumb, renderer, colourOn]); // eslint-disable-line react-hooks/exhaustive-deps

  // The drawing buffer follows the box, and where the sharp cover sits is read now and then (never per frame).
  useEffect(() => {
    const el = root.current;
    const c = canvas.current;
    if (!el || !c) return;
    const measure = () => {
      const r = el.getBoundingClientRect();
      const dpr = typeof window !== "undefined" ? window.devicePixelRatio || 1 : 1;
      const scale = renderScale(style, dpr, phone, r.width, r.height);
      const w = Math.max(1, Math.round(r.width * scale));
      const h = Math.max(1, Math.round(r.height * scale));
      if (c.width !== w) c.width = w;
      if (c.height !== h) c.height = h;
      size.current = { w, h, scale };
      const target = coverFn.current();
      if (target) {
        const t = target.getBoundingClientRect();
        // The breath scales the cover; its resting size is what counts.
        const breath = parseFloat(target.style.getPropertyValue("--art-breath")) || 1;
        const half = (Math.min(t.width, t.height) / 2 / breath) * scale;
        const round = style === "grooves" ? 1 : 0;
        anchor.current = [(t.left + t.width / 2 - r.left) * scale, (t.top + t.height / 2 - r.top) * scale, half, round];
      } else {
        anchor.current = [w / 2, h * 0.42, Math.min(w, h) * 0.3, style === "grooves" ? 1 : 0];
      }
    };
    measure();
    const every = setInterval(measure, 400);
    let ro: ResizeObserver | null = null;
    if (typeof ResizeObserver !== "undefined") {
      ro = new ResizeObserver(measure);
      ro.observe(el);
    }
    return () => {
      clearInterval(every);
      ro?.disconnect();
    };
  }, [style, phone, renderer]);

  // The cover's breath and light are the stage's while it shows; it gives them back when it goes
  // (or when another cover takes its place, as after a skip).
  const touched = useRef<HTMLElement | null>(null);
  useEffect(() => {
    return () => {
      giveBack(touched.current);
      touched.current = null;
    };
  }, [style]);

  const frame: ArtFrame = useRef<ArtFrame>({
    style,
    time: 0,
    spin: 0,
    low: 0,
    mid: 0,
    high: 0,
    pulse: 0,
    flow: 0,
    drift: 0,
    anchor: [0, 0, 0, 0],
    motion: 1,
    beatAt: signals.current.beatAt,
    beatStrength: signals.current.beatStrength,
  }).current;

  const fields = useRef<(HTMLElement | null)[]>([]);

  useFrameLoop(
    (dt, now) => {
      clock.current += dt / 1000;
      const sig = signals.current;
      const motion = reduced ? 0 : 1;
      if (!reduced) {
        const { frame: f, at } = latestFrame();
        const cur = current(f, at, now);
        sig.step(cur?.bands ?? null, cur, dt, clock.current * 1000, intensity);
      }
      // The record speeds up and slows down like a platter, never jumping.
      const wantSpin = playing && !reduced && style === "grooves" ? SPIN_DEG_S : 0;
      spinSpeed.current += (wantSpin - spinSpeed.current) * (1 - Math.exp(-dt / 700));
      spin.current = (spin.current + (spinSpeed.current * dt) / 1000) % 360;

      // Paused, a phone's cover settles back a little, as it does without the art.
      const restWant = phone && !playing ? PAUSED_REST : 1;
      rest.current += (restWant - rest.current) * (1 - Math.exp(-dt / 220));
      const el = coverFn.current();
      if (el !== touched.current) {
        giveBack(touched.current);
        touched.current = el;
      }
      if (el) {
        const gain = INTENSITY[intensity].gain;
        const breath = rest.current * (1 + motion * (0.034 * sig.pulse + 0.012 * sig.low * gain));
        el.style.setProperty("--art-breath", breath.toFixed(4));
        if (style === "grooves") el.style.setProperty("--art-spin", `${spin.current.toFixed(2)}deg`);
        if (style === "light") writeLight(el, sig, motion, anchor.current, size.current, clock.current);
      }

      const r = renderer;
      if (r === "webgl2" && glRef.current && !glRef.current.lost) {
        frame.style = style;
        frame.time = clock.current;
        frame.spin = (spin.current * Math.PI) / 180;
        frame.low = sig.low;
        frame.mid = sig.mid;
        frame.high = sig.high;
        frame.pulse = sig.pulse;
        frame.flow = sig.flow;
        frame.drift = sig.drift;
        frame.anchor = anchor.current;
        frame.motion = motion;
        glRef.current.draw(frame, now, size.current.w, size.current.h);
      } else if (r === "css") {
        moveFields(fields.current, sig, motion);
      }
    },
    // Paused, the art rests at a slower rate.
    playing ? fps : Math.min(fps, 20),
    renderer !== null,
  );

  const shown = renderer === "css" ? "sleeve" : style;
  return (
    <div
      ref={root}
      className={`art-stage art-stage--${shown}`}
      data-testid="art-stage"
      data-renderer={renderer ?? undefined}
      data-style={shown}
      data-reduced={reduced || undefined}
      aria-hidden="true"
    >
      <canvas ref={canvas} className="art-stage__canvas" hidden={renderer === "css"} />
      {renderer === "css" && (
        <div className="art-css" style={{ "--art-cover-opacity": thumb ? 1 : 0 } as CSSProperties}>
          <div className="art-css__cover">{thumb && <Art path={thumb} fluid requestSize={160} />}</div>
          {[0, 1, 2, 3].map((i) => (
            <i key={i} ref={(n) => void (fields.current[i] = n)} className={`art-css__field art-css__field--${i}`} />
          ))}
          <div className="art-css__vignette" />
        </div>
      )}
      <div className="art-stage__scrim art-stage__scrim--top" />
      <div className="art-stage__scrim art-stage__scrim--bottom" />
    </div>
  );
}

const COVER_PROPS = ["--art-breath", "--art-spin", "--art-leak-x", "--art-leak-y", "--art-leak-a", "--art-grain", "--art-grain-tile"];

/** Takes the stage's custom properties off a cover. */
function giveBack(el: HTMLElement | null) {
  if (el) for (const k of COVER_PROPS) el.style.removeProperty(k);
}

/** The composited Sleeve's colour fields: the shader's paths, as transforms. */
function moveFields(fields: (HTMLElement | null)[], sig: MusicSignals, motion: number) {
  const dr = sig.drift;
  for (let i = 0; i < fields.length; i++) {
    const el = fields[i];
    if (!el) continue;
    const x = Math.sin(dr * (0.11 + 0.031 * i) * motion + i * 1.91) * 30;
    const y = Math.cos(dr * (0.083 + 0.027 * i) * motion + i * 2.67) * 26;
    const s = 1 + motion * (0.3 * sig.low + 0.1 * sig.pulse);
    el.style.transform = `translate(${x.toFixed(2)}%, ${y.toFixed(2)}%) scale(${s.toFixed(3)})`;
    el.style.opacity = (0.5 + 0.3 * sig.low * motion).toFixed(3);
  }
}

/** Light: where the strongest leak falls on the sharp cover, how bright, and the grain's frame. */
function writeLight(el: HTMLElement, sig: MusicSignals, motion: number, anchor: readonly number[], size: { w: number; h: number; scale: number }, t: number) {
  const k = size.scale || 1;
  const W = size.w / k;
  const H = size.h / k;
  const L = Math.max(W, H) || 1;
  const half = anchor[2] / k;
  if (half > 0) {
    const [lx, ly] = leakAt(0, sig.drift, motion, [W / L, H / L]);
    // The leak's centre on the stage (y down), then in the cover's own box.
    const px = W / 2 + lx * L;
    const py = H / 2 - ly * L;
    const left = anchor[0] / k - half;
    const top = anchor[1] / k - half;
    el.style.setProperty("--art-leak-x", `${(((px - left) / (2 * half)) * 100).toFixed(1)}%`);
    el.style.setProperty("--art-leak-y", `${(((py - top) / (2 * half)) * 100).toFixed(1)}%`);
  }
  const flare = Math.min(1.4, sig.pulse * 0.9 + sig.high * 0.45) * motion;
  el.style.setProperty("--art-leak-a", (0.2 + 0.5 * flare).toFixed(3));
  if (!el.style.getPropertyValue("--art-grain-tile")) {
    const tile = grain();
    if (tile) el.style.setProperty("--art-grain-tile", tile);
  }
  const g = Math.floor(t * 24) * motion;
  el.style.setProperty("--art-grain", `${(g * 37) % 128}px ${(g * 71) % 128}px`);
}
