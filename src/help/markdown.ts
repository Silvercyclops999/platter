/**
 * The small Markdown the help pages are written in (docs/help/*.md), read
 * the same way by the app's help panel and by the docs site build
 * (scripts/build-help-site.ts), so the two never differ. It has no imports,
 * so Node can run it as it is.
 *
 * Supported: # to ### headings, paragraphs, "- " and "1. " lists (one
 * level), fenced code, images on their own line, `code`, **bold**, *italic*,
 * [links](target) and <!-- comments --> (dropped).
 */

export type Inline =
  | { t: "text"; v: string }
  | { t: "code"; v: string }
  | { t: "strong"; c: Inline[] }
  | { t: "em"; c: Inline[] }
  | { t: "link"; href: string; c: Inline[] }
  | { t: "img"; src: string; alt: string };

export type Block =
  | { t: "h"; level: 1 | 2 | 3; id: string; c: Inline[]; text: string }
  | { t: "p"; c: Inline[] }
  | { t: "ul"; items: Inline[][] }
  | { t: "ol"; items: Inline[][] }
  | { t: "code"; v: string }
  | { t: "img"; src: string; alt: string };

/** A heading's anchor: lowercase, apostrophes dropped, anything else but letters and digits becomes "-". */
export function slugify(text: string): string {
  return text
    .toLowerCase()
    .replace(/[`*'’]/g, "")
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "");
}

/** Text without <!-- comments --> (TODO notes for the writers). */
export function stripComments(md: string): string {
  return md.replace(/<!--[\s\S]*?-->/g, "");
}

export function parseInline(src: string): Inline[] {
  const out: Inline[] = [];
  let text = "";
  const flush = () => {
    if (text) out.push({ t: "text", v: text });
    text = "";
  };
  let i = 0;
  while (i < src.length) {
    const rest = src.slice(i);
    let m: RegExpMatchArray | null;
    if ((m = rest.match(/^`([^`]+)`/))) {
      flush();
      out.push({ t: "code", v: m[1] });
    } else if ((m = rest.match(/^!\[([^\]]*)\]\(([^)\s]+)\)/))) {
      flush();
      out.push({ t: "img", alt: m[1], src: m[2] });
    } else if ((m = rest.match(/^\[([^\]]+)\]\(([^)\s]+)\)/))) {
      flush();
      out.push({ t: "link", href: m[2], c: parseInline(m[1]) });
    } else if ((m = rest.match(/^\*\*([^*]+)\*\*/))) {
      flush();
      out.push({ t: "strong", c: parseInline(m[1]) });
    } else if ((m = rest.match(/^\*([^*\s][^*]*)\*/))) {
      flush();
      out.push({ t: "em", c: parseInline(m[1]) });
    } else {
      text += src[i];
      i += 1;
      continue;
    }
    i += m[0].length;
  }
  flush();
  return out;
}

export function plainText(c: Inline[]): string {
  return c
    .map((n) => (n.t === "text" || n.t === "code" ? n.v : n.t === "img" ? n.alt : plainText(n.c)))
    .join("");
}

export function parse(md: string): Block[] {
  const lines = stripComments(md).replace(/\r\n?/g, "\n").split("\n");
  const blocks: Block[] = [];
  let para: string[] = [];
  let list: { kind: "ul" | "ol"; items: string[] } | null = null;
  const endPara = () => {
    if (para.length) blocks.push({ t: "p", c: parseInline(para.join(" ")) });
    para = [];
  };
  const endList = () => {
    if (list) blocks.push({ t: list.kind, items: list.items.map(parseInline) });
    list = null;
  };
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    const trimmed = line.trim();
    let m: RegExpMatchArray | null;
    if (trimmed.startsWith("```")) {
      endPara();
      endList();
      const code: string[] = [];
      i += 1;
      while (i < lines.length && !lines[i].trim().startsWith("```")) code.push(lines[i++]);
      blocks.push({ t: "code", v: code.join("\n") });
    } else if (!trimmed) {
      endPara();
      endList();
    } else if ((m = trimmed.match(/^(#{1,3})\s+(.*)$/))) {
      endPara();
      endList();
      const c = parseInline(m[2]);
      const text = plainText(c);
      blocks.push({ t: "h", level: m[1].length as 1 | 2 | 3, id: slugify(text), c, text });
    } else if ((m = trimmed.match(/^!\[([^\]]*)\]\(([^)\s]+)\)$/))) {
      endPara();
      endList();
      blocks.push({ t: "img", alt: m[1], src: m[2] });
    } else if ((m = trimmed.match(/^[-*]\s+(.*)$/)) || (m = trimmed.match(/^\d+[.)]\s+(.*)$/))) {
      endPara();
      const kind = /^\d/.test(trimmed) ? "ol" : "ul";
      if (list && list.kind !== kind) endList();
      if (!list) list = { kind, items: [] };
      list.items.push(m[1]);
    } else if (list && /^\s+/.test(line)) {
      // A wrapped list item.
      list.items[list.items.length - 1] += ` ${trimmed}`;
    } else {
      endList();
      para.push(trimmed);
    }
  }
  endPara();
  endList();
  return blocks;
}

const esc = (s: string) => s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");

/** Where links and images point in the output. */
export interface Targets {
  link: (href: string) => string;
  image: (src: string) => string;
}

function inlineHtml(c: Inline[], to: Targets): string {
  return c
    .map((n) => {
      switch (n.t) {
        case "text":
          return esc(n.v);
        case "code":
          return `<code>${esc(n.v)}</code>`;
        case "strong":
          return `<strong>${inlineHtml(n.c, to)}</strong>`;
        case "em":
          return `<em>${inlineHtml(n.c, to)}</em>`;
        case "img":
          return `<img src="${esc(to.image(n.src))}" alt="${esc(n.alt)}">`;
        case "link":
          return `<a href="${esc(to.link(n.href))}">${inlineHtml(n.c, to)}</a>`;
      }
    })
    .join("");
}

export function toHtml(blocks: Block[], to: Targets): string {
  return blocks
    .map((b) => {
      switch (b.t) {
        case "h":
          return `<h${b.level} id="${b.id}">${inlineHtml(b.c, to)}</h${b.level}>`;
        case "p":
          return `<p>${inlineHtml(b.c, to)}</p>`;
        case "ul":
        case "ol":
          return `<${b.t}>${b.items.map((it) => `<li>${inlineHtml(it, to)}</li>`).join("")}</${b.t}>`;
        case "code":
          return `<pre><code>${esc(b.v)}</code></pre>`;
        case "img":
          return `<p><img src="${esc(to.image(b.src))}" alt="${esc(b.alt)}"></p>`;
      }
    })
    .join("\n");
}

/** A help page link: "lidarr.md#practice-mode" -> { slug: "lidarr", anchor: "practice-mode" }. */
export function helpLink(href: string): { slug: string; anchor?: string } | null {
  const m = href.match(/^([a-z0-9-]+)\.md(?:#([a-z0-9-]+))?$/);
  if (!m) return null;
  return { slug: m[1], anchor: m[2] };
}
