// Memory check for the album grid against the browser mock.
//
//   npx vite --port 1460            (in another terminal)
//   node scripts/measure-memory.mjs [--dpr 1.5] [--albums 5000] [--url http://localhost:1460]
//   (set CHROME=<path to chrome.exe> to use an already installed Chromium)
//
// Opens Your Library with N albums, scrolls the whole grid, then reports the
// JS heap, DOM node count, mounted <img> count, the decoded size of the
// mounted artwork and the private memory of the Chromium process tree
// (Windows only). Art is served as real JPEGs at the size each URL asks for
// (the mock's ?httpArt=1 mode), so image numbers track what the app requests.
import { execFileSync } from "node:child_process";
import { chromium } from "playwright-core";

const arg = (name, def) => {
  const i = process.argv.indexOf(`--${name}`);
  return i > 0 ? process.argv[i + 1] : def;
};
const base = arg("url", "http://localhost:1460");
const dpr = Number(arg("dpr", "1"));
const albums = Number(arg("albums", "5000"));

// A marker switch so the browser process can be found for the memory total.
const marker = `--platter-measure=${process.pid}`;
const browser = await chromium.launch({
  headless: true,
  // --args "--flag-a --flag-b" tries browser switches (e.g. WebView2 candidates).
  args: [marker, ...arg("args", "").split(" ").filter(Boolean)],
  ...(process.env.CHROME ? { executablePath: process.env.CHROME } : { channel: "chromium" }),
});
const ctx = await browser.newContext({ viewport: { width: 1440, height: 900 }, deviceScaleFactor: dpr });

// One JPEG per requested size, made once in a scratch page.
const jpegs = new Map();
const maker = await ctx.newPage();
await maker.setContent("<canvas></canvas>");
async function jpeg(w, h) {
  const key = `${w}x${h}`;
  if (!jpegs.has(key)) {
    const b64 = await maker.evaluate(
      ([w, h]) => {
        const c = document.querySelector("canvas");
        c.width = w;
        c.height = h;
        const g = c.getContext("2d");
        const grad = g.createLinearGradient(0, 0, w, h);
        grad.addColorStop(0, "#a0522d");
        grad.addColorStop(1, "#203040");
        g.fillStyle = grad;
        g.fillRect(0, 0, w, h);
        // A little texture so the JPEG is a realistic size.
        for (let i = 0; i < (w * h) / 40; i++) {
          g.fillStyle = `hsla(${(i * 37) % 360} 60% 60% / .25)`;
          g.fillRect((i * 7919) % w, (i * 104729) % h, 3, 3);
        }
        return c.toDataURL("image/jpeg", 0.8).split(",")[1];
      },
      [w, h],
    );
    jpegs.set(key, Buffer.from(b64, "base64"));
  }
  return jpegs.get(key);
}

const page = await ctx.newPage();
let artRequests = 0;
let artBytes = 0;
await page.route("http://art.localhost/**", async (route) => {
  const u = new URL(route.request().url());
  const w = Number(u.searchParams.get("w") || 300);
  const h = Number(u.searchParams.get("h") || w);
  const body = await jpeg(w, h);
  artRequests++;
  artBytes += body.length;
  await route.fulfill({ status: 200, contentType: "image/jpeg", body, headers: { "cache-control": "max-age=31536000, immutable" } });
});

const cdp = await ctx.newCDPSession(page);
await cdp.send("Performance.enable");

async function snapshot(label) {
  await cdp.send("HeapProfiler.collectGarbage");
  await page.waitForTimeout(300);
  const { metrics } = await cdp.send("Performance.getMetrics");
  const m = Object.fromEntries(metrics.map((x) => [x.name, x.value]));
  const imgs = await page.evaluate(() => {
    const list = [...document.querySelectorAll("img")];
    let decoded = 0;
    const sizes = {};
    for (const img of list) {
      if (!img.complete || !img.currentSrc) continue;
      const u = new URL(img.currentSrc, location.href);
      const w = Number(u.searchParams.get("w")) || img.naturalWidth;
      const h = Number(u.searchParams.get("h")) || img.naturalHeight;
      decoded += w * h * 4;
      sizes[w] = (sizes[w] || 0) + 1;
    }
    const shown = list[0]?.getBoundingClientRect().width ?? 0;
    return { count: list.length, decoded, sizes, firstCssWidth: Math.round(shown) };
  });
  let privateMB = null;
  if (process.platform === "win32") {
    const ps = `$all = Get-CimInstance Win32_Process | Select-Object ProcessId,ParentProcessId,PrivatePageCount,CommandLine;
      $ids = New-Object System.Collections.Generic.HashSet[int];
      foreach ($p in $all) { if ($p.CommandLine -like '*${marker}*') { [void]$ids.Add([int]$p.ProcessId) } }
      do { $n = $ids.Count; foreach ($p in $all) { if ($ids.Contains([int]$p.ParentProcessId)) { [void]$ids.Add([int]$p.ProcessId) } } } while ($ids.Count -ne $n);
      $all | Where-Object { $ids.Contains([int]$_.ProcessId) } | ForEach-Object {
        $t = if ($_.CommandLine -match '--type=([a-z-]+)') { $Matches[1] } else { 'browser' };
        if ($_.CommandLine -match 'utility-sub-type=([a-zA-Z.]+)') { $t = $Matches[1] };
        '{0}={1}' -f $t, $_.PrivatePageCount }`;
    const out = execFileSync("powershell.exe", ["-NoProfile", "-Command", ps], { encoding: "utf8" });
    privateMB = {};
    for (const line of out.trim().split(/\s+/)) {
      const [t, v] = line.split("=");
      privateMB[t] = (privateMB[t] || 0) + Math.round(Number(v) / 1048576);
    }
    privateMB.total = Object.values(privateMB).reduce((a, b) => a + b, 0);
  }
  const row = {
    label,
    jsHeapMB: +(m.JSHeapUsedSize / 1048576).toFixed(1),
    nodes: m.Nodes,
    imgs: imgs.count,
    decodedImgMB: +(imgs.decoded / 1048576).toFixed(1),
    imgSizes: imgs.sizes,
    artRequests,
    artKB: Math.round(artBytes / 1024),
    browserTreePrivateMB: privateMB,
  };
  console.log(JSON.stringify(row));
  return row;
}

await page.goto(`${base}/?albums=${albums}&latency=0&httpArt=1`);
await page.getByRole("button", { name: "Your Library" }).first().click();
await page.getByRole("list", { name: "Albums" }).waitFor();
await page.waitForTimeout(1500);
await snapshot(`library loaded (dpr ${dpr})`);

// Scroll the whole grid in viewport-sized steps.
for (;;) {
  const done = await page.evaluate(() => {
    const el = document.querySelector(".main__scroll");
    el.scrollTop += el.clientHeight * 0.9;
    return el.scrollTop + el.clientHeight >= el.scrollHeight - 2;
  });
  await page.waitForTimeout(60);
  if (done) break;
}
await page.waitForTimeout(1500);
await snapshot(`after scrolling ${albums} albums`);

// Back to the top (what you see when you return to the page).
await page.evaluate(() => (document.querySelector(".main__scroll").scrollTop = 0));
await page.waitForTimeout(1500);
await snapshot("back at top");

await browser.close();
