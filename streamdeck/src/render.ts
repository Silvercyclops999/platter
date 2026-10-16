/**
 * Key, dial and infobar pictures as SVG. Vector, so they're sharp on 72,
 * 96 and 144 pixel keys alike; covers come at 288 px. Only SVG Tiny
 * features (the Stream Deck app draws SVG with them): long text fades out
 * through a gradient fill rather than a mask.
 */
import { GLYPHS } from "../scripts/glyphs.mjs";

/** Platter's colours. */
export const C = {
  bg: "#171513",
  bg2: "#201D1A",
  text: "#F4EFE8",
  muted: "#B8AFA4",
  faint: "#5E574D",
  /** The one accent: on, playing, the bar. */
  accent: "#F0957A",
  warn: "#F2B13D",
  shade: "#000000",
};

export const FONT = `Inter, "Inter Variable", "Segoe UI", "Helvetica Neue", Arial, sans-serif`;

/** Escape text for SVG. */
export const esc = (s: string): string => s.replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c] as string);

/** About how wide `text` is at `size` px in an Inter-like face. */
export function textWidth(text: string, size: number, bold = false): number {
  let w = 0;
  for (const ch of text) {
    if ("iljI.,:;'|!`".includes(ch)) w += 0.27;
    else if ("ft()[]{} r-".includes(ch)) w += 0.36;
    else if ("mwMW@%".includes(ch)) w += 0.86;
    else if (/[A-Z]/.test(ch)) w += 0.66;
    else if (/[0-9]/.test(ch)) w += 0.57;
    else if (ch.charCodeAt(0) > 0x2e7f) w += 1;
    else w += 0.53;
  }
  return w * size * (bold ? 1.06 : 1);
}

/** A short stable id for a fade, from what it fades (the same picture gives the same SVG). */
function fadeId(...parts: (string | number)[]): string {
  let h = 2166136261;
  for (const ch of parts.join("|")) h = Math.imul(h ^ ch.charCodeAt(0), 16777619);
  return `fade${(h >>> 0).toString(36)}`;
}

/** One line of text, fading out at `max` px wide when it doesn't fit. */
export function line(
  text: string,
  x: number,
  y: number,
  opts: { size: number; max: number; fill?: string; bold?: boolean; anchor?: "start" | "middle" | "end"; opacity?: number; fit?: number },
): string {
  // Shrink to fit, down to `fit` px, before fading.
  let size = opts.size;
  while (opts.fit && size > opts.fit && textWidth(text, size, opts.bold) > opts.max) size--;
  const o = { ...opts, size };
  const fill = o.fill ?? C.text;
  const anchor = o.anchor ?? "start";
  const w = textWidth(text, o.size, o.bold);
  const weight = o.bold ? 700 : 500;
  const base = `x="${x}" y="${y}" font-family='${FONT}' font-size="${o.size}" font-weight="${weight}" text-anchor="${anchor}"${o.opacity !== undefined ? ` opacity="${o.opacity}"` : ""}`;
  if (w <= o.max) return `<text ${base} fill="${fill}">${esc(text)}</text>`;
  // Too long: start-anchored, fading over the last 22 px of the room.
  const x0 = anchor === "middle" ? x - o.max / 2 : anchor === "end" ? x - o.max : x;
  const id = fadeId(text, x0, y, o.size, o.max, fill);
  const at = Math.max(0, (o.max - 22) / o.max);
  const grad = `<linearGradient id="${id}" gradientUnits="userSpaceOnUse" x1="${x0}" y1="0" x2="${x0 + o.max}" y2="0"><stop offset="0" stop-color="${fill}"/><stop offset="${at.toFixed(3)}" stop-color="${fill}"/><stop offset="1" stop-color="${fill}" stop-opacity="0"/></linearGradient>`;
  // Characters past the room would draw transparent anyway; leave them out.
  const keep = Math.ceil((o.max / Math.max(1, w)) * [...text].length) + 1;
  const cut = [...text].slice(0, keep).join("");
  return `<defs>${grad}</defs><text x="${x0}" y="${y}" font-family='${FONT}' font-size="${o.size}" font-weight="${weight}" text-anchor="start" fill="url(#${id})"${o.opacity !== undefined ? ` opacity="${o.opacity}"` : ""}>${esc(cut)}</text>`;
}

