/**
 * Share cards: an image of a song, album or playlist (or a few lyric lines)
 * to post elsewhere. Laid out here as plain data (so tests can check it),
 * then drawn on a canvas. Everything is made on this computer.
 *
 * Two formats: a 9:16 story card (1080 x 1920) and a square card
 * (1080 x 1080). Both have the cover on a background tinted from it, the
 * title and artist, and the Platter mark; songs can add a progress bar at
 * the current time, released music a song.link line, and any card the QR
 * code for its Platter link.
 */
import { formatDuration } from "./format";

export type CardFormat = "story" | "square";

export interface CardContent {
  title: string;
  /** The artist line, or what the playlist is. */
  subtitle: string;
  thumb: string | null;
  /** A lyric card: up to four lines in place of the big cover. */
  lyrics?: string[] | null;
  /** Songs: where it's at, drawn as a progress bar. */
  progress?: { atMs: number; durationMs: number } | null;
  /** Released music: the song.link line. */
  songLink?: string | null;
  /** The QR code's modules ('1' dark). */
  qr?: { size: number; rows: string[] } | null;
}

export interface CardText {
  lines: string[];
  x: number;
  y: number;
  size: number;
  weight: 400 | 600 | 700 | 800;
  color: "primary" | "secondary" | "muted";
  align: "left" | "center";
  lineHeight: number;
}

export interface CardLayout {
  width: number;
  height: number;
  art: { x: number; y: number; size: number; radius: number } | null;
  texts: { title: CardText; subtitle: CardText; lyrics: CardText | null; link: CardText | null };
  progress: { x: number; y: number; width: number; frac: number; left: string; right: string; timeSize: number } | null;
  qr: { x: number; y: number; size: number } | null;
  mark: { x: number; y: number; size: number };
}

/** Width of `text` in pixels for a font size and weight. */
export type Measure = (text: string, size: number, weight: number) => number;

export const CARD_SIZES: Record<CardFormat, { width: number; height: number }> = {
  story: { width: 1080, height: 1920 },
  square: { width: 1080, height: 1080 },
};

/** Lines of a lyric card. */
export const LYRIC_LINES_MAX = 4;
const PAD = 80;

/**
 * Break `text` into at most `max` lines no wider than `width`; a last line
 * that doesn't fit ends in an ellipsis. Long single words are cut too.
 */
export function wrapText(text: string, width: number, size: number, weight: number, measure: Measure, max: number): string[] {
  const words = text.trim().split(/\s+/).filter(Boolean);
  const lines: string[] = [];
  let cur = "";
  for (const w of words) {
    const next = cur ? `${cur} ${w}` : w;
    if (measure(next, size, weight) <= width) cur = next;
    else {
      if (cur) lines.push(cur);
      cur = w;
    }
  }
  if (cur) lines.push(cur);
  if (lines.length <= max) return lines.map((l) => fit(l, width, size, weight, measure));
  const kept = lines.slice(0, max);
  kept[max - 1] = fit(`${kept[max - 1]} ${lines.slice(max).join(" ")}`, width, size, weight, measure, true);
  return kept.map((l, i) => (i < max - 1 ? fit(l, width, size, weight, measure) : l));
}

/** Shorten one line with an ellipsis until it fits. */
function fit(line: string, width: number, size: number, weight: number, measure: Measure, force = false): string {
  if (!force && measure(line, size, weight) <= width) return line;
  let s = line;
  while (s.length > 1 && measure(`${s}…`, size, weight) > width) s = s.slice(0, -1);
  return `${s.trimEnd()}…`;
}

function text(lines: string[], x: number, y: number, size: number, weight: CardText["weight"], color: CardText["color"], align: CardText["align"], lineHeight = 1.2): CardText {
  return { lines, x, y, size, weight, color, align, lineHeight };
}

