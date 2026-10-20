/**
 * The art styles' WebGL2 drawing: one program per style (built when first
 * asked for), one quad with no buffers, two cover textures crossfading on a
 * song change, and a handful of uniforms set per frame. Nothing here touches
 * React; the stage owns one of these and calls `draw` from its frame loop.
 * A lost context is reported once and the stage falls back; a restored one
 * builds everything again.
 */
import type { RGB } from "../../lib/color";
import type { ArtStyle } from "../styles";
import { FRAGMENTS, VERTEX } from "./shaders";

const UNIFORMS = ["uCover", "uPrev", "uMix", "uRes", "uTime", "uSpin", "uSig", "uPhase", "uPal", "uBase", "uAnchor", "uMotion", "uBeatAt", "uBeatK"] as const;
type Uniform = (typeof UNIFORMS)[number];

interface Program {
  prog: WebGLProgram;
  loc: Record<Uniform, WebGLUniformLocation | null>;
}

/** What a frame draws with. */
export interface ArtFrame {
  style: ArtStyle;
  time: number;
  /** The record's turn, radians (Grooves). */
  spin: number;
  low: number;
  mid: number;
  high: number;
  pulse: number;
  flow: number;
  drift: number;
  /** The sharp cover: centre x, y and half size in canvas pixels (y down, as on the page), and 0 square or 1 round. */
  anchor: [number, number, number, number];
  motion: number;
  beatAt: Float32Array;
  beatStrength: Float32Array;
}

/** The cover a texture is made from: drawn into a square first, so every source uploads the same way. */
export const TEXTURE_SIZE = 512;

export class ArtGL {
  private gl: WebGL2RenderingContext;
  private programs = new Map<ArtStyle, Program | null>();
  private cover: WebGLTexture | null = null;
  private prev: WebGLTexture | null = null;
  private vao: WebGLVertexArrayObject | null = null;
  private palette = new Float32Array(12);
  private paletteFrom = new Float32Array(12);
  private paletteTo = new Float32Array(12);
  private base = new Float32Array([0.04, 0.035, 0.03]);
  private baseFrom = new Float32Array(3);
  private baseTo = new Float32Array([0.04, 0.035, 0.03]);
  /** Crossfade: 0 shows the previous cover, 1 the current one. */
  private mix = 1;
  private coverStart = -Infinity;
  private fadeMs = 0;
  private paletteStart = -Infinity;
  private paletteMs = 0;
  private lastSource: TexImageSource | null = null;
  lost = false;

  private constructor(gl: WebGL2RenderingContext) {
    this.gl = gl;
    this.setup();
  }

  /** A renderer on `canvas`, or null when WebGL2 isn't there or won't build. */
  static create(canvas: HTMLCanvasElement): ArtGL | null {
    let gl: WebGL2RenderingContext | null = null;
    try {
      gl = canvas.getContext("webgl2", {
        alpha: false,
        antialias: false,
        depth: false,
        stencil: false,
        premultipliedAlpha: true,
        preserveDrawingBuffer: false,
        powerPreference: "low-power",
      }) as WebGL2RenderingContext | null;
    } catch {
      gl = null;
    }
    if (!gl || gl.isContextLost?.()) return null;
    const r = new ArtGL(gl);
    // A style that doesn't build means this GPU can't draw the art: fall back.
    return r.program("sleeve") ? r : null;
  }

  private setup() {
    const gl = this.gl;
    this.vao = gl.createVertexArray();
    this.cover = this.texture();
    this.prev = this.texture();
    this.programs.clear();
    if (this.lastSource) this.upload(this.cover, this.lastSource);
  }

  /** Builds everything again after the context comes back. */
  restore() {
    this.lost = false;
    this.setup();
  }

