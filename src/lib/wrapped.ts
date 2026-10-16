/**
 * Wrapped and Top Songs, the parts with no React: each year's colour and
 * its record-groove pattern (seeded by the year, so a year always looks the
 * same), the sentences that lead each card, what the summary card may show
 * and where everything on it goes, and the small rules the pages share
 * (when Wrapped shows, what search finds). Pure, so tests check them.
 *
 * Unreleased songs count everywhere on this computer and are marked; on the
 * exported summary card they're left off unless the person turns that on.
 */
import { CARD_SIZES, drawMark, drawText, fontFor, roundRect, wrapText, type CardLayout, type CardText, type Measure } from "./shareCard";
import { change, hourLabel, minutes, parseDay } from "./stats";
import type { Personality, TopSongsIndex, Wrapped } from "./statsTypes";

// ---------------------------------------------------------------- colour

export interface YearTone {
  /** The banner and card ground. */
  deep: string;
  /** Type and marks on dark: the card's one accent. */
  light: string;
}

/**
 * Deep, warm studio colours, each with the tint that reads on near black.
 * In year order (the year modulo 8), so years next to each other, and two
 * apart, never look alike.
 */
export const YEAR_TONES: YearTone[] = [
  { deep: "#8B4F3B", light: "#F2A584" },
  { deep: "#3A6470", light: "#93D3DE" },
  { deep: "#8A6A35", light: "#EDC984" },
  { deep: "#5C4A82", light: "#BCAEF0" },
  { deep: "#3E6B57", light: "#9FD8B9" },
  { deep: "#7E3F4C", light: "#F0A0B0" },
  { deep: "#3F5F73", light: "#9CC6E0" },
  { deep: "#6B4E71", light: "#D6B0DE" },
];

/** All-Time's tone: the house ivory on warm graphite. */
export const ALL_TIME_TONE: YearTone = { deep: "#3B3631", light: "#ECE4D3" };