/** Where everything goes on a card. `y` of a text is its first baseline. */
export function layoutCard(format: CardFormat, c: CardContent, measure: Measure): CardLayout {
  const { width, height } = CARD_SIZES[format];
  const story = format === "story";
  const lyrics = c.lyrics?.filter((l) => l.trim()).slice(0, LYRIC_LINES_MAX) ?? null;
  const qrSize = c.qr ? (story ? 200 : 140) : 0;
  const qr = c.qr ? { x: width - PAD - qrSize, y: height - PAD - qrSize, size: qrSize } : null;
  const mark = { x: PAD, y: height - PAD - (story ? 56 : 48), size: story ? 56 : 48 };

  if (lyrics?.length) {
    // A small cover with the title beside it, then the lines, big.
    const artSize = story ? 140 : 112;
    const top = story ? 300 : PAD;
    const tx = PAD + artSize + 32;
    const tw = width - tx - PAD;
    const tSize = story ? 44 : 38;
    const sSize = story ? 34 : 30;
    const title = text(wrapText(c.title, tw, tSize, 700, measure, 1), tx, top + artSize / 2 - 6, tSize, 700, "primary", "left");
    const subtitle = text(wrapText(c.subtitle, tw, sSize, 400, measure, 1), tx, top + artSize / 2 + sSize + 6, sSize, 400, "secondary", "left");
    const lSize = story ? 72 : 56;
    const bottom = (qr ? qr.y : mark.y) - 48;
    const lyricTop = top + artSize + (story ? 100 : 72);
    // Each line wraps on its own; drop lines that would run into the footer.
    const lines: string[] = [];
    for (const l of lyrics) {
      const wrapped = wrapText(l, width - PAD * 2, lSize, 800, measure, 3);
      if (lyricTop + (lines.length + wrapped.length) * lSize * 1.25 > bottom) break;
      lines.push(...wrapped);
    }
    return {
      width,
      height,
      art: { x: PAD, y: top, size: artSize, radius: 12 },
      texts: {
        title,
        subtitle,
        lyrics: text(lines, PAD, lyricTop + lSize, lSize, 800, "primary", "left", 1.25),
        link: null,
      },
      progress: null,
      qr,
      mark,
    };
  }

  const artSize = story ? 800 : 560;
  const art = { x: (width - artSize) / 2, y: story ? 260 : 96, size: artSize, radius: story ? 20 : 16 };
  const textWidth = story ? artSize : 760;
  const tSize = story ? 68 : 52;
  const sSize = story ? 40 : 32;
  let y = art.y + artSize + (story ? 110 : 80);
  const titleLines = wrapText(c.title, textWidth, tSize, 800, measure, 2);
  const title = text(titleLines, width / 2, y, tSize, 800, "primary", "center", 1.15);
  y += (titleLines.length - 1) * tSize * 1.15 + sSize + (story ? 24 : 16);
  const subtitle = text(wrapText(c.subtitle, textWidth, sSize, 400, measure, 1), width / 2, y, sSize, 400, "secondary", "center");
  y += story ? 56 : 40;
  let progress: CardLayout["progress"] = null;
  if (c.progress && c.progress.durationMs > 0) {
    const pw = story ? artSize : 560;
    const frac = Math.min(1, Math.max(0, c.progress.atMs / c.progress.durationMs));
    progress = {
      x: (width - pw) / 2,
      y,
      width: pw,
      frac,
      left: formatDuration(c.progress.atMs),
      right: formatDuration(c.progress.durationMs),
      timeSize: story ? 28 : 24,
    };
    y += story ? 90 : 70;
  }
  const lSize = story ? 30 : 24;
  const link = c.songLink ? text([fit(c.songLink, textWidth, lSize, 400, measure)], width / 2, y + lSize, lSize, 400, "muted", "center") : null;
  return { width, height, art, texts: { title, subtitle, lyrics: null, link }, progress, qr, mark };
}

// ---------------------------------------------------------------- drawing

export interface CardColors {
  /** The cover's most vivid colour (the tint). */
  vivid: string;
}

const TEXT_COLOR = { primary: "#FFFFFF", secondary: "rgba(255,255,255,0.78)", muted: "rgba(255,255,255,0.6)" };
const FONT = '"Inter Variable", "Inter", system-ui, -apple-system, "Segoe UI", sans-serif';

export const fontFor = (size: number, weight: number) => `${weight} ${size}px ${FONT}`;

/** A Measure backed by a canvas context. */
export function canvasMeasure(ctx: CanvasRenderingContext2D): Measure {
  return (t, size, weight) => {
    ctx.font = fontFor(size, weight);
    return ctx.measureText(t).width;
  };
}

function roundRect(ctx: CanvasRenderingContext2D, x: number, y: number, w: number, h: number, r: number) {
  ctx.beginPath();
  ctx.moveTo(x + r, y);
  ctx.arcTo(x + w, y, x + w, y + h, r);
  ctx.arcTo(x + w, y + h, x, y + h, r);
  ctx.arcTo(x, y + h, x, y, r);
  ctx.arcTo(x, y, x + w, y, r);
  ctx.closePath();
}

function drawText(ctx: CanvasRenderingContext2D, t: CardText) {
  ctx.font = fontFor(t.size, t.weight);
  ctx.fillStyle = TEXT_COLOR[t.color];
  ctx.textAlign = t.align;
  ctx.textBaseline = "alphabetic";
  t.lines.forEach((l, i) => ctx.fillText(l, t.x, t.y + i * t.size * t.lineHeight));
}

