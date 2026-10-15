/**
 * A month's recap as a share card: the month, minutes listened and the
 * change on the month before, a block of the month's covers, top artists
 * and songs, and the month's daily listening as bars. Laid out as plain
 * data (tests check it), then drawn with the share card's text and mark
 * drawing. Unreleased songs and albums never go on a card.
 */
import { CARD_SIZES, drawMark, drawText, fontFor, roundRect, wrapText, type CardFormat, type CardLayout, type CardText, type Measure } from "./shareCard";
import { change, minutes, monthName } from "./stats";
import type { MonthRecap } from "./statsTypes";

export interface RecapCardContent {
  month: string;
  minutes: number;
  /** "+12% on August" or null. */
  change: string | null;
  artists: string[];
  songs: { title: string; artist: string }[];
  /** Listening time per day. */
  daily: number[];
  /** The card's colour, picked from the month (the dialog swaps in the first cover's). */
  color: string;
  /** Up to four covers of the month's top albums; unreleased ones never. */
  covers: string[];
}

export interface RecapLayout {
  width: number;
  height: number;
  texts: CardText[];
  bars: { x: number; y: number; w: number; h: number }[];
  /** Where each cover goes: a 2 by 2 block at the top right. */
  covers: { x: number; y: number; size: number; radius: number }[];
  mark: CardLayout["mark"];
}

const PAD = 80;
/** One colour per month of the year, from Platter's warm range. */
const MONTH_COLORS = ["#7A4B3A", "#6B4E71", "#3F5F73", "#3E6B57", "#6E7340", "#8A6A35", "#8B4F3B", "#5C4A82", "#3A6470", "#7A5A3A", "#6A3F4E", "#3F4F6B"];

/** What a recap card may show: unreleased songs and albums are left out. */
export function recapContent(r: MonthRecap): RecapCardContent {
  const [, m] = r.month.split("-").map(Number);
  const prevName = monthName(prevMonth(r.month)).split(" ")[0];
  const pct = change(r.totals.ms, r.prevMs);
  return {
    month: monthName(r.month),
    minutes: minutes(r.totals.ms),
    change: pct ? `${pct} on ${prevName}` : null,
    artists: r.topArtists.map((a) => a.name),
    songs: r.topSongs.filter((s) => !s.unreleased).map((s) => ({ title: s.title, artist: s.artist })),
    daily: r.daily.map((d) => d.ms),
    color: MONTH_COLORS[(m - 1 + 12) % 12],
    covers: r.topAlbums
      .filter((a) => !a.unreleased && a.thumb)
      .map((a) => a.thumb as string)
      .slice(0, 4),
  };
}

function prevMonth(month: string): string {
  const [y, m] = month.split("-").map(Number);
  return m === 1 ? `${y - 1}-12` : `${y}-${String(m - 1).padStart(2, "0")}`;
}

const t = (lines: string[], x: number, y: number, size: number, weight: CardText["weight"], color: CardText["color"], align: CardText["align"] = "left", lineHeight = 1.2): CardText => ({
  lines,
  x,
  y,
  size,
  weight,
  color,
  align,
  lineHeight,
});