/** A small seeded random source (the same one the demo history uses). */
export function seeded(seed: number): () => number {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

/** A year's colour. Neighbouring years never share one. */
export function yearTone(year: number | null): YearTone {
  if (year == null) return ALL_TIME_TONE;
  return YEAR_TONES[((year % YEAR_TONES.length) + YEAR_TONES.length) % YEAR_TONES.length];
}

// ---------------------------------------------------------------- pattern

export interface Groove {
  r: number;
  /** Opacity of the line, 0 to 1. */
  alpha: number;
}

/**
 * A record seen from above, partly off the edge: grooves in bands with a
 * quiet gap between songs, the label and the spindle hole. Sizes are in
 * the box's pixels.
 */
export interface YearPattern {
  cx: number;
  cy: number;
  /** The record's edge. */
  radius: number;
  label: number;
  hole: number;
  grooves: Groove[];
  /** How many songs the grooves are cut into. */
  bands: number;
}

/**
 * The pattern for a year (null: All-Time) in a `w` by `h` box. A wide box
 * (a banner) gets a record bigger than itself, cut by the edges; a squarer
 * one (a card, a tile) a smaller record high on the right, clear of type
 * in the lower left.
 */
export function yearPattern(year: number | null, w: number, h: number): YearPattern {
  const rand = seeded(year == null ? 0x51a7 : year * 7919);
  const wide = w / h >= 2.4;
  const radius = h * (wide ? 1.05 + rand() * 0.35 : 0.58 + rand() * 0.12);
  const cx = w * (wide ? 0.66 + rand() * 0.2 : 0.72 + rand() * 0.1);
  const cy = h * (wide ? 0.3 + rand() * 0.45 : 0.3 + rand() * 0.12);
  const label = radius * (0.3 + rand() * 0.04);
  const hole = Math.max(3, label * 0.06);
  const bands = 4 + Math.floor(rand() * 5);
  // Band widths vary like song lengths do.
  const weights = Array.from({ length: bands }, () => 0.6 + rand());
  const total = weights.reduce((a, n) => a + n, 0);
  const pitch = Math.max(3, h / 90);
  const gap = pitch * 3.2;
  const inner = label + pitch * 4;
  const usable = radius - inner - gap * (bands - 1);
  const grooves: Groove[] = [];
  let r = inner;
  weights.forEach((wt, i) => {
    const end = r + (usable * wt) / total;
    for (; r < end; r += pitch) grooves.push({ r, alpha: 0.07 + 0.05 * ((Math.floor(r / pitch) % 3) / 2) });
    if (i < bands - 1) r = end + gap;
  });
  return { cx, cy, radius, label, hole, grooves, bands };
}

/** The printed ring on the label, as a share of its radius. */
export const LABEL_RING = 0.74;

/** Draw a pattern on a canvas over `deep`, inside the box at (x, y). `labelArt` fills the label. */
export function drawYearPattern(ctx: CanvasRenderingContext2D, p: YearPattern, tone: YearTone, x: number, y: number, w: number, h: number, labelArt: CanvasImageSource | null = null) {
  ctx.save();
  ctx.beginPath();
  ctx.rect(x, y, w, h);
  ctx.clip();
  ctx.fillStyle = tone.deep;
  ctx.fillRect(x, y, w, h);
  ctx.beginPath();
  ctx.arc(x + p.cx, y + p.cy, p.radius, 0, Math.PI * 2);
  ctx.fillStyle = "rgba(14,13,12,0.38)";
  ctx.fill();
  ctx.lineWidth = Math.max(1, h / 360);
  for (const g of p.grooves) {
    ctx.beginPath();
    ctx.arc(x + p.cx, y + p.cy, g.r, 0, Math.PI * 2);
    ctx.strokeStyle = `rgba(255,240,220,${g.alpha})`;
    ctx.stroke();
  }
  ctx.beginPath();
  ctx.arc(x + p.cx, y + p.cy, p.label, 0, Math.PI * 2);
  ctx.fillStyle = tone.light;
  ctx.fill();
  if (labelArt) {
    ctx.save();
    ctx.clip();
    ctx.drawImage(labelArt, x + p.cx - p.label, y + p.cy - p.label, p.label * 2, p.label * 2);
    ctx.restore();
  }
  ctx.beginPath();
  ctx.arc(x + p.cx, y + p.cy, p.label * LABEL_RING, 0, Math.PI * 2);
  ctx.strokeStyle = labelArt ? "rgba(255,240,220,0.35)" : `${tone.deep}59`;
  ctx.lineWidth = Math.max(1, p.label / 60);
  ctx.stroke();
  ctx.beginPath();
  ctx.arc(x + p.cx, y + p.cy, p.hole, 0, Math.PI * 2);
  ctx.fillStyle = "#0E0D0C";
  ctx.fill();
  ctx.restore();
}

// ---------------------------------------------------------------- when

/** A query's words with the years taken out, and the years. */
function queryParts(query: string): { text: string; years: number[] } {
  const words = query.toLowerCase().replace(/[^a-z0-9 ]+/g, " ").replace(/\s+/g, " ").trim().split(" ").filter(Boolean);
  const years = words.map(Number).filter((n) => Number.isInteger(n) && n >= 1900 && n <= 9999);
  return { text: words.filter((w) => !/^\d+$/.test(w)).join(" "), years };
}

const ASKS = /^(your )?(top( songs?)?|wrapped|all time( top( songs?)?)?|most played)$/;
const TYPED = ["top songs", "your top songs", "wrapped", "all time top songs"];

/** Whether a query is about Top Songs lists ("top songs", "wrapped", "top 2025", a prefix of those while typing). */
export function asksForTopSongs(query: string): boolean {
  const { text } = queryParts(query);
  return ASKS.test(text) || (text.length >= 3 && TYPED.some((p) => p.startsWith(text)));
}

/** The lists search shows for a query (null: All-Time), newest first. A year in the query picks that year. */
export function topSongsMatches(query: string, ix: Pick<TopSongsIndex, "years" | "allTime">): (number | null)[] {
  if (!asksForTopSongs(query)) return [];
  const { text, years } = queryParts(query);
  const list: (number | null)[] = years.length ? ix.years.filter((y) => years.includes(y)) : [...ix.years];
  if (!years.length && ix.allTime && !text.startsWith("wrap")) list.push(null);
  return list;
}

// ---------------------------------------------------------------- words

const MONTHS = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];

/** "March 14". */
export function dayName(day: string): string {
  const d = parseDay(day);
  return `${MONTHS[d.getMonth()]} ${d.getDate()}`;
}

