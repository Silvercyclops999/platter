/**
 * The help pages, bundled with the app so they work offline. Written in
 * docs/help/*.md; the docs site is built from the same files.
 */
import { ordered, type HelpDoc } from "./site";
import { plainText, type Block } from "./markdown";

const sources = import.meta.glob("../../docs/help/*.md", { query: "?raw", import: "default", eager: true }) as Record<string, string>;
const imageUrls = import.meta.glob("../../docs/help/images/*.png", { query: "?url", import: "default", eager: true }) as Record<string, string>;

export const HELP_PAGES: HelpDoc[] = ordered(
  Object.entries(sources).map(([path, md]) => ({ slug: path.replace(/^.*\/([^/]+)\.md$/, "$1"), md })),
);

export function helpPage(slug: string): HelpDoc | undefined {
  return HELP_PAGES.find((p) => p.slug === slug);
}

/** A bundled image's address, from its path in the Markdown ("images/x.png"). */
export function helpImage(src: string): string | undefined {
  const name = src.replace(/^.*\//, "");
  const hit = Object.entries(imageUrls).find(([p]) => p.endsWith(`/images/${name}`));
  return hit?.[1];
}

/** One searchable piece of help: a page, or a section of one. */
export interface HelpHit {
  slug: string;
  anchor?: string;
  title: string;
  /** The page's title, for a section hit. */
  page?: string;
  snippet: string;
}

function blockText(b: Block): string {
  switch (b.t) {
    case "h":
      return b.text;
    case "p":
      return plainText(b.c);
    case "ul":
    case "ol":
      return b.items.map(plainText).join(" ");
    case "code":
      return b.v;
    case "img":
      return b.alt;
  }
}

interface Section {
  hit: HelpHit;
  title: string;
  text: string;
}

const sections: Section[] = HELP_PAGES.flatMap((p) => {
  const out: Section[] = [{ hit: { slug: p.slug, title: p.title, snippet: p.summary }, title: p.title.toLowerCase(), text: p.summary.toLowerCase() }];
  let cur: Section | null = null;
  for (const b of p.blocks) {
    if (b.t === "h" && b.level === 2) {
      cur = { hit: { slug: p.slug, anchor: b.id, title: b.text, page: p.title, snippet: "" }, title: b.text.toLowerCase(), text: "" };
      out.push(cur);
    } else if (cur) {
      const t = blockText(b);
      if (!cur.hit.snippet && b.t === "p") cur.hit.snippet = t;
      cur.text += ` ${t.toLowerCase()}`;
    } else {
      out[0].text += ` ${blockText(b).toLowerCase()}`;
    }
  }
  return out;
});

/**
 * Help pages and sections for a search, best first: every word must appear;
 * words in the title count most.
 */
export function searchHelp(query: string, limit = 8): HelpHit[] {
  const words = query.toLowerCase().split(/\s+/).filter(Boolean);
  if (!words.length) return [];
  return sections
    .map((s) => {
      let score = 0;
      for (const w of words) {
        if (s.title.includes(w)) score += 3;
        else if (s.text.includes(w)) score += 1;
        else return null;
      }
      // A whole page outranks one of its sections at the same score.
      return { s, score: score + (s.hit.anchor ? 0 : 0.5) };
    })
    .filter((x): x is { s: Section; score: number } => x !== null)
    .sort((a, b) => b.score - a.score)
    .slice(0, limit)
    .map((x) => x.s.hit);
}


/** One section of a page: its heading and what follows, up to the next heading as big. */
export function helpSection(slug: string, anchor?: string): Block[] {
  const page = helpPage(slug);
  if (!page) return [];
  if (!anchor) return page.blocks;
  const start = page.blocks.findIndex((b) => b.t === "h" && b.id === anchor);
  if (start < 0) return [];
  const head = page.blocks[start] as Extract<Block, { t: "h" }>;
  const end = page.blocks.findIndex((b, i) => i > start && b.t === "h" && b.level <= head.level);
  return page.blocks.slice(start, end < 0 ? undefined : end);
}