/** A data URL for an SVG picture. */
export function svgUrl(body: string, w = 144, h = 144): string {
  const doc = `<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" viewBox="0 0 ${w} ${h}" width="${w}" height="${h}">${body}</svg>`;
  return `data:image/svg+xml;charset=utf8,${encodeURIComponent(doc)}`;
}

/** A cover image filling a box. */
export const image = (href: string, x: number, y: number, w: number, h: number): string =>
  `<image x="${x}" y="${y}" width="${w}" height="${h}" preserveAspectRatio="xMidYMid slice" xlink:href="${esc(href)}" href="${esc(href)}"/>`;

/** A glyph, at `scale` around a centre. */
export function glyph(name: string, colour: string, cx = 72, cy = 72, scale = 0.62): string {
  const g = GLYPHS[name];
  if (!g) return "";
  return `<g transform="translate(${cx} ${cy}) scale(${scale}) translate(-72 -72)">${g(colour)}</g>`;
}

/** A thin progress bar. */
export function bar(x: number, y: number, w: number, h: number, v: number, track = "#FFFFFF", trackOpacity = 0.22): string {
  const f = Math.max(0, Math.min(1, v));
  return `<rect x="${x}" y="${y}" width="${w}" height="${h}" rx="${h / 2}" fill="${track}" fill-opacity="${trackOpacity}"/>${f > 0 ? `<rect x="${x}" y="${y}" width="${Math.max(h, w * f)}" height="${h}" rx="${h / 2}" fill="${C.accent}"/>` : ""}`;
}

export type Badge = "unpaired" | "refused";

/** A small badge over the picture: not paired (a link) or not allowed (a bar). */
export function badge(kind: Badge): string {
  const mark =
    kind === "unpaired"
      ? `<path d="M118 22h8M124 16l6 6-6 6" fill="none" stroke="${C.bg}" stroke-width="3.5" stroke-linecap="round" stroke-linejoin="round"/>`
      : `<rect x="114" y="20" width="16" height="4.5" rx="2" fill="${C.bg}"/>`;
  return `<circle cx="122" cy="22" r="17" fill="${C.warn}"/><circle cx="122" cy="22" r="17" fill="none" stroke="${C.bg}" stroke-width="3"/>${mark}`;
}

// ---------------------------------------------------------------- keys

export interface KeyPic {
  /** A glyph name from scripts/glyphs.mjs. */
  glyph?: string;
  /** Drawn in the accent (a mode that's on). */
  on?: boolean;
  /** Greyed out: the action can't run now. */
  off?: boolean;
  /** Small text under the glyph. */
  label?: string;
  /** Big text instead of (or over) the glyph: a number, a countdown. */
  big?: string;
  /** A cover behind everything. */
  cover?: string | null;
  /** Darken the cover, 0 to 1. */
  dim?: number;
  /** A level bar at the bottom (0 to 1). */
  bar?: number | null;
  badge?: Badge | null;
  /** The user's own picture, instead of all of the above but the badge. */
  custom?: string | null;
  /** An accent frame: this is what's playing. */
  ring?: boolean;
}

const RING = `<rect x="4" y="4" width="136" height="136" rx="10" fill="none" stroke="${C.accent}" stroke-width="8"/>`;