/** "6 hours and 12 minutes", "45 minutes", "1 hour". */
export function hoursAndMinutes(ms: number): string {
  const total = minutes(ms);
  const h = Math.floor(total / 60);
  const m = total % 60;
  const hs = `${h} ${h === 1 ? "hour" : "hours"}`;
  const ms_ = `${m} ${m === 1 ? "minute" : "minutes"}`;
  if (!h) return ms_;
  return m ? `${hs} and ${ms_}` : hs;
}

/** "28 days and 14 hours", for a year's listening. */
export function daysAndHours(ms: number): string {
  const hoursTotal = Math.floor(minutes(ms) / 60);
  const d = Math.floor(hoursTotal / 24);
  const h = hoursTotal % 24;
  if (!d) return hoursAndMinutes(ms);
  const ds = `${d} ${d === 1 ? "day" : "days"}`;
  return h ? `${ds} and ${h} ${h === 1 ? "hour" : "hours"}` : ds;
}

/** "Indie", "Indie and Jazz", "Indie, Jazz and Soul". */
export function andList(items: string[]): string {
  if (items.length <= 1) return items[0] ?? "";
  return `${items.slice(0, -1).join(", ")} and ${items[items.length - 1]}`;
}

const pct = (share: number) => Math.round(share * 100);

/** The minutes card: how long, and the change on the year before. */
export function minutesWords(w: Wrapped): { lead: string; big: string; change: string } {
  const p = change(w.totals.ms, w.prevMs);
  const changeLine = !p
    ? `The first year in your history.`
    : p === "0%" || p === "+0%"
      ? `About the same as ${w.year - 1}.`
      : p.startsWith("-")
        ? `${p.slice(1)} less than ${w.year - 1}.`
        : `${p.slice(1)} more than ${w.year - 1}.`;
  return { lead: `You spent ${daysAndHours(w.totals.ms)} with music in ${w.year}.`, big: minutes(w.totals.ms).toLocaleString(), change: changeLine };
}

export function artistWords(w: Wrapped): { lead: string; sub: string } | null {
  const a = w.topArtists[0];
  if (!a) return null;
  const every = a.plays > 0 ? Math.round(w.totals.plays / a.plays) : 0;
  const lead =
    every >= 2 && every <= 25
      ? `1 in every ${every} songs you played was by ${a.name}.`
      : every < 2
        ? `Most of what you played was ${a.name}.`
        : `${a.name} was the artist you played most.`;
  return { lead, sub: `${a.plays.toLocaleString()} plays, ${minutes(a.ms).toLocaleString()} minutes.` };
}

export function songWords(w: Wrapped): { lead: string; sub: string } | null {
  const s = w.topSongs[0];
  if (!s) return null;
  const next = w.topSongs[1];
  const ahead = next ? s.plays - next.plays : 0;
  return {
    lead: `You played “${s.title}” ${s.plays === 1 ? "once" : `${s.plays.toLocaleString()} times`}.`,
    sub: ahead > 0 ? `${ahead.toLocaleString()} more ${ahead === 1 ? "play" : "plays"} than anything else you played in ${w.year}.` : `Your most played song of ${w.year}.`,
  };
}

export function albumWords(w: Wrapped): { lead: string; sub: string; genres: string | null } | null {
  const a = w.topAlbums[0];
  if (!a) return null;
  const g = w.topGenres.map((x) => x.name);
  const genres = g.length === 0 ? null : g.length === 1 ? `Mostly ${g[0]}.` : `Mostly ${g[0]}, then ${andList(g.slice(1, 4))}.`;
  return { lead: `“${a.title}” was the album you kept going back to.`, sub: `By ${a.artist}, played ${a.plays.toLocaleString()} times.`, genres };
}

const TIME_WORDS: Record<Personality["time"], { who: string; when: string; short: string }> = {
  morning: { who: "a morning listener", when: "between 5 AM and noon", short: "Morning listener" },
  afternoon: { who: "an afternoon listener", when: "between noon and 5 PM", short: "Afternoon listener" },
  evening: { who: "an evening listener", when: "between 5 and 10 PM", short: "Evening listener" },
  night: { who: "a night listener", when: "between 10 PM and 5 AM", short: "Night listener" },
};

