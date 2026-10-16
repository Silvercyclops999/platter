/**
 * The public docs site: the same Markdown as the in-app help, as plain HTML
 * pages. Pure, so a test can build it and check every link; the workflow
 * runs scripts/build-help-site.ts, which writes the files.
 */
import { helpLink, parse, plainText, toHtml, type Block } from "./markdown.ts";
import { HELP_ORDER } from "./order.ts";

export interface SourcePage {
  slug: string;
  md: string;
}

export interface HelpDoc {
  slug: string;
  title: string;
  summary: string;
  blocks: Block[];
}

export function readPage(p: SourcePage): HelpDoc {
  const blocks = parse(p.md);
  const h1 = blocks.find((b) => b.t === "h" && b.level === 1);
  const first = blocks.find((b) => b.t === "p");
  return {
    slug: p.slug,
    title: h1 && h1.t === "h" ? h1.text : p.slug,
    summary: first && first.t === "p" ? plainText(first.c) : "",
    // The title shows in the page frame, not twice.
    blocks: blocks.filter((b) => b !== h1),
  };
}

/** Pages in reading order, then any not listed yet. */
export function ordered(pages: SourcePage[]): HelpDoc[] {
  const rank = (s: string) => {
    const i = (HELP_ORDER as readonly string[]).indexOf(s);
    return i < 0 ? HELP_ORDER.length : i;
  };
  return pages
    .slice()
    .sort((a, b) => rank(a.slug) - rank(b.slug) || a.slug.localeCompare(b.slug))
    .map(readPage);
}

const REPO = "https://github.com/Silvercyclops999/platter";

const CSS = `:root{color-scheme:light dark;--bg:#fbfaf7;--fg:#1d1c1a;--muted:#5d5a55;--line:#e4e0d8;--link:#2f5fb3}
@media (prefers-color-scheme:dark){:root{--bg:#141414;--fg:#ecebe8;--muted:#a9a6a0;--line:#2c2b29;--link:#8fb3ff}}
*{box-sizing:border-box}body{margin:0;background:var(--bg);color:var(--fg);font:16px/1.6 system-ui,-apple-system,"Segoe UI",sans-serif}
.wrap{display:flex;gap:40px;max-width:1080px;margin:0 auto;padding:32px 16px}nav{flex:0 0 220px}main{flex:1;min-width:0;max-width:720px}
nav ul{list-style:none;padding:0;margin:0}nav li{margin:4px 0}nav a[aria-current]{font-weight:600;color:var(--fg)}
a{color:var(--link)}h1{font-size:2rem;line-height:1.2;margin:0 0 8px}h2{margin-top:2em;border-top:1px solid var(--line);padding-top:1em}
.lede{color:var(--muted);font-size:1.1rem}img{max-width:100%;height:auto;border-radius:8px;border:1px solid var(--line)}
code{font-size:.9em;background:var(--line);padding:1px 5px;border-radius:4px}pre{overflow:auto;background:var(--line);padding:12px;border-radius:8px}pre code{background:none;padding:0}
footer{margin-top:48px;color:var(--muted);font-size:.9rem}
@media (max-width:760px){.wrap{flex-direction:column;gap:16px}nav{flex:none}}`;

const esc = (s: string) => s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");

/** Help links go to the page's .html; anything else stays as written. */
export function siteLink(href: string): string {
  const h = helpLink(href);
  if (h) return `${h.slug}.html${h.anchor ? `#${h.anchor}` : ""}`;
  return href;
}

function frame(title: string, nav: string, body: string): string {
  return `<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>${esc(title)}</title>
<link rel="stylesheet" href="style.css">
</head>
<body>
<div class="wrap">
<nav aria-label="Help pages">${nav}</nav>
<main>
${body}
<footer>Platter help. The same pages are built into the app. <a href="${REPO}">Platter on GitHub</a></footer>
</main>
</div>
</body>
</html>
`;
}

/** Every file of the site, by path. Images are copied by the caller from docs/help/images. */
export function buildSite(pages: SourcePage[]): Map<string, string> {
  const docs = ordered(pages);
  const out = new Map<string, string>();
  const nav = (current: string | null) =>
    `<ul><li><a href="index.html"${current === null ? ' aria-current="page"' : ""}>Platter help</a></li>${docs
      .map((d) => `<li><a href="${d.slug}.html"${d.slug === current ? ' aria-current="page"' : ""}>${esc(d.title)}</a></li>`)
      .join("")}</ul>`;
  const to = { link: siteLink, image: (src: string) => src };
  for (const d of docs) {
    const body = `<h1>${esc(d.title)}</h1>\n${toHtml(d.blocks, to)}`;
    out.set(`${d.slug}.html`, frame(`${d.title}: Platter help`, nav(d.slug), body));
  }
  const index = `<h1>Platter help</h1>
<p class="lede">How to use Platter, the desktop music player for your Plex library.</p>
<ul>${docs.map((d) => `<li><a href="${d.slug}.html">${esc(d.title)}</a>: ${esc(d.summary)}</li>`).join("")}</ul>`;
  out.set("index.html", frame("Platter help", nav(null), index));
  out.set("style.css", CSS);
  return out;
}

/**
 * Links in the built site that lead nowhere: a missing page, a missing
 * anchor, or an image that isn't there. External links aren't fetched.
 */
export function brokenLinks(files: Map<string, string>, images: Set<string>): string[] {
  const ids = new Map<string, Set<string>>();
  for (const [path, html] of files) {
    if (path.endsWith(".html")) ids.set(path, new Set([...html.matchAll(/ id="([^"]+)"/g)].map((m) => m[1])));
  }
  const broken: string[] = [];
  for (const [path, html] of files) {
    if (!path.endsWith(".html")) continue;
    for (const [, href] of html.matchAll(/ href="([^"]+)"/g)) {
      if (/^https:\/\//.test(href)) continue;
      const [file, anchor] = href.split("#");
      const target = file === "" ? path : file;
      if (target === "style.css" && files.has(target)) continue;
      const known = ids.get(target);
      if (!known || (anchor && !known.has(anchor))) broken.push(`${path}: ${href}`);
    }
    for (const [, src] of html.matchAll(/ src="([^"]+)"/g)) {
      if (!images.has(src)) broken.push(`${path}: image ${src}`);
    }
  }
  return broken;
}