/** A plain key: a glyph (or big text) on Platter's dark tile, maybe over a cover. */
export function keyPic(p: KeyPic): string {
  if (p.custom) {
    const shade = p.off ? `<rect width="144" height="144" fill="${C.shade}" fill-opacity="0.55"/>` : "";
    return svgUrl(image(p.custom, 0, 0, 144, 144) + shade + (p.ring ? RING : "") + (p.badge ? badge(p.badge) : ""));
  }
  const colour = p.off ? C.faint : p.on ? C.accent : C.text;
  let body = `<rect width="144" height="144" fill="${C.bg}"/>`;
  if (p.cover) {
    body += image(p.cover, 0, 0, 144, 144);
    body += `<rect width="144" height="144" fill="${C.shade}" fill-opacity="${p.dim ?? 0.55}"/>`;
  }
  const hasLabel = !!p.label;
  const hasBar = p.bar !== undefined && p.bar !== null;
  const cy = hasLabel || hasBar ? 60 : 72;
  if (p.big !== undefined) {
    const size = p.big.length > 5 ? 30 : p.big.length > 3 ? 38 : 46;
    if (p.glyph) body += glyph(p.glyph, colour, 72, 50, 0.38);
    body += line(p.big, 72, p.glyph ? 104 : cy + size * 0.36, { size, max: 132, fill: colour, bold: true, anchor: "middle" });
  } else if (p.glyph) {
    body += glyph(p.glyph, colour, 72, cy, hasLabel || hasBar ? 0.56 : 0.66);
  }
  if (hasLabel) body += line(p.label as string, 72, hasBar ? 118 : 128, { size: 17, max: 128, fill: p.off ? C.faint : C.muted, anchor: "middle", fit: 13 });
  if (hasBar) body += bar(20, 128, 104, 6, p.bar as number);
  if (p.ring) body += RING;
  if (p.badge) body += badge(p.badge);
  return svgUrl(body);
}

/** A slice of a big cover spread over a group of keys (col and row from 0). */
export interface Slice {
  size: number;
  col: number;
  row: number;
}

export interface CoverPic {
  cover: string | null;
  paused: boolean;
  pausedLook: "circle" | "label";
  /** m:ss in a band at the bottom. */
  time?: string | null;
  /** Title over the top of the cover. */
  title?: string | null;
  slice?: Slice | null;
  badge?: Badge | null;
  custom?: string | null;
}

/** Play / Pause: the cover full-bleed, no text unless asked for. */
export function coverPic(p: CoverPic): string {
  if (!p.cover && !p.custom) {
    return keyPic({ glyph: p.paused ? "play" : "pause", badge: p.badge, label: p.time ?? undefined });
  }
  let body = `<rect width="144" height="144" fill="${C.bg}"/>`;
  if (p.custom) body += image(p.custom, 0, 0, 144, 144);
  else if (p.slice && p.slice.size > 1) {
    const n = p.slice.size;
    body += image(p.cover as string, -p.slice.col * 144, -p.slice.row * 144, n * 144, n * 144);
  } else body += image(p.cover as string, 0, 0, 144, 144);
  // One circle for the whole group, on the key holding its middle.
  const middle = !p.slice || p.slice.size <= 1 || (p.slice.col === Math.floor((p.slice.size - 1) / 2) && p.slice.row === Math.floor((p.slice.size - 1) / 2));
  if (p.paused && !p.custom) {
    if (p.pausedLook === "label") {
      body += `<rect x="8" y="8" width="70" height="26" rx="13" fill="${C.shade}" fill-opacity="0.6"/>`;
      body += line("Paused", 43, 26, { size: 15, max: 66, fill: C.text, bold: true, anchor: "middle" });
    } else if (middle) {
      const cx = p.slice && p.slice.size > 1 && p.slice.size % 2 === 0 ? 144 : 72;
      body += `<circle cx="${cx}" cy="${cx}" r="34" fill="${C.shade}" fill-opacity="0.5"/>`;
      body += `<path d="M${cx - 10} ${cx - 17}v34l28-17z" fill="${C.text}"/>`;
    }
  }
  if (p.title) {
    body += `<rect width="144" height="34" fill="${C.shade}" fill-opacity="0.55"/>`;
    body += line(p.title, 10, 23, { size: 16, max: 124, bold: true, fit: 13 });
  }
  if (p.time) {
    body += `<rect y="114" width="144" height="30" fill="${C.shade}" fill-opacity="0.6"/>`;
    body += line(p.time, 72, 135, { size: 19, max: 132, bold: true, anchor: "middle" });
  }
  if (p.badge) body += badge(p.badge);
  return svgUrl(body);
}

