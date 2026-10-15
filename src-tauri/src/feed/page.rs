//! The overlay page at `/now-playing.html`: a cover, the title and artist,
//! and a progress bar, refreshed from the JSON once a second. Made for OBS
//! browser sources, so the clear theme has no background at all.
//!
//! Options come from the query string and only the listed values are used:
//! `size` small, medium (default) or large; `theme` dark (default), light or
//! clear; `art` and `progress` 1 (default) or 0. Song text is set with
//! `textContent`, never as HTML.

/// Scripts and styles are inline; the page may only talk to this server.
pub const CSP: &str = "default-src 'none'; img-src 'self' data:; style-src 'unsafe-inline'; script-src 'unsafe-inline'; connect-src 'self'";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Size {
    Small,
    Medium,
    Large,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Theme {
    Dark,
    Light,
    Clear,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PageOptions {
    pub size: Size,
    pub theme: Theme,
    pub art: bool,
    pub progress: bool,
}

impl Default for PageOptions {
    fn default() -> Self {
        Self { size: Size::Medium, theme: Theme::Dark, art: true, progress: true }
    }
}

fn flag(v: Option<&str>, default: bool) -> bool {
    match v.map(|s| s.trim().to_ascii_lowercase()).as_deref() {
        Some("1" | "true" | "yes" | "on") => true,
        Some("0" | "false" | "no" | "off") => false,
        _ => default,
    }
}

impl PageOptions {
    pub fn from_query(q: &[(String, String)]) -> Self {
        let get = |k: &str| q.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str());
        let d = Self::default();
        Self {
            size: match get("size").map(str::to_ascii_lowercase).as_deref() {
                Some("small" | "s") => Size::Small,
                Some("large" | "l") => Size::Large,
                _ => d.size,
            },
            theme: match get("theme").map(str::to_ascii_lowercase).as_deref() {
                Some("light") => Theme::Light,
                Some("clear" | "transparent") => Theme::Clear,
                _ => d.theme,
            },
            art: flag(get("art"), d.art),
            progress: flag(get("progress"), d.progress),
        }
    }

    /// Classes on the page's root, from the fixed lists above only.
    pub fn classes(&self) -> String {
        let size = match self.size {
            Size::Small => "size-small",
            Size::Medium => "size-medium",
            Size::Large => "size-large",
        };
        let theme = match self.theme {
            Theme::Dark => "theme-dark",
            Theme::Light => "theme-light",
            Theme::Clear => "theme-clear",
        };
        let mut c = format!("{size} {theme}");
        if !self.art {
            c.push_str(" no-art");
        }
        if !self.progress {
            c.push_str(" no-progress");
        }
        c
    }
}

const TEMPLATE: &str = r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Now playing</title>
<style>
  html, body { margin: 0; background: transparent; }
  body { font-family: "Segoe UI", -apple-system, system-ui, sans-serif; }
  .card { display: inline-flex; align-items: center; gap: var(--gap); padding: var(--pad); border-radius: 12px; max-width: var(--max); box-sizing: border-box; }
  .card[hidden] { display: none; }
  .theme-dark .card { background: rgba(18, 17, 16, 0.86); color: #f4efe6; }
  .theme-light .card { background: rgba(255, 253, 248, 0.92); color: #121110; }
  .theme-clear .card { background: transparent; color: #fff; text-shadow: 0 1px 3px rgba(0, 0, 0, 0.8); }
  .size-small { --art: 48px; --title: 14px; --sub: 12px; --pad: 8px; --gap: 10px; --max: 320px; }
  .size-medium { --art: 72px; --title: 18px; --sub: 14px; --pad: 12px; --gap: 14px; --max: 440px; }
  .size-large { --art: 112px; --title: 26px; --sub: 18px; --pad: 16px; --gap: 18px; --max: 640px; }
  .art { width: var(--art); height: var(--art); border-radius: 6px; object-fit: cover; flex: none; background: rgba(128, 128, 128, 0.25); }
  .no-art .art { display: none; }
  .text { min-width: 0; flex: 1; }
  .title, .artist { white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .title { font-size: var(--title); font-weight: 700; }
  .artist { font-size: var(--sub); opacity: 0.75; margin-top: 2px; }
  .bar { height: 4px; border-radius: 2px; margin-top: 8px; background: rgba(128, 128, 128, 0.35); overflow: hidden; }
  .bar span { display: block; height: 100%; width: 0; background: currentColor; }
  .no-progress .bar { display: none; }
</style>
</head>
<body class="__CLASSES__">
<div class="card" id="card" hidden>
  <img class="art" id="art" alt="">
  <div class="text">
    <div class="title" id="title"></div>
    <div class="artist" id="artist"></div>
    <div class="bar"><span id="fill"></span></div>
  </div>
</div>
<script>
(() => {
  const $ = (id) => document.getElementById(id);
  let now = null;
  let at = 0;
  const draw = () => {
    if (!now || !now.title) { $("card").hidden = true; return; }
    $("card").hidden = false;
    $("title").textContent = now.title;
    $("artist").textContent = now.artist || "";
    if (now.art && $("art").getAttribute("src") !== now.art) $("art").src = now.art;
    if (!now.art) $("art").removeAttribute("src");
    const pos = now.positionMs + (now.playing ? Date.now() - at : 0);
    const pct = now.durationMs > 0 ? Math.min(100, (pos / now.durationMs) * 100) : 0;
    $("fill").style.width = pct + "%";
  };
  const poll = async () => {
    try {
      const r = await fetch("/now-playing.json", { cache: "no-store" });
      now = await r.json();
      at = Date.now();
    } catch (e) {
      now = null;
    }
    draw();
  };
  poll();
  setInterval(poll, 1000);
  setInterval(draw, 250);
})();
</script>
</body>
</html>
"#;

pub fn render(opts: &PageOptions) -> String {
    TEMPLATE.replace("__CLASSES__", &opts.classes())
}