/** The Platter mark (a record seen from above) and the word. */
function drawMark(ctx: CanvasRenderingContext2D, m: CardLayout["mark"]) {
  const s = m.size / 32;
  ctx.save();
  ctx.translate(m.x, m.y);
  ctx.scale(s, s);
  const ring = (r: number, fill: string | null, stroke: string | null) => {
    ctx.beginPath();
    ctx.arc(16, 16, r, 0, Math.PI * 2);
    if (fill) {
      ctx.fillStyle = fill;
      ctx.fill();
    }
    if (stroke) {
      ctx.strokeStyle = stroke;
      ctx.lineWidth = 1;
      ctx.stroke();
    }
  };
  ring(15, "#1C1916", "#3A332C");
  ring(11.5, null, "#2C2722");
  ring(8.5, null, "#2C2722");
  ring(5.5, "#ECE4D3", null);
  ring(1.3, "#121110", null);
  ctx.beginPath();
  ctx.moveTo(24.5, 5.5);
  ctx.lineTo(19.5, 13);
  ctx.strokeStyle = "#B8AFA4";
  ctx.lineWidth = 1.6;
  ctx.lineCap = "round";
  ctx.stroke();
  ctx.restore();
  ctx.font = fontFor(Math.round(m.size * 0.62), 700);
  ctx.fillStyle = "#FFFFFF";
  ctx.textAlign = "left";
  ctx.textBaseline = "middle";
  ctx.fillText("Platter", m.x + m.size + m.size * 0.3, m.y + m.size / 2 + 1);
}

function drawQr(ctx: CanvasRenderingContext2D, box: NonNullable<CardLayout["qr"]>, qr: NonNullable<CardContent["qr"]>) {
  const quiet = 2;
  const cell = box.size / (qr.size + quiet * 2);
  ctx.fillStyle = "#FFFFFF";
  roundRect(ctx, box.x, box.y, box.size, box.size, box.size * 0.06);
  ctx.fill();
  ctx.fillStyle = "#000000";
  qr.rows.forEach((row, y) => {
    for (let x = 0; x < row.length; x++) {
      if (row[x] === "1") ctx.fillRect(box.x + (x + quiet) * cell, box.y + (y + quiet) * cell, Math.ceil(cell), Math.ceil(cell));
    }
  });
}

/** Draw a laid-out card. `art` is the loaded cover, or null for a plain square. */
export function drawCard(ctx: CanvasRenderingContext2D, l: CardLayout, c: CardContent, art: CanvasImageSource | null, colors: CardColors) {
  // Background: the tint, deepening towards near black at the bottom.
  ctx.fillStyle = colors.vivid;
  ctx.fillRect(0, 0, l.width, l.height);
  const shade = ctx.createLinearGradient(0, 0, 0, l.height);
  shade.addColorStop(0, "rgba(14,13,12,0.05)");
  shade.addColorStop(0.6, "rgba(14,13,12,0.5)");
  shade.addColorStop(1, "rgba(14,13,12,0.88)");
  ctx.fillStyle = shade;
  ctx.fillRect(0, 0, l.width, l.height);

  if (l.art) {
    const a = l.art;
    ctx.save();
    ctx.shadowColor = "rgba(0,0,0,0.45)";
    ctx.shadowBlur = a.size * 0.08;
    ctx.shadowOffsetY = a.size * 0.03;
    roundRect(ctx, a.x, a.y, a.size, a.size, a.radius);
    ctx.fillStyle = "#201D1A";
    ctx.fill();
    ctx.restore();
    if (art) {
      ctx.save();
      roundRect(ctx, a.x, a.y, a.size, a.size, a.radius);
      ctx.clip();
      ctx.drawImage(art, a.x, a.y, a.size, a.size);
      ctx.restore();
    }
  }
  drawText(ctx, l.texts.title);
  drawText(ctx, l.texts.subtitle);
  if (l.texts.lyrics) drawText(ctx, l.texts.lyrics);
  if (l.progress) {
    const p = l.progress;
    const h = 8;
    ctx.fillStyle = "rgba(255,255,255,0.28)";
    roundRect(ctx, p.x, p.y, p.width, h, h / 2);
    ctx.fill();
    ctx.fillStyle = "#FFFFFF";
    roundRect(ctx, p.x, p.y, Math.max(h, p.width * p.frac), h, h / 2);
    ctx.fill();
    ctx.font = fontFor(p.timeSize, 600);
    ctx.fillStyle = TEXT_COLOR.secondary;
    ctx.textBaseline = "top";
    ctx.textAlign = "left";
    ctx.fillText(p.left, p.x, p.y + h + 14);
    ctx.textAlign = "right";
    ctx.fillText(p.right, p.x + p.width, p.y + h + 14);
  }
  if (l.texts.link) drawText(ctx, l.texts.link);
  if (l.qr && c.qr) drawQr(ctx, l.qr, c.qr);
  drawMark(ctx, l.mark);
}