export interface InfoPic {
  cover: string | null;
  title: string;
  artist: string;
  /** "1:02 / 3:20". */
  time: string;
  progress: number;
  badge?: Badge | null;
}

/** Song info: title and artist over the dimmed cover, the time and a bar. */
export function infoPic(p: InfoPic): string {
  let body = `<rect width="144" height="144" fill="${C.bg}"/>`;
  if (p.cover) body += image(p.cover, 0, 0, 144, 144) + `<rect width="144" height="144" fill="${C.shade}" fill-opacity="0.62"/>`;
  body += line(p.title, 10, 42, { size: 20, max: 126, bold: true, fit: 15 });
  body += line(p.artist, 10, 66, { size: 16, max: 126, fill: C.muted });
  body += line(p.time, 10, 112, { size: 16, max: 126, bold: true });
  body += bar(10, 124, 124, 6, p.progress);
  if (p.badge) body += badge(p.badge);
  return svgUrl(body);
}

/** Queue: the next song's cover with "Next: title". */
export function nextPic(cover: string | null, title: string | null, badgeKind?: Badge | null): string {
  let body = `<rect width="144" height="144" fill="${C.bg}"/>`;
  if (cover) body += image(cover, 0, 0, 144, 144) + `<rect y="96" width="144" height="48" fill="${C.shade}" fill-opacity="0.65"/>`;
  else body += glyph("queue", title ? C.text : C.faint, 72, 58, 0.56);
  body += line(title ? `Next: ${title}` : "Nothing next", 10, 128, { size: 16, max: 126, bold: !!title, fill: title ? C.text : C.muted, fit: 13 });
  if (badgeKind) body += badge(badgeKind);
  return svgUrl(body);
}

// ---------------------------------------------------------------- dials

/** The touch strip of one dial: 200 x 100. */
export const STRIP_W = 200;
export const STRIP_H = 100;

export interface PlayerView {
  cover: string | null;
  title: string;
  artist: string;
  elapsed: string;
  total: string;
  progress: number;
  paused: boolean;
  /** Volume overlay, 0 to 100 (null: hidden). */
  overlay?: { volume: number; muted: boolean } | null;
  badge?: Badge | null;
}

/**
 * The dial's player view: cover at the left, title (big while short) and
 * artist, the elapsed time left and the song's length right, a thin bar.
 */
export function playerStrip(p: PlayerView): string {
  let body = `<rect width="${STRIP_W}" height="${STRIP_H}" fill="${C.bg}"/>`;
  if (p.cover) body += image(p.cover, 8, 8, 56, 56);
  else body += `<rect x="8" y="8" width="56" height="56" rx="6" fill="${C.bg2}"/>` + glyph("album", C.faint, 36, 36, 0.3);
  if (p.paused && p.cover) body += `<rect x="8" y="8" width="56" height="56" fill="${C.shade}" fill-opacity="0.45"/><path d="M30 25v22l18-11z" fill="${C.text}"/>`;
  // Big text while the title is short; smaller past about 20 characters.
  const big = [...p.title].length <= 20;
  body += line(p.title, 74, big ? 31 : 28, { size: big ? 19 : 15, max: 120, bold: true, fit: big ? 15 : undefined });
  body += line(p.artist, 74, 52, { size: 14, max: 120, fill: C.muted });
  body += line(p.elapsed, 8, 84, { size: 13, max: 60, fill: C.muted });
  body += line(p.total, 192, 84, { size: 13, max: 60, fill: C.muted, anchor: "end" });
  body += bar(8, 90, 184, 4, p.progress);
  if (p.overlay) body += volumeOverlay(p.overlay.volume, p.overlay.muted);
  if (p.badge) body += `<g transform="translate(150 -2) scale(0.6)">${badge(p.badge)}</g>`;
  return svgUrl(body, STRIP_W, STRIP_H);
}

