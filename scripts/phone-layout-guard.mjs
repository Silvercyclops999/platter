// Phone layout guard: opens every phone page in the browser mock at phone
// widths, in Platter's own look and in Classic, and fails on layout faults.
//
//   node scripts/phone-layout-guard.mjs [--widths 390,430,440] [--looks studio,dark] [--routes album,artist] [--shots <folder>] [--headed]
//   (needs Chromium once: npx playwright-core install --only-shell chromium; set CHROME=<path> to use
//   another, GUARD_VERBOSE=1 to log each page and what it shows)
//
// It checks, on each page:
//   text running out of its box sideways without an ellipsis, and pages that scroll sideways;
//   icons (SVGs up to 48 px) cut off by a box that hides its overflow;
//   single-line text cut off at the top or bottom (descenders) by a box that hides its overflow;
//   tap targets (buttons, links, role=button, inputs) smaller than 44 x 44 CSS px.
// An inline text link that is meant to be small can opt out with data-tap-inline.
//
// Now playing is checked too, open over Home with a queue behind it.
//
// The pages come from src/phone/routes.tsx, so a new route is covered as soon
// as it has a sample in SAMPLES below (the guard fails until it has one).
import { readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { chromium } from "playwright-core";
import { createServer } from "vite";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const arg = (name, def) => {
  const i = process.argv.indexOf(`--${name}`);
  return i > 0 ? process.argv[i + 1] : def;
};
const widths = arg("widths", "390,430,440").split(",").map(Number);
// "studio" is Platter's own look (the default), "dark" is Classic.
const looks = arg("looks", "studio,dark").split(",");
const LOOK_NAME = { studio: "Platter", dark: "Classic" };
const only = arg("routes", "")
  .split(",")
  .filter(Boolean);
const headed = process.argv.includes("--headed");
// --shots <folder> also saves a screenshot of each page.
const shots = arg("shots", "");

// Every route name the phone has a page for.
const table = readFileSync(path.join(root, "src/phone/routes.tsx"), "utf8");
const block = table.slice(table.indexOf("export const PHONE_PAGES"));
const names = [...block.slice(0, block.indexOf("\n};")).matchAll(/^ {2}(\w+):/gm)].map((m) => m[1]);
if (names.length < 10) throw new Error("couldn't read the phone routes from src/phone/routes.tsx");

// A sample route for each name, built in the page from the mock's own data.
// `m` is the mock backend. Return one route, or several to cover each.
const SAMPLES = `async (m) => {
  const fx = m.fx;
  const artist = fx.artists[0].id;
  const album = fx.albums[0].id;
  const track = fx.tracks[0].id;
  const playlist = fx.playlists[0].id;
  const crate = (await m.crates("label")).crates[0];
  const catArtist = "c0ffee00-0000-4000-8000-000000000001";
  // The catalogue answers once the app has settled its connection.
  let cat = null;
  for (let i = 0; i < 40 && !cat; i++) {
    cat = await m.catalogueArtist(catArtist).catch(() => null);
    if (!cat) await new Promise((r) => setTimeout(r, 100));
  }
  const catAlbum = cat?.sections?.find((s) => s.albums.length)?.albums[0]?.releaseGroupId ?? cat?.popular?.[0]?.releaseGroupId;
  const credits = await m.albumCredits(album).catch(() => null);
  const person = credits?.credits?.[0]?.name ?? "Nell Ardent";
  return {
    home: { name: "home" },
    search: { name: "search" },
    library: { name: "library" },
    artist: { name: "artist", id: artist },
    discography: { name: "discography", id: artist },
    lineage: { name: "lineage", id: track },
    era: { name: "era", id: "190", era: "Glass Season" },
    album: { name: "album", id: album },
    playlist: { name: "playlist", id: playlist },
    liked: { name: "liked" },
    settings: { name: "settings" },
    recents: { name: "recents" },
    whatsNew: { name: "whatsNew" },
    releaseRadar: { name: "releaseRadar" },
    lyrics: { name: "lyrics" },
    homeShelf: { name: "homeShelf", shelf: "recentlyAdded" },
    artistShelf: { name: "artistShelf", id: artist, shelf: "fansAlsoLike" },
    mix: { name: "mix", id: "daylist" },
    radio: { name: "radio", seed: "artist:" + artist },
    bookmarks: { name: "bookmarks" },
    stats: [{ name: "stats" }, { name: "stats", tab: "recaps" }, { name: "stats", tab: "compare" }],
    profile: { name: "profile" },
    topSongs: { name: "topSongs" },
    import: { name: "import" },
    doctor: { name: "doctor" },
    devices: { name: "devices" },
    soundMap: { name: "soundMap" },
    crates: { name: "crates" },
    crate: crate ? { name: "crate", kind: "label", key: crate.key } : { name: "crates" },
    catalogueArtist: { name: "catalogueArtist", mbid: catArtist },
    catalogueAlbum: catAlbum ? { name: "catalogueAlbum", id: catAlbum } : { name: "catalogueArtist", mbid: catArtist },
    newToYou: { name: "newToYou" },
    onThisDay: { name: "onThisDay" },
    shelf: { name: "shelf" },
    diary: { name: "diary" },
    blendNew: { name: "blendNew" },
    credits: { name: "credits", person },
    mixes: { name: "mixes" },
    archive: { name: "archive" },
    downloads: { name: "downloads" },
    help: [{ name: "help" }, { name: "help", slug: "getting-started" }],
  };
}`;

// Runs in the page: every fault on what's on screen now.
function audit() {
  const faults = [];
  const vw = window.innerWidth;
  const shell = document.querySelector("[data-testid=phone-shell]") ?? document.body;
  const label = (el) => {
    const parts = [];
    for (let e = el; e && e !== shell && parts.length < 3; e = e.parentElement) {
      const cls = [...e.classList].filter((c) => !c.startsWith("is-")).slice(0, 2);
      parts.unshift(e.tagName.toLowerCase() + (cls.length ? "." + cls.join(".") : ""));
    }
    return parts.join(" > ");
  };
  const text = (el) => (el.getAttribute?.("aria-label") || el.textContent || "").replace(/\s+/g, " ").trim().slice(0, 40);
  const add = (kind, el, detail) => faults.push({ kind, sel: label(el), text: text(el), detail });
  const hidden = (el) => !!el.closest("[inert], [aria-hidden=true]") || el.getClientRects().length === 0;
  const style = (el) => getComputedStyle(el);
  const scrolls = (v) => v === "auto" || v === "scroll";
  const clips = (v) => v === "hidden" || v === "clip";
  const isScroller = (el) => {
    const s = style(el);
    return scrolls(s.overflowX) || scrolls(s.overflowY);
  };
  const box = (el) => {
    for (let e = el; e; e = e.parentElement) {
      const d = style(e).display;
      if (d !== "inline" && d !== "contents") return e;
    }
    return el;
  };
  // The padding box, where content is cut off.
  const inner = (el) => {
    const r = el.getBoundingClientRect();
    const left = r.left + el.clientLeft;
    const top = r.top + el.clientTop;
    return { left, top, right: left + el.clientWidth, bottom: top + el.clientHeight };
  };

  // Sideways scrolling: the page, and any up-and-down scroller that also goes sideways.
  const doc = document.scrollingElement;
  if (doc.scrollWidth > vw + 1) add("page scrolls sideways", document.body, `${doc.scrollWidth}px wide`);
  for (const el of shell.querySelectorAll("*")) {
    const s = style(el);
    if (scrolls(s.overflowY) && scrolls(s.overflowX) && el.scrollWidth > el.clientWidth + 1 && el.scrollHeight > el.clientHeight + 1 && !hidden(el)) {
      add("page scrolls sideways", el, `${el.scrollWidth}px in ${el.clientWidth}px`);
    }
  }

  // Text: each run of text, against the box that holds it.
  const walker = document.createTreeWalker(shell, NodeFilter.SHOW_TEXT, { acceptNode: (n) => (n.data.trim() ? NodeFilter.FILTER_ACCEPT : NodeFilter.FILTER_REJECT) });
  const range = document.createRange();
  const seenWide = new Set();
  for (let n = walker.nextNode(); n; n = walker.nextNode()) {
    const host = n.parentElement;
    if (!host || hidden(host) || host.closest("svg, script, style, [data-layout-guard-skip]")) continue;
    if (style(host).visibility === "hidden") continue;
    range.selectNodeContents(n);
    const rects = [...range.getClientRects()].filter((r) => r.width > 0);
    if (!rects.length) continue;
    const b = box(host);
    const bs = style(b);
    // Text only screen readers get (a 1 px clipped box) is meant to be cut.
    const br = b.getBoundingClientRect();
    if (br.width <= 1 || br.height <= 1) continue;
    // Sideways: text past its box with no ellipsis.
    if (!isScroller(b) && bs.textOverflow !== "ellipsis" && !seenWide.has(b)) {
      const p = inner(b);
      const padR = parseFloat(bs.paddingRight) || 0;
      const padL = parseFloat(bs.paddingLeft) || 0;
      const right = Math.max(...rects.map((r) => r.right));
      const left = Math.min(...rects.map((r) => r.left));
      if (right > p.right - padR + 1.5 || left < p.left + padL - 1.5 || (b.scrollWidth > b.clientWidth + 1 && right > p.right + 0.5)) {
        seenWide.add(b);
        add("text runs out sideways", b, `${Math.round(right - left)}px of text in ${Math.round(p.right - p.left - padL - padR)}px`);
      }
    }
    // Up and down: single-line text cut by a box that hides its overflow.
    const lines = new Set(rects.map((r) => Math.round(r.top)));
    if (lines.size === 1) {
      const top = Math.min(...rects.map((r) => r.top));
      const bottom = Math.max(...rects.map((r) => r.bottom));
      for (let e = b, depth = 0; e && e !== shell && depth < 4; e = e.parentElement, depth++) {
        const s = style(e);
        if (isScroller(e)) break;
        if (!clips(s.overflowY) && !clips(s.overflow)) continue;
        if (s.webkitLineClamp && s.webkitLineClamp !== "none") break;
        const p = inner(e);
        const cut = Math.max(bottom - p.bottom, p.top - top);
        if (cut > 1) {
          add("text cut off top or bottom", e, `${cut.toFixed(1)}px cut (font ${s.fontSize}, line ${s.lineHeight})`);
          break;
        }
      }
    }
  }

  // Icons cut off by a box that hides its overflow.
  for (const svg of shell.querySelectorAll("svg")) {
    if (hidden(svg)) continue;
    const r = svg.getBoundingClientRect();
    if (r.width === 0 || r.width > 48 || r.height > 48) continue;
    for (let e = svg.parentElement, depth = 0; e && e !== shell && depth < 6; e = e.parentElement, depth++) {
      const s = style(e);
      if (isScroller(e)) break;
      if (!clips(s.overflowX) && !clips(s.overflowY)) continue;
      const p = inner(e);
      const cut = Math.max(p.left - r.left, r.right - p.right, p.top - r.top, r.bottom - p.bottom);
      if (cut > 1) {
        add("icon cut off", svg, `${cut.toFixed(1)}px outside ${label(e)}`);
        break;
      }
    }
  }

  // Tap targets.
  const targets = shell.querySelectorAll("button, a[href], [role=button], input:not([type=hidden]), select, textarea");
  for (const el of targets) {
    if (hidden(el) || el.closest("[data-tap-inline]")) continue;
    const s = style(el);
    if (s.visibility === "hidden" || s.pointerEvents === "none") continue;
    let target = el;
    // A visually hidden input is tapped through its label.
    if (el.tagName === "INPUT" && (parseFloat(s.opacity) === 0 || el.getBoundingClientRect().width <= 1)) {
      target = el.closest("label") ?? (el.id ? document.querySelector(`label[for="${CSS.escape(el.id)}"]`) : null) ?? el;
    }
    // Inside a bigger target (a whole row that is a label or button), the bigger one is what's tapped.
    const outer = target.parentElement?.closest("label, button, a[href], [role=button]");
    if (outer && outer !== target && !hidden(outer)) {
      const o = outer.getBoundingClientRect();
      if (o.width >= 44 && o.height >= 44 && target.tagName === "INPUT") continue;
    }
    const r = target.getBoundingClientRect();
    let { left, top, right, bottom } = r;
    // A ::before or ::after placed around the target widens what a finger hits,
    // as far as no box around it cuts it off.
    let widened = false;
    for (const pseudo of ["::before", "::after"]) {
      const ps = getComputedStyle(target, pseudo);
      if (ps.content === "none" || ps.position !== "absolute" || ps.pointerEvents === "none") continue;
      const pl = parseFloat(ps.left);
      const pt = parseFloat(ps.top);
      const pw = parseFloat(ps.width);
      const ph = parseFloat(ps.height);
      if ([pl, pt, pw, ph].some(Number.isNaN)) continue;
      const ox = r.left + target.clientLeft + pl;
      const oy = r.top + target.clientTop + pt;
      left = Math.min(left, ox);
      top = Math.min(top, oy);
      right = Math.max(right, ox + pw);
      bottom = Math.max(bottom, oy + ph);
      widened = true;
    }
    let cutBy = "";
    if (widened) {
      for (let e = target.parentElement; e && e !== shell; e = e.parentElement) {
        const s = style(e);
        if (s.overflowX === "visible" && s.overflowY === "visible") continue;
        const p = inner(e);
        // A scroller cuts off only what's before its start; past its end it scrolls further.
        if (isScroller(e)) {
          p.left -= e.scrollLeft;
          p.top -= e.scrollTop;
          p.right = p.bottom = Infinity;
        }
        const was = (right - left) * (bottom - top);
        left = Math.max(left, Math.min(r.left, p.left));
        right = Math.min(right, Math.max(r.right, p.right));
        top = Math.max(top, Math.min(r.top, p.top));
        bottom = Math.min(bottom, Math.max(r.bottom, p.bottom));
        if (!cutBy && (right - left) * (bottom - top) < was - 0.5) cutBy = `, its frame cut by ${label(e)}`;
        // Boxes outside the scroller cut its window, which the content scrolls through.
        if (isScroller(e)) break;
      }
    }
    const w = right - left;
    const h = bottom - top;
    if (w < 43.5 || h < 43.5) add("tap target under 44px", target, `${Math.round(w)}x${Math.round(h)}${cutBy}`);
  }
  return faults;
}

async function settle(page) {
  await page.evaluate(async () => {
    const deadline = performance.now() + 4000;
    const frame = () => new Promise((r) => requestAnimationFrame(() => r()));
    while (performance.now() < deadline) {
      const main = document.querySelector(".ph-main");
      const busy = main?.querySelector(".ph-loading, [aria-busy=true]");
      const moving = document.getAnimations().some((a) => a.playState === "running" && a.effect?.getTiming().iterations !== Infinity);
      if (!busy && !moving) break;
      await new Promise((r) => setTimeout(r, 50));
    }
    await document.fonts.ready;
    await frame();
    await frame();
  });
}

const server = await createServer({
  root,
  logLevel: "error",
  server: { port: 1470, strictPort: false, watch: null, hmr: false },
  // Find every package up front, so a lazy page doesn't make Vite reload mid-run.
  optimizeDeps: { entries: ["index.html", "src/**/*.tsx", "!src/**/*.test.tsx", "!src/test/**"] },
});
await server.listen();
const base = server.resolvedUrls.local[0].replace(/\/$/, "");

const browser = await chromium.launch({
  headless: !headed,
  ...(process.env.CHROME ? { executablePath: process.env.CHROME } : {}),
});

const failures = [];
const started = Date.now();
try {
  for (const look of looks) {
    for (const width of widths) {
      const ctx = await browser.newContext({ viewport: { width, height: 844 }, deviceScaleFactor: 1, reducedMotion: "reduce", hasTouch: true });
      const page = await ctx.newPage();
      page.on("pageerror", (e) => failures.push({ route: "(page)", width, look, kind: "script error", sel: "", text: String(e.message).slice(0, 120), detail: "" }));
      if (process.env.GUARD_VERBOSE) {
        page.on("console", (m) => console.error("console:", m.type(), m.text().slice(0, 200)));
        page.on("framenavigated", (f) => f === page.mainFrame() && console.error("navigated:", f.url()));
        page.on("crash", () => console.error("crashed"));
      }
      await page.goto(`${base}/?phone=1&latency=0&theme=${look}`);
      await page.locator("[data-testid=phone-shell]").waitFor({ timeout: 60000 });
      const samples = await page.evaluate(`(${SAMPLES})(window.__platterMock)`);
      // The app's own router and phone state (the same modules the page runs).
      await page.evaluate(async () => {
        const { useRouter } = await import("/src/stores/router.ts");
        const { usePhoneUi } = await import("/src/phone/phoneUi.ts");
        window.__guardGo = (route) => {
          usePhoneUi.setState({ nowPlaying: false, lyrics: false, sheets: [] });
          useRouter.getState().reset(route);
          window.scrollTo(0, 0);
        };
      });
      for (const name of names) {
        if (only.length && !only.includes(name)) continue;
        if (!samples[name]) {
          failures.push({ route: name, width, look, kind: "no sample route", sel: "scripts/phone-layout-guard.mjs", text: "add one to SAMPLES", detail: "" });
          continue;
        }
        for (const route of [samples[name]].flat()) {
          if (process.env.GUARD_VERBOSE) console.error(look, width, JSON.stringify(route));
          // Changed after the call returns, so nothing the page does next holds up the call.
          await page.evaluate((route) => {
            setTimeout(() => window.__guardGo(route), 0);
          }, route);
          await page.waitForTimeout(100);
          await settle(page);
          if (process.env.GUARD_VERBOSE) {
            const shown = await page.evaluate(() => (document.querySelector(".ph-main")?.innerText ?? "").replace(/\s+/g, " ").slice(0, 90));
            console.error("  shows:", shown);
          }
          const where = JSON.stringify(route).replace(/"name":/, "").replace(/"/g, "");
          if (shots) await page.screenshot({ path: path.join(shots, `${look}-${width}-${where.replace(/[^\w-]+/g, "_")}.png`) });
          for (const f of await page.evaluate(audit)) failures.push({ route: where, width, look, ...f });
        }
      }
      // Now playing, over Home, with a song and a queue behind it.
      if (!only.length || only.includes("nowPlaying")) {
        await page.evaluate(async () => {
          const m = window.__platterMock;
          const { player } = await import("/src/stores/player.ts");
          const { usePhoneUi } = await import("/src/phone/phoneUi.ts");
          window.__guardGo({ name: "home" });
          await player.playTracks(m.fx.tracks.slice(0, 4).map((t) => t.id), m.fx.tracks[0].id);
          await new Promise((r) => setTimeout(r, 300));
          usePhoneUi.setState({ nowPlaying: true });
        });
        await page.waitForTimeout(300);
        await settle(page);
        if (shots) await page.screenshot({ path: path.join(shots, `${look}-${width}-nowPlaying.png`) });
        for (const f of await page.evaluate(audit)) failures.push({ route: "nowPlaying", width, look, ...f });
      }
      await ctx.close();
    }
  }
} finally {
  await browser.close();
  await server.close();
}

// The same fault on the same kind of element at several widths is one line,
// with how many of them there were.
const groups = new Map();
for (const f of failures) {
  const key = [f.look, f.route, f.kind, f.sel].join("|");
  const g = groups.get(key) ?? { ...f, widths: new Set(), count: 0 };
  g.widths.add(f.width);
  g.count++;
  groups.set(key, g);
}
for (const g of groups.values()) {
  const more = g.count > g.widths.size ? ` (${g.count} in all)` : "";
  console.log(`${g.route}  ${[...g.widths].join("/")}px  ${LOOK_NAME[g.look] ?? g.look}  ${g.kind}  ${g.sel}  "${g.text}"  ${g.detail}${more}`);
}
const secs = Math.round((Date.now() - started) / 1000);
if (groups.size) {
  console.log(`\n${groups.size} phone layout fault(s) in ${secs}s.`);
  process.exit(1);
}
console.log(`Phone layout guard: ${only.length ? names.filter((n) => only.includes(n)).length : names.length} routes${!only.length || only.includes("nowPlaying") ? " and now playing" : ""}, widths ${widths.join("/")}, looks ${looks.map((l) => LOOK_NAME[l] ?? l).join("/")}: no faults (${secs}s).`);
