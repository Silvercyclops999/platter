/**
 * The art styles' fragment shaders. Each draws the whole screen on one quad
 * from the playing cover (a texture with mipmaps, so a soft copy is one
 * lookup at a coarse level) and its palette, moved by the music signals.
 * Coordinates are device pixels with y up; the sharp cover itself is a page
 * element on top, and uAnchor says where it sits so the art can gather
 * round it.
 */
import type { ArtStyle } from "../styles";

export const VERTEX = `#version 300 es
void main() {
  vec2 p = vec2(float((gl_VertexID & 1) * 2 - 1), float((gl_VertexID >> 1) * 2 - 1));
  gl_Position = vec4(p, 0.0, 1.0);
}`;

const COMMON = `#version 300 es
precision highp float;
uniform sampler2D uCover;
uniform sampler2D uPrev;
uniform float uMix;
uniform vec2 uRes;
uniform float uTime;
uniform float uSpin;     // the record's turn (radians)
uniform vec4 uSig;      // low, mid, high, pulse (0..1)
uniform vec2 uPhase;    // flow, drift (seconds-ish, sped by the music)
uniform vec3 uPal[4];
uniform vec3 uBase;
uniform vec4 uAnchor;   // the sharp cover: centre x, y and half size (px), corner (0 square, 1 round)
uniform float uMotion;  // 0 under reduced motion
uniform vec4 uBeatAt;   // the last four beats (stage seconds), newest first
uniform vec4 uBeatK;    // and their strengths
out vec4 outColor;

float hash12(vec2 p) {
  vec3 p3 = fract(vec3(p.xyx) * 0.1031);
  p3 += dot(p3, p3.yzx + 33.33);
  return fract((p3.x + p3.y) * p3.z);
}
float vnoise(vec2 p) {
  vec2 i = floor(p);
  vec2 f = fract(p);
  vec2 u = f * f * (3.0 - 2.0 * f);
  float a = hash12(i);
  float b = hash12(i + vec2(1.0, 0.0));
  float c = hash12(i + vec2(0.0, 1.0));
  float d = hash12(i + vec2(1.0, 1.0));
  return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}
float fbm(vec2 p) {
  float v = 0.0;
  float a = 0.5;
  mat2 r = mat2(0.8, -0.6, 0.6, 0.8);
  for (int i = 0; i < 5; i++) {
    v += a * vnoise(p);
    p = r * p * 2.03 + 17.1;
    a *= 0.5;
  }
  return v;
}
vec3 coverAt(vec2 uv, float lod) {
  return mix(textureLod(uPrev, uv, lod).rgb, textureLod(uCover, uv, lod).rgb, uMix);
}
// A soft copy: five lookups at a coarse level, so the blur has no blocks.
vec3 soft(vec2 uv, float lod) {
  float d = exp2(lod) / 640.0;
  vec3 c = coverAt(uv, lod) * 0.36;
  c += coverAt(uv + vec2(d, d * 0.4), lod) * 0.16;
  c += coverAt(uv + vec2(-d * 0.4, d), lod) * 0.16;
  c += coverAt(uv + vec2(-d, -d * 0.4), lod) * 0.16;
  c += coverAt(uv + vec2(d * 0.4, -d), lod) * 0.16;
  return c;
}
vec3 saturateBy(vec3 c, float s) {
  float l = dot(c, vec3(0.2126, 0.7152, 0.0722));
  return mix(vec3(l), c, s);
}
vec3 screenBlend(vec3 a, vec3 b) { return 1.0 - (1.0 - a) * (1.0 - clamp(b, 0.0, 1.0)); }
// Distance in px outside the sharp cover (negative inside), rounded or square.
float coverDistance(vec2 fc) {
  vec2 d = fc - uAnchor.xy;
  float h = uAnchor.z;
  float rnd = length(d) - h;
  vec2 q = abs(d) - vec2(h);
  float sq = length(max(q, 0.0)) + min(max(q.x, q.y), 0.0);
  return mix(sq, rnd, uAnchor.w);
}
vec3 finish(vec3 c, vec2 fc) {
  // A soft shoulder instead of clipping, then a little dither so gradients never band.
  c = c / (1.0 + max(vec3(0.0), c - 0.85) * 1.6);
  c += (hash12(fc + fract(uTime) * 61.0) - 0.5) / 160.0;
  return clamp(c, 0.0, 1.0);
}
`;