export interface Trait {
  value: string;
  label: string;
  line: string;
}

/** The hour with the most listening (the earlier when tied), or null. */
export function busiestHour(hours: number[]): number | null {
  let best: number | null = null;
  hours.forEach((ms, h) => {
    if (ms > 0 && (best == null || ms > hours[best])) best = h;
  });
  return best;
}

/** The personality card: a lead (the part of the day) and three traits: the busiest hour, repeats or deep cuts, new or old. */
export function personalityWords(p: Personality, hours: number[] = []): { lead: string; name: [string, string]; traits: Trait[] } {
  const t = TIME_WORDS[p.time];
  const peak = busiestHour(hours);
  const traits: Trait[] = [
    peak != null
      ? { value: hourLabel(peak), label: "Busiest hour", line: "You listened more in this hour of the day than in any other." }
      : { value: `${pct(p.timeShare)}%`, label: t.short, line: `of your minutes came ${t.when}.` },
    p.repeats
      ? { value: `${pct(p.repeatShare)}%`, label: "On repeat", line: "of your plays went to songs you played 5 times or more." }
      : { value: `${pct(1 - p.repeatShare)}%`, label: "Deep cuts", line: "of your plays went to songs you played fewer than 5 times." },
    p.mostlyNew
      ? { value: `${pct(p.newShare)}%`, label: "New to you", line: "of what you played, you heard for the first time this year." }
      : { value: `${pct(1 - p.newShare)}%`, label: "Old favourites", line: "of what you played, you already knew before this year." },
  ];
  const name: [string, string] = [t.short, p.repeats ? "On repeat" : "Into deep cuts"];
  return { lead: `You're ${t.who}: ${pct(p.timeShare)}% of your minutes came ${t.when}.`, name, traits };
}

export function dayWords(w: Wrapped): { lead: string; sub: string; replay: string | null } | null {
  const d = w.biggestDay;
  if (!d) return null;
  const r = w.replayed;
  return {
    lead: `${dayName(d.key)} was your biggest day: ${hoursAndMinutes(d.ms)}.`,
    sub: `${d.plays.toLocaleString()} ${d.plays === 1 ? "song" : "songs"} in one day.`,
    replay: r ? `On ${dayName(r.day)} you played “${r.song.title}” ${r.song.plays} times.` : null,
  };
}

// ---------------------------------------------------------------- summary card

export interface SummaryContent {
  year: number;
  minutes: number;
  change: string | null;
  artists: string[];
  songs: { title: string; artist: string }[];
  genre: string | null;
  /** Two short lines: the part of the day, repeats or deep cuts. */
  personality: [string, string] | null;
  tone: YearTone;
  /** The top artist's picture for the corner (an art path). */
  artistArt: string | null;
  /** Unreleased songs went in (the person turned that on). */
  withUnreleased: boolean;
}

/** What the summary card may show: unreleased songs only when `includeUnreleased`. */
export function summaryContent(w: Wrapped, includeUnreleased = false): SummaryContent {
  const p = change(w.totals.ms, w.prevMs);
  return {
    year: w.year,
    minutes: minutes(w.totals.ms),
    change: p ? `${p} on ${w.year - 1}` : null,
    artists: w.topArtists.slice(0, 5).map((a) => a.name),
    songs: w.topSongs
      .filter((s) => includeUnreleased || !s.unreleased)
      .slice(0, 5)
      .map((s) => ({ title: s.title, artist: s.artist })),
    genre: w.topGenres[0]?.name ?? null,
    personality: w.personality ? personalityWords(w.personality).name : null,
    tone: yearTone(w.year),
    artistArt: w.topArtists[0]?.thumb ?? null,
    withUnreleased: includeUnreleased,
  };
}

export interface SummaryLayout {
  width: number;
  height: number;
  /** The year band at the top: the pattern and the numerals over it. */
  band: { x: number; y: number; w: number; h: number };
  /** The record; its label carries the top artist's picture when there is one. */
  pattern: YearPattern;
  numerals: { text: string; x: number; y: number; size: number };
  texts: CardText[];
  /** Thin rules between sections. */
  rules: { x: number; y: number; w: number }[];
  mark: CardLayout["mark"];
}

const PAD = 88;