/** The volume overlay, over the whole strip. */
export function volumeOverlay(volume: number, muted: boolean): string {
  const v = Math.max(0, Math.min(100, Math.round(volume)));
  return (
    `<rect width="${STRIP_W}" height="${STRIP_H}" fill="${C.bg}" fill-opacity="0.94"/>` +
    glyph(muted ? "mute" : "volume", C.text, 36, 44, 0.36) +
    line(muted ? "Muted" : `${v}`, 120, 58, { size: muted ? 26 : 40, max: 120, bold: true, anchor: "middle" }) +
    bar(20, 82, 160, 6, muted ? 0 : v / 100)
  );
}

/** The volume dial: a big number and a bar, or "Muted". */
export function volumeStrip(volume: number, muted: boolean, badgeKind?: Badge | null): string {
  let body = `<rect width="${STRIP_W}" height="${STRIP_H}" fill="${C.bg}"/>` + volumeOverlay(volume, muted);
  if (badgeKind) body += `<g transform="translate(150 -2) scale(0.6)">${badge(badgeKind)}</g>`;
  return svgUrl(body, STRIP_W, STRIP_H);
}

/** The Play / Pause dial: cover, title, artist, a centred time and a bar. */
export function nowStrip(p: { cover: string | null; title: string; artist: string; time: string; progress: number; paused: boolean; badge?: Badge | null }): string {
  let body = `<rect width="${STRIP_W}" height="${STRIP_H}" fill="${C.bg}"/>`;
  if (p.cover) body += image(p.cover, 8, 8, 52, 52);
  else body += `<rect x="8" y="8" width="52" height="52" rx="6" fill="${C.bg2}"/>`;
  if (p.paused) body += `<rect x="8" y="8" width="52" height="52" fill="${C.shade}" fill-opacity="0.45"/><path d="M28 23v22l18-11z" fill="${C.text}"/>`;
  body += line(p.title, 70, 30, { size: 16, max: 124, bold: true, fit: 13 });
  body += line(p.artist, 70, 51, { size: 13, max: 124, fill: C.muted });
  body += line(p.time, 100, 80, { size: 14, max: 100, bold: true, anchor: "middle" });
  body += bar(8, 89, 184, 4, p.progress);
  if (p.badge) body += `<g transform="translate(150 -2) scale(0.6)">${badge(p.badge)}</g>`;
  return svgUrl(body, STRIP_W, STRIP_H);
}

export interface BrowseView {
  /** What's browsed: "Playlists"... */
  heading: string;
  items: { title: string; subtitle: string | null; cover: string | null }[];
  /** The one picked. */
  index: number;
  style: "covers" | "list";
}