/**
 * Sleeve: the cover, enlarged and soft, under four colour fields from its
 * palette that drift on slow paths and swell with the low end, and a glow
 * round the sharp cover that breathes with the bass.
 */
const SLEEVE = `${COMMON}
void main() {
  vec2 fc = gl_FragCoord.xy;
  float L = max(uRes.x, uRes.y);
  vec2 p = (fc - 0.5 * uRes) / L;
  vec2 aspect = uRes / L;
  float m = uMotion;
  float dr = uPhase.y;
  float a = m * 0.06 * sin(dr * 0.21);
  mat2 R = mat2(cos(a), -sin(a), sin(a), cos(a));
  // The soft cover, enlarged, slowly swirling like light through water.
  vec2 w = m * 0.04 * vec2(sin(p.y * 3.1 + dr * 0.37), cos(p.x * 2.7 - dr * 0.31));
  vec2 uv = 0.5 + (R * p + w) / (1.36 + 0.025 * uSig.x * m) + m * 0.025 * vec2(sin(dr * 0.17), cos(dr * 0.13));
  vec3 bg = saturateBy(soft(uv, 5.6), 1.45) * 0.42;

  vec3 light = vec3(0.0);
  for (int i = 0; i < 4; i++) {
    float fi = float(i);
    vec2 c = vec2(sin(dr * (0.11 + 0.031 * fi) * m + fi * 1.91), cos(dr * (0.083 + 0.027 * fi) * m + fi * 2.67));
    c *= aspect * vec2(0.42, 0.4);
    float r = (0.24 + 0.03 * fi) * (1.0 + 0.3 * uSig.x * m + 0.12 * uSig.w * m);
    vec2 d = p - c;
    float f = exp(-dot(d, d) / (r * r));
    light += uPal[i] * f * (0.5 + 0.3 * uSig.x * m);
  }
  vec3 col = screenBlend(bg, light * 0.72);

  // The glow round the cover, breathing with the bass.
  float h = max(uAnchor.z, 1.0);
  float out_ = max(coverDistance(fc), 0.0) / h;
  float glow = exp(-out_ * (1.7 - 0.5 * uSig.x * m)) * (0.3 + 0.42 * uSig.x * m + 0.2 * uSig.w * m);
  col = screenBlend(col, uPal[0] * glow);
  col *= 1.0 + 0.07 * uSig.w * m;

  float v = smoothstep(0.95, 0.15, length(p / aspect * vec2(0.9, 1.0)));
  col = mix(uBase, col, 0.3 + 0.7 * v);
  outColor = vec4(finish(col, fc), 1.0);
}`;

/**
 * Ink: the cover's colours as smoke, a domain-warped noise field that the
 * bass pushes along faster and the highs light at its edges. Round the
 * sharp cover the smoke is the cover itself, pulled apart, so the cover
 * dissolves into it.
 */
const INK = `${COMMON}
void main() {
  vec2 fc = gl_FragCoord.xy;
  float S = min(uRes.x, uRes.y);
  vec2 p = (fc - uAnchor.xy) / S;
  float t = uPhase.x * 0.55;
  // A beat breathes the smoke out from the cover.
  p *= 1.0 - 0.05 * uSig.w * uMotion;
  vec2 q = vec2(fbm(p * 1.25 + vec2(0.0, t * 0.6)), fbm(p * 1.25 + vec2(5.2, 1.3) - t * 0.5));
  vec2 r = vec2(fbm(p * 1.25 + 3.0 * q + vec2(1.7, 9.2) + t * 0.8), fbm(p * 1.25 + 3.0 * q + vec2(8.3, 2.8) - t * 0.7));
  float f = fbm(p * 1.25 + 3.4 * r);

  vec3 col = mix(uBase, uPal[1] * 0.55, smoothstep(0.15, 0.85, f));
  col = mix(col, uPal[0] * 0.95, smoothstep(0.25, 1.05, length(q)) * 0.75);
  col = mix(col, uPal[2] * 0.8, smoothstep(0.45, 0.95, r.x) * 0.55);
  // Wisps: where the field folds, lit by the highs.
  float wisp = smoothstep(0.55, 0.95, f * f * 2.1);
  col += uPal[3] * wisp * (0.12 + 0.55 * uSig.z * uMotion);
  col *= 0.8 + 0.35 * uSig.x * uMotion;

  // The cover, pulled into the smoke by the same field near its edges.
  float h = max(uAnchor.z, 1.0);
  float dist = coverDistance(fc) / h;
  vec2 cuv = 0.5 + (fc - uAnchor.xy) / (2.0 * h);
  cuv += (r - 0.5) * 0.22 * smoothstep(-0.3, 1.2, dist);
  vec3 cov = saturateBy(coverAt(cuv, 1.5 + 2.5 * smoothstep(0.0, 1.0, dist)), 1.15);
  float near = 1.0 - smoothstep(-0.25, 0.9 + 0.15 * uSig.x, dist + (f - 0.5) * 0.7);
  col = mix(col, cov * 0.92, near * 0.9);

  // Brightest round the cover, the smoke settling darker toward the screen's edges.
  col *= mix(1.0, 0.5, smoothstep(0.25, 0.95, length(p)));
  vec2 vp = (fc - 0.5 * uRes) / max(uRes.x, uRes.y);
  col *= smoothstep(0.9, 0.25, length(vp)) * 0.5 + 0.5;
  outColor = vec4(finish(col, fc), 1.0);
}`;

