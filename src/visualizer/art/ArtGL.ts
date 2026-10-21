/**
 * Grooves and Blur on the GPU: one WebGL2 program each, one quad with no
 * buffers, and a handful of uniforms set per frame. No textures: the colours
 * come from the cover's palette, and the cover itself is the page's own
 * image on top. Nothing here touches React. A context that can't build the
 * programs gives itself back and the view draws with CSS layers instead.
 */
import type { RGB } from "../../lib/color";
import { BLUR, GROOVES, VERTEX } from "./shaders";

const UNIFORMS = ["uRes", "uTime", "uSpin", "uSig", "uDrift", "uFlow", "uPal", "uBase", "uLabel", "uBeatAt", "uBeatK"] as const;
type Uniform = (typeof UNIFORMS)[number];

/** The styles drawn here. */
export type GLStyle = "grooves" | "blur";
const SOURCES: Record<GLStyle, string> = { grooves: GROOVES, blur: BLUR };

/** What a frame draws with. */
export interface ArtFrame {
  time: number;
  /** The record's turn, radians. */
  spin: number;
  low: number;
  mid: number;
  high: number;
  pulse: number;
  drift: number;
  flow: number;
  /** The cover: centre x, y (y down, as on the page) and half its side, in canvas pixels. */
  label: [number, number, number];
  beatAt: Float32Array;
  beatStrength: Float32Array;
}

export class ArtGL {
  private gl: WebGL2RenderingContext;
  private progs: Partial<Record<GLStyle, { prog: WebGLProgram; loc: Record<Uniform, WebGLUniformLocation | null> }>> = {};
  private vao: WebGLVertexArrayObject | null = null;
  private palette = new Float32Array(12);
  private paletteFrom = new Float32Array(12);
  private paletteTo = new Float32Array(12);
  private base = new Float32Array([0.04, 0.035, 0.03]);
  private baseFrom = new Float32Array(3);
  private baseTo = new Float32Array([0.04, 0.035, 0.03]);
  private paletteStart = -Infinity;
  private paletteMs = 0;
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
    if (r.ready()) return r;
    r.dispose();
    return null;
  }

  /** Whether it can draw: the program built and the context is here. */
  ready(): boolean {
    return !this.gl.isContextLost() && !!this.progs.grooves && !!this.progs.blur;
  }

  private setup() {
    const gl = this.gl;
    this.vao = gl.createVertexArray();
    this.progs = {};
    for (const style of Object.keys(SOURCES) as GLStyle[]) {
      const built = this.build(SOURCES[style]);
      if (built) this.progs[style] = built;
    }
  }

  /** Builds everything again after the context comes back. */
  restore() {
    this.lost = false;
    this.setup();
  }

  private compile(type: number, src: string): WebGLShader | null {
    const gl = this.gl;
    const s = gl.createShader(type);
    if (!s) return null;
    gl.shaderSource(s, src);
    gl.compileShader(s);
    if (!gl.getShaderParameter(s, gl.COMPILE_STATUS)) {
      if (import.meta.env?.DEV) console.warn("visualizer shader", gl.getShaderInfoLog(s));
      gl.deleteShader(s);
      return null;
    }
    return s;
  }

  private build(fragment: string): { prog: WebGLProgram; loc: Record<Uniform, WebGLUniformLocation | null> } | null {
    const gl = this.gl;
    const vs = this.compile(gl.VERTEX_SHADER, VERTEX);
    const fs = this.compile(gl.FRAGMENT_SHADER, fragment);
    let out: { prog: WebGLProgram; loc: Record<Uniform, WebGLUniformLocation | null> } | null = null;
    if (vs && fs) {
      const prog = gl.createProgram();
      if (prog) {
        gl.attachShader(prog, vs);
        gl.attachShader(prog, fs);
        gl.linkProgram(prog);
        if (gl.getProgramParameter(prog, gl.LINK_STATUS)) {
          const loc = {} as Record<Uniform, WebGLUniformLocation | null>;
          for (const u of UNIFORMS) loc[u] = gl.getUniformLocation(prog, u);
          out = { prog, loc };
        } else gl.deleteProgram(prog);
      }
    }
    if (vs) gl.deleteShader(vs);
    if (fs) gl.deleteShader(fs);
    return out;
  }

  /** The palette to ease toward (four colours and the base, 0..1). */
  setPalette(colours: readonly RGB[], base: RGB, nowMs: number, fadeMs = 900) {
    this.ease(nowMs);
    this.paletteFrom.set(this.palette);
    this.baseFrom.set(this.base);
    for (let i = 0; i < 4; i++) for (let c = 0; c < 3; c++) this.paletteTo[i * 3 + c] = colours[i]?.[c] ?? 0.4;
    this.baseTo.set(base);
    this.paletteStart = nowMs;
    this.paletteMs = fadeMs;
  }

  private ease(nowMs: number) {
    const x = this.paletteMs > 0 ? Math.min(1, Math.max(0, (nowMs - this.paletteStart) / this.paletteMs)) : 1;
    const e = x * x * (3 - 2 * x);
    for (let i = 0; i < 12; i++) this.palette[i] = this.paletteFrom[i] + (this.paletteTo[i] - this.paletteFrom[i]) * e;
    for (let i = 0; i < 3; i++) this.base[i] = this.baseFrom[i] + (this.baseTo[i] - this.baseFrom[i]) * e;
  }

  draw(style: GLStyle, f: ArtFrame, nowMs: number, width: number, height: number) {
    const gl = this.gl;
    const p = this.progs[style];
    if (this.lost || gl.isContextLost() || !p) return;
    this.ease(nowMs);
    gl.viewport(0, 0, width, height);
    gl.useProgram(p.prog);
    gl.bindVertexArray(this.vao);
    const l = p.loc;
    gl.uniform2f(l.uRes, width, height);
    gl.uniform1f(l.uTime, f.time);
    gl.uniform1f(l.uSpin, f.spin);
    gl.uniform4f(l.uSig, f.low, f.mid, f.high, f.pulse);
    gl.uniform1f(l.uDrift, f.drift);
    gl.uniform1f(l.uFlow, f.flow);
    gl.uniform3fv(l.uPal, this.palette);
    gl.uniform3fv(l.uBase, this.base);
    // The page's y runs down, the GPU's up.
    gl.uniform3f(l.uLabel, f.label[0], height - f.label[1], f.label[2]);
    gl.uniform4fv(l.uBeatAt, f.beatAt);
    gl.uniform4fv(l.uBeatK, f.beatStrength);
    gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4);
  }

  /** Frees everything and gives the GPU context back. */
  dispose() {
    const gl = this.gl;
    for (const p of Object.values(this.progs)) if (p) gl.deleteProgram(p.prog);
    this.progs = {};
    gl.deleteVertexArray(this.vao);
    this.vao = null;
    this.lost = true;
    try {
      gl.getExtension("WEBGL_lose_context")?.loseContext();
    } catch {
      /* already gone */
    }
  }
}
