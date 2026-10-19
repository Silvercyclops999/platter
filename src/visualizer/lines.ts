/**
 * Thin lines on a canvas: WebGL2 when the canvas gets a context, Canvas 2D
 * otherwise. A lost GPU context reports itself, so the view can swap in a
 * fresh canvas drawn with Canvas 2D.
 */
import { parseColour, type RGB } from "../lib/color";

export interface Line {
  /** x, y pairs in CSS pixels. */
  points: Float32Array;
  colour: string;
  alpha: number;
  /** Width in CSS pixels. */
  width: number;
}

export interface LineRenderer {
  readonly kind: "webgl2" | "2d";
  draw(lines: Line[], w: number, h: number, dpr: number): void;
  dispose(): void;
}

const VERT = `#version 300 es
in vec2 a_pos;
uniform vec2 u_size;
void main() {
  vec2 c = a_pos / u_size * 2.0 - 1.0;
  gl_Position = vec4(c.x, -c.y, 0.0, 1.0);
}`;

const FRAG = `#version 300 es
precision mediump float;
uniform vec4 u_colour;
out vec4 o;
void main() { o = u_colour; }`;

function rgb(c: string): RGB {
  return parseColour(c) ?? [236, 228, 211];
}

class GlLines implements LineRenderer {
  readonly kind = "webgl2" as const;
  private prog: WebGLProgram;
  private buf: WebGLBuffer;
  private vao: WebGLVertexArrayObject;
  private uSize: WebGLUniformLocation | null;
  private uColour: WebGLUniformLocation | null;
  private verts = new Float32Array(0);

  constructor(private readonly gl: WebGL2RenderingContext) {
    const sh = (type: number, src: string) => {
      const s = gl.createShader(type)!;
      gl.shaderSource(s, src);
      gl.compileShader(s);
      if (!gl.getShaderParameter(s, gl.COMPILE_STATUS)) throw new Error(gl.getShaderInfoLog(s) ?? "shader");
      return s;
    };
    const prog = gl.createProgram()!;
    gl.attachShader(prog, sh(gl.VERTEX_SHADER, VERT));
    gl.attachShader(prog, sh(gl.FRAGMENT_SHADER, FRAG));
    gl.linkProgram(prog);
    if (!gl.getProgramParameter(prog, gl.LINK_STATUS)) throw new Error(gl.getProgramInfoLog(prog) ?? "program");
    this.prog = prog;
    this.uSize = gl.getUniformLocation(prog, "u_size");
    this.uColour = gl.getUniformLocation(prog, "u_colour");
    this.buf = gl.createBuffer()!;
    this.vao = gl.createVertexArray()!;
    gl.bindVertexArray(this.vao);
    gl.bindBuffer(gl.ARRAY_BUFFER, this.buf);
    const loc = gl.getAttribLocation(prog, "a_pos");
    gl.enableVertexAttribArray(loc);
    gl.vertexAttribPointer(loc, 2, gl.FLOAT, false, 0, 0);
    gl.enable(gl.BLEND);
    gl.blendFunc(gl.SRC_ALPHA, gl.ONE_MINUS_SRC_ALPHA);
  }

  /** A strip of quads, one per segment, `width` wide. Returns the vertex count. */
  private strip(points: Float32Array, width: number): number {
    const segs = points.length / 2 - 1;
    const need = Math.max(0, segs) * 12;
    if (this.verts.length < need) this.verts = new Float32Array(need);
    const v = this.verts;
    const hw = width / 2;
    let o = 0;
    for (let i = 0; i < segs; i++) {
      const x0 = points[i * 2];
      const y0 = points[i * 2 + 1];
      const x1 = points[i * 2 + 2];
      const y1 = points[i * 2 + 3];
      const dx = x1 - x0;
      const dy = y1 - y0;
      const len = Math.hypot(dx, dy) || 1;
      const nx = (-dy / len) * hw;
      const ny = (dx / len) * hw;
      // Each segment runs on half a width past its ends, so joins don't gap.
      const ex = (dx / len) * hw * 0.5;
      const ey = (dy / len) * hw * 0.5;
      const ax = x0 - ex;
      const ay = y0 - ey;
      const bx = x1 + ex;
      const by = y1 + ey;
      v.set([ax + nx, ay + ny, ax - nx, ay - ny, bx + nx, by + ny, bx + nx, by + ny, ax - nx, ay - ny, bx - nx, by - ny], o);
      o += 12;
    }
    return o / 2;
  }

  draw(lines: Line[], w: number, h: number, dpr: number) {
    const gl = this.gl;
    if (gl.isContextLost()) return;
    gl.viewport(0, 0, Math.round(w * dpr), Math.round(h * dpr));
    gl.clearColor(0, 0, 0, 0);
    gl.clear(gl.COLOR_BUFFER_BIT);
    gl.useProgram(this.prog);
    gl.bindVertexArray(this.vao);
    gl.bindBuffer(gl.ARRAY_BUFFER, this.buf);
    gl.uniform2f(this.uSize, w, h);
    for (const l of lines) {
      const n = this.strip(l.points, l.width);
      if (!n) continue;
      const [r, g, b] = rgb(l.colour);
      gl.uniform4f(this.uColour, r / 255, g / 255, b / 255, l.alpha);
      gl.bufferData(gl.ARRAY_BUFFER, this.verts.subarray(0, n * 2), gl.DYNAMIC_DRAW);
      gl.drawArrays(gl.TRIANGLES, 0, n);
    }
  }

  dispose() {
    const gl = this.gl;
    if (gl.isContextLost()) return;
    gl.deleteBuffer(this.buf);
    gl.deleteVertexArray(this.vao);
    gl.deleteProgram(this.prog);
    // Hand the GPU context back now rather than when the canvas is collected.
    gl.getExtension("WEBGL_lose_context")?.loseContext();
  }
}

class CanvasLines implements LineRenderer {
  readonly kind = "2d" as const;
  constructor(private readonly ctx: CanvasRenderingContext2D) {}

  draw(lines: Line[], w: number, h: number, dpr: number) {
    const c = this.ctx;
    c.setTransform(dpr, 0, 0, dpr, 0, 0);
    c.clearRect(0, 0, w, h);
    c.lineJoin = "round";
    c.lineCap = "round";
    for (const l of lines) {
      if (l.points.length < 4) continue;
      c.globalAlpha = l.alpha;
      c.strokeStyle = l.colour;
      c.lineWidth = l.width;
      c.beginPath();
      c.moveTo(l.points[0], l.points[1]);
      for (let i = 2; i < l.points.length; i += 2) c.lineTo(l.points[i], l.points[i + 1]);
      c.stroke();
    }
    c.globalAlpha = 1;
  }

  dispose() {}
}

/**
 * A renderer for `canvas`: WebGL2 when `preferGl` and the canvas has it,
 * else Canvas 2D. Null when the canvas gives neither (tests without a
 * canvas implementation).
 */
export function linesFor(canvas: HTMLCanvasElement, preferGl: boolean): LineRenderer | null {
  if (preferGl) {
    try {
      const gl = canvas.getContext("webgl2", { antialias: true, premultipliedAlpha: false, alpha: true });
      if (gl) return new GlLines(gl);
    } catch {
      /* no GPU here: Canvas 2D */
    }
  }
  try {
    const ctx = canvas.getContext("2d");
    return ctx ? new CanvasLines(ctx) : null;
  } catch {
    return null;
  }
}