/** Where everything goes. `y` of a text is its first baseline. */
export function layoutRecapCard(format: CardFormat, c: RecapCardContent, measure: Measure): RecapLayout {
  const { width, height } = CARD_SIZES[format];
  const story = format === "story";
  const inner = width - PAD * 2;
  const texts: CardText[] = [];
  let y = story ? 170 : 120;
  texts.push(t([c.month], PAD, y, story ? 64 : 52, 800, "primary"));
  y += story ? 60 : 50;
  texts.push(t(["My month in music"], PAD, y, story ? 34 : 28, 400, "secondary"));
  y += story ? 180 : 140;
  const big = story ? 180 : 128;
  texts.push(t([c.minutes.toLocaleString()], PAD, y, big, 800, "primary", "left", 1));
  y += story ? 64 : 50;
  texts.push(t([c.change ? `minutes listened · ${c.change}` : "minutes listened"], PAD, y, story ? 36 : 30, 400, "secondary"));

  // Daily bars under the number.
  const bars: RecapLayout["bars"] = [];
  const barTop = y + (story ? 50 : 36);
  const barH = story ? 140 : 90;
  const n = c.daily.length;
  if (n) {
    const max = Math.max(1, ...c.daily);
    const gap = 6;
    const w = (inner - gap * (n - 1)) / n;
    c.daily.forEach((v, i) => {
      const h = Math.max(4, (v / max) * barH);
      bars.push({ x: PAD + i * (w + gap), y: barTop + barH - h, w, h });
    });
  }
  y = barTop + barH + (story ? 96 : 72);

  // Top artists and songs: one column each on a story, side by side on a square.
  const rows = story ? 5 : 3;
  const head = story ? 32 : 26;
  const item = story ? 44 : 34;
  const sub = story ? 30 : 24;
  const colW = story ? inner : (inner - 48) / 2;
  const markSize = story ? 56 : 48;
  const markY = height - PAD - markSize;
  // Rows stop short of the mark; a row that wouldn't fit is left off.
  const limit = markY - (story ? 48 : 32);
  const list = (title: string, x: number, top: number, entries: { line: string; sub?: string }[]) => {
    let yy = top;
    texts.push(t([title.toUpperCase()], x, yy, head, 700, "muted"));
    yy += head + (story ? 28 : 20);
    entries.slice(0, rows).forEach((e, i) => {
      const need = item * 1.1 + (e.sub ? sub * 1.25 : 0);
      if (yy + need > limit) return;
      const num = `${i + 1}`;
      const nx = x + (story ? 52 : 40);
      texts.push(t([num], x, yy + item * 0.85, item, 800, "secondary"));
      texts.push(t(wrapText(e.line, colW - (nx - x), item, 700, measure, 1), nx, yy + item * 0.85, item, 700, "primary"));
      yy += item * 1.1;
      if (e.sub) {
        texts.push(t(wrapText(e.sub, colW - (nx - x), sub, 400, measure, 1), nx, yy + sub * 0.85, sub, 400, "secondary"));
        yy += sub * 1.25;
      }
      yy += story ? 18 : 12;
    });
    return yy;
  };
  const artists = c.artists.map((a) => ({ line: a }));
  const songs = c.songs.map((s) => ({ line: s.title, sub: s.artist }));
  if (story) {
    y = list("Top artists", PAD, y, artists) + 56;
    list("Top songs", PAD, y, songs);
  } else {
    list("Top artists", PAD, y, artists);
    list("Top songs", PAD + colW + 48, y, songs);
  }
  // The month's covers, 2 by 2, top right beside the month and minutes.
  const cell = story ? 150 : 104;
  const gap = story ? 14 : 10;
  const block = cell * 2 + gap;
  const covers = c.covers
    .slice(0, 4)
    .map((_, i) => ({ x: width - PAD - block + (i % 2) * (cell + gap), y: (story ? 96 : 64) + Math.floor(i / 2) * (cell + gap), size: cell, radius: story ? 14 : 10 }));
  return { width, height, texts, bars, covers, mark: { x: PAD, y: markY, size: markSize } };
}

/** Draw a laid-out recap card. `covers` are the loaded images, in the content's order (null when one didn't load). */
export function drawRecapCard(ctx: CanvasRenderingContext2D, l: RecapLayout, c: RecapCardContent, covers: (CanvasImageSource | null)[] = []) {
  ctx.fillStyle = c.color;
  ctx.fillRect(0, 0, l.width, l.height);
  const shade = ctx.createLinearGradient(0, 0, l.width, l.height);
  shade.addColorStop(0, "rgba(14,13,12,0.05)");
  shade.addColorStop(0.7, "rgba(14,13,12,0.55)");
  shade.addColorStop(1, "rgba(14,13,12,0.85)");
  ctx.fillStyle = shade;
  ctx.fillRect(0, 0, l.width, l.height);
  ctx.fillStyle = "rgba(255,255,255,0.85)";
  for (const b of l.bars) {
    roundRect(ctx, b.x, b.y, b.w, b.h, Math.min(4, b.w / 2));
    ctx.fill();
  }
  l.covers.forEach((b, i) => {
    const img = covers[i];
    ctx.save();
    roundRect(ctx, b.x, b.y, b.size, b.size, b.radius);
    ctx.fillStyle = "rgba(14,13,12,0.35)";
    ctx.fill();
    if (img) {
      ctx.clip();
      ctx.drawImage(img, b.x, b.y, b.size, b.size);
    }
    ctx.restore();
  });
  ctx.font = fontFor(10, 400);
  for (const tx of l.texts) drawText(ctx, tx);
  drawMark(ctx, l.mark);
}