/**
 * Grooves: a record in the cover's colours. Fine grooves run out from the
 * label (the sharp cover, round), their spacing breathing with the level,
 * track bands set apart by quiet gaps, a fixed sheen from the room's light,
 * and a ring rippling outward from the label on each beat.
 */
const GROOVES = `${COMMON}
vec3 pal(float x) {
  x = fract(x) * 4.0;
  int i = int(floor(x));
  float f = smoothstep(0.0, 1.0, fract(x));
  vec3 a = i == 0 ? uPal[0] : i == 1 ? uPal[1] : i == 2 ? uPal[2] : uPal[3];
  vec3 b = i == 0 ? uPal[1] : i == 1 ? uPal[2] : i == 2 ? uPal[3] : uPal[0];
  return mix(a, b, f);
}
void main() {
  vec2 fc = gl_FragCoord.xy;
  vec2 d = fc - uAnchor.xy;
  float h = max(uAnchor.z, 1.0);
  float R = length(d) / h;
  float ang = atan(d.y, d.x);
  float m = uMotion;
  float lvl = 0.5 * (uSig.x + uSig.y);

  // Ripples: a ring for each recent beat, travelling out from the label.
  float rip = 0.0;
  for (int i = 0; i < 4; i++) {
    float age = uTime - uBeatAt[i];
    if (age < 0.0 || age > 4.0) continue;
    float at = 1.05 + age * 1.5;
    float w = (R - at) * 5.0;
    rip += uBeatK[i] * exp(-age * 1.1) * exp(-w * w);
  }
  rip *= m;

  float freq = 34.0 * (1.0 - 0.07 * lvl * m);
  float phase = R * freq + rip * 0.9;
  float g = 0.5 + 0.5 * cos(phase * 6.2831853);
  float aa = fwidth(phase);
  g = mix(g, 0.5, smoothstep(0.25, 0.9, aa));

  // Track bands with a quiet gap between them.
  float bandW = 0.55;
  float bx = (R - 1.12) / bandW;
  float gap = smoothstep(0.0, 0.04, fract(bx)) * smoothstep(1.0, 0.93, fract(bx));
  float inGrooves = smoothstep(1.06, 1.14, R);
  // The run-in: a smooth ring of dead wax right round the label.
  float wax = smoothstep(1.0, 1.03, R) * (1.0 - inGrooves);

  // A spinning record shows its texture turning while the sheen stays put.
  float spin = uSpin;
  vec2 rd = mat2(cos(spin), -sin(spin), sin(spin), cos(spin)) * d / h;
  float grain = vnoise(rd * 18.0) * 0.5 + vnoise(rd * 41.0) * 0.5;

  // The room's light from the side, so its two bright wedges sit left and right of the label.
  float la = 0.22 + 0.3 * sin(uPhase.y * 0.4) * m;
  float sheen = pow(abs(cos(ang - la)), 10.0) * 0.85 + pow(abs(cos(ang - la)), 2.0) * 0.15;
  vec3 tint = pal(R * 0.11 + floor(bx) * 0.25 + uPhase.y * 0.015 * m);

  vec3 vinyl = uBase * 0.7 + vec3(0.008);
  float fall = exp(-max(R - 1.0, 0.0) * 0.22);
  vec3 col = vinyl;
  float lit = (0.07 + 0.55 * sheen) * (0.55 + 0.75 * lvl * m) * fall;
  col += tint * g * lit * gap * inGrooves;
  col += tint * (0.03 + 0.05 * grain) * inGrooves * fall;
  col += vec3(0.05) * wax * (0.4 + sheen);
  col += tint * rip * (0.32 + 0.4 * sheen) * inGrooves;
  col *= 1.0 + 0.08 * uSig.w * m;

  // The light falls away toward the screen's edges.
  vec2 vp = (fc - 0.5 * uRes) / max(uRes.x, uRes.y);
  col *= smoothstep(1.0, 0.3, length(vp)) * 0.6 + 0.4;
  outColor = vec4(finish(col, fc), 1.0);
}`;