const tx = (lines: string[], x: number, y: number, size: number, weight: CardText["weight"], color: CardText["color"], lineHeight = 1.2): CardText => ({
  lines,
  x,
  y,
  size,
  weight,
  color,
  align: "left",
  lineHeight,
});

/** Where everything goes on the 1080 by 1920 summary. `y` of a text is its first baseline. */
export function layoutSummaryCard(c: SummaryContent, measure: Measure): SummaryLayout {
  const { width, height } = CARD_SIZES.story;
  const inner = width - PAD * 2;
  const band = { x: 0, y: 0, w: width, h: 640 };
  const pattern = yearPattern(c.year, band.w, band.h);
  const texts: CardText[] = [];
  const rules: SummaryLayout["rules"] = [];
  texts.push(tx(["WRAPPED"], PAD, 132, 34, 700, "primary"));
  const numerals = { text: String(c.year), x: PAD - 12, y: band.h - 44, size: 260 };

  let y = band.h + 140;
  texts.push(tx([c.minutes.toLocaleString()], PAD, y, 120, 800, "primary", 1));
  y += 56;
  texts.push(tx([c.change ? `minutes listened · ${c.change}` : "minutes listened"], PAD, y, 32, 400, "secondary"));
  y += 60;
  rules.push({ x: PAD, y, w: inner });
  y += 80;

  const colW = (inner - 56) / 2;
  const list = (title: string, x: number, entries: { line: string; sub?: string }[]) => {
    let yy = y;
    texts.push(tx([title.toUpperCase()], x, yy, 26, 700, "muted"));
    yy += 58;
    entries.slice(0, 5).forEach((e, i) => {
      const nx = x + 46;
      texts.push(tx([String(i + 1)], x, yy, 36, 800, "secondary"));
      texts.push(tx(wrapText(e.line, colW - 46, 36, 700, measure, 1), nx, yy, 36, 700, "primary"));
      yy += 46;
      if (e.sub) {
        texts.push(tx(wrapText(e.sub, colW - 46, 26, 400, measure, 1), nx, yy - 6, 26, 400, "secondary"));
        yy += 28;
      }
      yy += 16;
    });
    return yy;
  };
  const a = list("Top artists", PAD, c.artists.map((n) => ({ line: n })));
  const s = list("Top songs", PAD + colW + 56, c.songs.map((x) => ({ line: x.title, sub: x.artist })));
  y = Math.max(a, s) + 20;
  rules.push({ x: PAD, y, w: inner });
  y += 76;
  const facts: [string, string[] | null][] = [
    ["TOP GENRE", c.genre ? wrapText(c.genre, colW, 40, 700, measure, 2) : null],
    ["LISTENING", c.personality ? c.personality.map((l) => wrapText(l, colW, 40, 700, measure, 1)[0]) : null],
  ];
  facts.forEach(([head, lines], i) => {
    if (!lines) return;
    const x = PAD + i * (colW + 56);
    texts.push(tx([head], x, y, 26, 700, "muted"));
    texts.push(tx(lines, x, y + 56, 40, 700, "primary"));
  });
  const markSize = 56;
  return { width, height, band, pattern, numerals, texts, rules, mark: { x: PAD, y: height - PAD - markSize, size: markSize } };
}

/** Draw a laid-out summary. `artistImg` is the top artist's loaded picture, or null. */
export function drawSummaryCard(ctx: CanvasRenderingContext2D, l: SummaryLayout, c: SummaryContent, artistImg: CanvasImageSource | null = null) {
  ctx.fillStyle = "#0E0D0C";
  ctx.fillRect(0, 0, l.width, l.height);
  drawYearPattern(ctx, l.pattern, c.tone, l.band.x, l.band.y, l.band.w, l.band.h, artistImg);
  ctx.font = fontFor(l.numerals.size, 800);
  ctx.fillStyle = c.tone.light;
  ctx.textAlign = "left";
  ctx.textBaseline = "alphabetic";
  ctx.fillText(l.numerals.text, l.numerals.x, l.numerals.y);
  ctx.fillStyle = "rgba(255,240,220,0.14)";
  for (const r of l.rules) {
    roundRect(ctx, r.x, r.y, r.w, 2, 1);
    ctx.fill();
  }
  for (const t of l.texts) drawText(ctx, t);
  drawMark(ctx, l.mark);
}