  private texture(): WebGLTexture | null {
    const gl = this.gl;
    const t = gl.createTexture();
    gl.bindTexture(gl.TEXTURE_2D, t);
    gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, 1, 1, 0, gl.RGBA, gl.UNSIGNED_BYTE, new Uint8Array([24, 20, 18, 255]));
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR_MIPMAP_LINEAR);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.MIRRORED_REPEAT);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.MIRRORED_REPEAT);
    gl.generateMipmap(gl.TEXTURE_2D);
    return t;
  }

  private upload(t: WebGLTexture | null, src: TexImageSource) {
    const gl = this.gl;
    gl.bindTexture(gl.TEXTURE_2D, t);
    gl.pixelStorei(gl.UNPACK_FLIP_Y_WEBGL, true);
    gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, gl.RGBA, gl.UNSIGNED_BYTE, src);
    gl.pixelStorei(gl.UNPACK_FLIP_Y_WEBGL, false);
    gl.generateMipmap(gl.TEXTURE_2D);
  }

  private compile(type: number, src: string): WebGLShader | null {
    const gl = this.gl;
    const s = gl.createShader(type);
    if (!s) return null;
    gl.shaderSource(s, src);
    gl.compileShader(s);
    if (!gl.getShaderParameter(s, gl.COMPILE_STATUS)) {
      if (import.meta.env?.DEV) console.warn("art shader", gl.getShaderInfoLog(s));
      gl.deleteShader(s);
      return null;
    }
    return s;
  }

  private program(style: ArtStyle): Program | null {
    if (this.programs.has(style)) return this.programs.get(style)!;
    const gl = this.gl;
    const vs = this.compile(gl.VERTEX_SHADER, VERTEX);
    const fs = this.compile(gl.FRAGMENT_SHADER, FRAGMENTS[style]);
    let out: Program | null = null;
    if (vs && fs) {
      const prog = gl.createProgram();
      if (prog) {
        gl.attachShader(prog, vs);
        gl.attachShader(prog, fs);
        gl.linkProgram(prog);
        if (gl.getProgramParameter(prog, gl.LINK_STATUS)) {
          const loc = {} as Program["loc"];
          for (const u of UNIFORMS) loc[u] = gl.getUniformLocation(prog, u);
          out = { prog, loc };
        }
      }
    }
    if (vs) gl.deleteShader(vs);
    if (fs) gl.deleteShader(fs);
    this.programs.set(style, out);
    return out;
  }

  /** A new cover: the old one fades out under it over `fadeMs` (0 swaps at once). */
  setCover(src: TexImageSource, nowMs: number, fadeMs: number) {
    const swap = this.prev;
    this.prev = this.cover;
    this.cover = swap;
    this.upload(this.cover, src);
    this.lastSource = src;
    this.fadeMs = fadeMs;
    this.coverStart = fadeMs > 0 ? nowMs : -Infinity;
  }

  /** The palette to ease toward (four colours and the base, 0..1). */
  setPalette(colours: readonly RGB[], base: RGB, nowMs: number, fadeMs = 900) {
    this.paletteFrom.set(this.palette);
    this.baseFrom.set(this.base);
    for (let i = 0; i < 4; i++) for (let c = 0; c < 3; c++) this.paletteTo[i * 3 + c] = colours[i]?.[c] ?? 0.4;
    this.baseTo.set(base);
    this.paletteStart = nowMs;
    this.paletteMs = fadeMs;
  }

  private ease(nowMs: number) {
    const smooth = (k: number) => {
      const x = Math.min(1, Math.max(0, k));
      return x * x * (3 - 2 * x);
    };
    this.mix = this.fadeMs > 0 ? smooth((nowMs - this.coverStart) / this.fadeMs) : 1;
    const e = this.paletteMs > 0 ? smooth((nowMs - this.paletteStart) / this.paletteMs) : 1;
    for (let i = 0; i < 12; i++) this.palette[i] = this.paletteFrom[i] + (this.paletteTo[i] - this.paletteFrom[i]) * e;
    for (let i = 0; i < 3; i++) this.base[i] = this.baseFrom[i] + (this.baseTo[i] - this.baseFrom[i]) * e;
  }

  draw(f: ArtFrame, nowMs: number, width: number, height: number) {
    const gl = this.gl;
    if (this.lost || gl.isContextLost()) return;
    const p = this.program(f.style) ?? this.program("sleeve");
    if (!p) return;
    this.ease(nowMs);
    gl.viewport(0, 0, width, height);
    gl.useProgram(p.prog);
    gl.bindVertexArray(this.vao);
    gl.activeTexture(gl.TEXTURE0);
    gl.bindTexture(gl.TEXTURE_2D, this.cover);
    gl.activeTexture(gl.TEXTURE1);
    gl.bindTexture(gl.TEXTURE_2D, this.prev);
    const l = p.loc;
    gl.uniform1i(l.uCover, 0);
    gl.uniform1i(l.uPrev, 1);
    gl.uniform1f(l.uMix, this.mix);
    gl.uniform2f(l.uRes, width, height);
    gl.uniform1f(l.uTime, f.time);
    gl.uniform1f(l.uSpin, f.spin);
    gl.uniform4f(l.uSig, f.low, f.mid, f.high, f.pulse);
    gl.uniform2f(l.uPhase, f.flow, f.drift);
    gl.uniform3fv(l.uPal, this.palette);
    gl.uniform3fv(l.uBase, this.base);
    // The page's y runs down, the GPU's up.
    gl.uniform4f(l.uAnchor, f.anchor[0], height - f.anchor[1], f.anchor[2], f.anchor[3]);
    gl.uniform1f(l.uMotion, f.motion);
    gl.uniform4fv(l.uBeatAt, f.beatAt);
    gl.uniform4fv(l.uBeatK, f.beatStrength);
    gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4);
  }

  dispose() {
    const gl = this.gl;
    for (const p of this.programs.values()) if (p) gl.deleteProgram(p.prog);
    this.programs.clear();
    gl.deleteTexture(this.cover);
    gl.deleteTexture(this.prev);
    gl.deleteVertexArray(this.vao);
  }
}
