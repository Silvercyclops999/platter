/**
 * Grooves, drawn by the GPU on one quad: a record in the cover's colours.
 * Fine grooves run out from the label (the sharp cover, round, a page
 * element on top), their spacing breathing with the level; track bands sit
 * apart by quiet gaps; the room's light makes two bright wedges that swell
 * with the low end; and a ring runs out from the label on each beat.
 * Coordinates are device pixels with y up.
 */

export const VERTEX = `#version 300 es
void main() {
  vec2 p = vec2(float((gl_VertexID & 1) * 2 - 1), float((gl_VertexID >> 1) * 2 - 1));
  gl_Position = vec4(p, 0.0, 1.0);
}`;

export const GROOVES = `#version 300 es
precision highp float;
uniform vec2 uRes;
uniform float uTime;
uniform float uSpin;     // the record's turn (radians)
uniform vec4 uSig;       // low, mid, high, pulse (0..1)
uniform float uDrift;    // a slow phase, sped a little by the mids
uniform vec3 uPal[4];
uniform vec3 uBase;
uniform vec3 uLabel;     // the label: centre x, y and radius (px)
uniform vec4 uBeatAt;    // the last four beats (seconds), newest first
uniform vec4 uBeatK;     // and their strengths
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
  return mix(mix(hash12(i), hash12(i + vec2(1.0, 0.0)), u.x), mix(hash12(i + vec2(0.0, 1.0)), hash12(i + vec2(1.0, 1.0)), u.x), u.y);
}

void main() {
  vec2 fc = gl_FragCoord.xy;
  vec2 d = fc - uLabel.xy;
  float h = max(uLabel.z, 1.0);
  float R = length(d) / h;
  float ang = atan(d.y, d.x);
  float low = uSig.x;
  float lvl = 0.5 * (uSig.x + uSig.y);

  // A ring for each recent beat, running out from the label and fading as it goes.
  float rip = 0.0;
  for (int i = 0; i < 4; i++) {
    float age = uTime - uBeatAt[i];
    if (age < 0.0 || age > 3.0) continue;
    float at = 1.08 + age * 1.3;
    float w = (R - at) * 6.0;
    rip += uBeatK[i] * exp(-age * 1.4) * exp(-w * w);
  }

  // The grooves: their spacing opens a little with the level, and a beat's ring pushes them out.
  float freq = 30.0 * (1.0 - 0.06 * lvl);
  float phase = R * freq + rip * 0.8;
  float g = 0.5 + 0.5 * cos(phase * 6.2831853);
  g = mix(g, 0.5, smoothstep(0.25, 0.9, fwidth(phase)));

  // Track bands with a quiet gap between them, and the dead wax round the label.
  float bx = (R - 1.14) / 0.6;
  float gap = smoothstep(0.0, 0.035, fract(bx)) * smoothstep(1.0, 0.94, fract(bx));
  float inGrooves = smoothstep(1.08, 1.15, R);
  float wax = smoothstep(1.0, 1.03, R) * (1.0 - inGrooves);

  // The pressing's own grain turns with the record; the light stays put.
  vec2 rd = mat2(cos(uSpin), -sin(uSpin), sin(uSpin), cos(uSpin)) * d / h;
  float grain = vnoise(rd * 16.0) * 0.5 + vnoise(rd * 38.0) * 0.5;

  // The room's light from the upper left: two wedges across the label, broader as the low end swells.
  float la = 2.35 + 0.12 * sin(uDrift * 0.5);
  float c = abs(cos(ang - la));
  float sheen = pow(c, mix(14.0, 7.0, low)) * 0.85 + pow(c, 2.0) * 0.15;
  // The cover's lead colour, with its second colour in every other band: a record, not a rainbow.
  float alt = mod(floor(bx), 2.0);
  vec3 tint = mix(uPal[0], uPal[1], 0.15 + 0.3 * alt);

  vec3 vinyl = uBase * 0.6 + vec3(0.006);
  float fall = exp(-max(R - 1.0, 0.0) * 0.28);
  float lit = (0.05 + 0.6 * sheen) * (0.45 + 0.9 * lvl) * fall;
  vec3 col = vinyl;
  col += tint * g * lit * gap * inGrooves;
  col += tint * (0.025 + 0.04 * grain) * inGrooves * fall;
  col += vec3(0.045) * wax * (0.4 + sheen);
  col += mix(tint, vec3(1.0), 0.25) * rip * (0.22 + 0.35 * sheen) * inGrooves;

  // Darker toward the screen's edges, as under a single lamp.
  vec2 vp = (fc - 0.5 * uRes) / max(uRes.x, uRes.y);
  col *= smoothstep(1.0, 0.3, length(vp)) * 0.6 + 0.4;
  // A soft shoulder instead of clipping, and a little dither so the dark never bands.
  col = col / (1.0 + max(vec3(0.0), col - 0.85) * 1.6);
  col += (hash12(fc + fract(uTime) * 61.0) - 0.5) / 180.0;
  outColor = vec4(clamp(col, 0.0, 1.0), 1.0);
}`;