/** The dial's browser view: covers to turn through, or a short list. */
export function browseStrip(b: BrowseView): string {
  let body = `<rect width="${STRIP_W}" height="${STRIP_H}" fill="${C.bg}"/>`;
  if (b.items.length === 0) {
    body += line(b.heading, 100, 44, { size: 15, max: 180, fill: C.muted, anchor: "middle" });
    body += line("Nothing here", 100, 66, { size: 15, max: 180, anchor: "middle" });
    return svgUrl(body, STRIP_W, STRIP_H);
  }
  const i = Math.max(0, Math.min(b.items.length - 1, b.index));
  const cur = b.items[i];
  if (b.style === "covers") {
    // The picked cover in the middle, its neighbours dimmed at the sides.
    for (const [d, x, s] of [
      [-1, -18, 50],
      [1, 168, 50],
    ] as const) {
      const it = b.items[i + d];
      if (!it) continue;
      body += it.cover ? image(it.cover, x, 12, s, s) : `<rect x="${x}" y="12" width="${s}" height="${s}" fill="${C.bg2}"/>`;
      body += `<rect x="${x}" y="12" width="${s}" height="${s}" fill="${C.bg}" fill-opacity="0.6"/>`;
    }
    body += cur.cover ? image(cur.cover, 66, 4, 68, 68) : `<rect x="66" y="4" width="68" height="68" rx="6" fill="${C.bg2}"/>` + glyph("playlist", C.faint, 100, 38, 0.3);
    body += line(cur.title, 100, 90, { size: 14, max: 186, bold: true, anchor: "middle" });
  } else {
    body += line(b.heading, 8, 18, { size: 12, max: 140, fill: C.muted });
    body += line(`${i + 1} / ${b.items.length}`, 192, 18, { size: 12, max: 60, fill: C.muted, anchor: "end" });
    for (const [d, y] of [
      [-1, 40],
      [0, 64],
      [1, 88],
    ] as const) {
      const it = b.items[i + d];
      if (!it) continue;
      if (d === 0) body += `<rect x="4" y="${y - 18}" width="192" height="24" rx="5" fill="${C.bg2}"/>`;
      body += line(it.title, 12, y, { size: d === 0 ? 15 : 13, max: 176, bold: d === 0, fill: d === 0 ? C.text : C.muted });
    }
  }
  return svgUrl(body, STRIP_W, STRIP_H);
}

/** A list dial (queue, devices): the picked one big, with a counter. */
export function listStrip(heading: string, title: string | null, sub: string | null, cover: string | null, pos: string, active = false): string {
  let body = `<rect width="${STRIP_W}" height="${STRIP_H}" fill="${C.bg}"/>`;
  body += line(heading, 8, 18, { size: 12, max: 140, fill: C.muted });
  body += line(pos, 192, 18, { size: 12, max: 60, fill: C.muted, anchor: "end" });
  const x = cover ? 74 : 8;
  if (cover) body += image(cover, 8, 28, 58, 58);
  body += line(title ?? "Nothing here", x, 52, { size: 16, max: 192 - x, bold: true, fill: active ? C.accent : C.text });
  if (sub) body += line(sub, x, 74, { size: 13, max: 192 - x, fill: C.muted });
  return svgUrl(body, STRIP_W, STRIP_H);
}

/** A message on a dial ("Platter isn't running"). */
export function messageStrip(title: string, sub: string, glyphName = "dial"): string {
  const body =
    `<rect width="${STRIP_W}" height="${STRIP_H}" fill="${C.bg}"/>` +
    glyph(glyphName, C.faint, 36, 50, 0.34) +
    line(title, 76, 46, { size: 15, max: 118, bold: true, fit: 12 }) +
    line(sub, 76, 68, { size: 13, max: 118, fill: C.muted, fit: 11 });
  return svgUrl(body, STRIP_W, STRIP_H);
}

// ---------------------------------------------------------------- Neo

/** The Stream Deck Neo info bar: 232 x 50. */
export const NEO_W = 232;
export const NEO_H = 50;

/** Now playing on the Neo info bar. */
export function neoBar(p: { cover: string | null; title: string; artist: string; progress: number; paused: boolean }): string {
  let body = `<rect width="${NEO_W}" height="${NEO_H}" fill="${C.bg}"/>`;
  let x = 6;
  if (p.cover) {
    body += image(p.cover, 4, 4, 42, 42);
    x = 54;
  }
  body += line(p.title, x, 21, { size: 14, max: NEO_W - x - 6, bold: true, fit: 12 });
  body += line(p.artist, x, 38, { size: 12, max: NEO_W - x - 6, fill: C.muted });
  body += bar(x, 44, NEO_W - x - 6, 3, p.progress);
  return svgUrl(body, NEO_W, NEO_H);
}

/** Read the SVG back out of a data URL (tests). */
export const svgOf = (url: string): string => decodeURIComponent(url.replace(/^data:image\/svg\+xml;charset=utf8,/, ""));