/**
 * Light: the cover large and close, as if photographed, with film grain and
 * soft light leaks coming in from the edges. The leaks drift with the mids
 * and flare on transients.
 */
const LIGHT = `${COMMON}
void main() {
  vec2 fc = gl_FragCoord.xy;
  float L = max(uRes.x, uRes.y);
  vec2 p = (fc - 0.5 * uRes) / L;
  vec2 aspect = uRes / L;
  float m = uMotion;
  float dr = uPhase.y;
  vec2 uv = 0.5 + p / (1.12 + 0.05 * sin(dr * 0.09) * m) + m * 0.02 * vec2(sin(dr * 0.13), cos(dr * 0.11));
  vec3 base = saturateBy(soft(uv, 3.6), 1.15);
  vec3 col = base * 0.4;

  float flare = clamp(uSig.w * 0.9 + uSig.z * 0.45, 0.0, 1.4) * m;
  vec3 leak = vec3(0.0);
  // Two leaks from opposite edges, warm amber and the cover's own colour warmed, sweeping slowly round.
  for (int i = 0; i < 2; i++) {
    float fi = float(i);
    float a = dr * (0.16 + 0.05 * fi) * m + fi * 3.14159 + 0.7;
    vec2 c = vec2(cos(a), sin(a * 0.7 + fi * 1.3)) * aspect * vec2(0.7, 0.36);
    vec3 tint = i == 0 ? vec3(1.0, 0.56, 0.24) : mix(uPal[0], vec3(1.0, 0.42, 0.46), 0.4) * 1.15;
    vec2 dd = (p - c) * vec2(1.0, 1.25);
    float r = 0.34 + 0.06 * uSig.y * m;
    float f = exp(-dot(dd, dd) / (r * r));
    leak += tint * f * (0.42 + 0.95 * flare);
  }
  // A broad soft band of light across the frame, like sun through a window, drifting with the mids.
  vec2 dir = normalize(vec2(0.8, 0.6));
  float off = dot(p, dir) - 0.5 * sin(dr * 0.12 + 1.0) * m;
  float band = exp(-off * off / 0.018);
  leak += mix(uPal[1], vec3(1.0, 0.86, 0.7), 0.55) * band * (0.1 + 0.28 * uSig.y * m + 0.2 * flare);
  col = screenBlend(col, leak);
  // A faint streak through the light, as in a lens.
  col += leak * exp(-abs(p.y * 9.0)) * 0.1 * flare;

  // Grain, a new frame of it 24 times a second, strongest in the mid tones.
  vec2 gp = floor(fc / 1.5) + floor(uTime * 24.0) * vec2(17.0, 59.0);
  float n = hash12(gp) + hash12(gp + 7.3) - 1.0;
  float luma = dot(col, vec3(0.2126, 0.7152, 0.0722));
  col += n * 0.07 * (1.0 - abs(luma - 0.45) * 1.2);

  float v = smoothstep(0.9, 0.2, length(p / aspect * vec2(0.85, 1.0)));
  col = mix(uBase, col, 0.25 + 0.75 * v);
  outColor = vec4(finish(col, fc), 1.0);
}`;

export const FRAGMENTS: Record<ArtStyle, string> = { sleeve: SLEEVE, ink: INK, grooves: GROOVES, light: LIGHT };

/**
 * How finely each style is drawn, against the screen's pixels: soft styles
 * at half, the grooves' fine lines at full.
 */
export const DETAIL: Record<ArtStyle, number> = { sleeve: 0.5, ink: 0.5, grooves: 1, light: 0.75 };

/** Where the light leaks sit, for the sharp cover's own light (same paths as the shader). */
export function leakAt(i: number, drift: number, motion: number, aspect: [number, number]): [number, number] {
  const a = drift * (0.16 + 0.05 * i) * motion + i * Math.PI + 0.7;
  return [Math.cos(a) * aspect[0] * 0.7, Math.sin(a * 0.7 + i * 1.3) * aspect[1] * 0.36];
}
