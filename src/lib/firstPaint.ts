/**
 * The main window opens hidden and the core shows it once the page has
 * drawn its first real frame, so it never appears blank or in the wrong
 * colours. `signalReady` is called once the shell is on the page with the
 * look applied: after the next frame has been drawn (two animation frames)
 * it tells the core, with the look's background colour so the next launch
 * can colour the window to match before anything draws.
 */
import { backend, type Backend } from "./backend";

/** If animation frames don't run (a hidden page can pause them), go anyway after this. */
export const READY_FALLBACK_MS = 150;

/** How long the page waits for the sign-in state before showing the window anyway (the shell frame). */
export const READY_WAIT_FOR_AUTH_MS = 800;

let sent = false;

/** `rgb(r, g, b)` / `rgba(...)` / `#rrggbb` as `#rrggbb`; null otherwise or when see-through. */
export function toHex(css: string): string | null {
  const s = css.trim().toLowerCase();
  if (/^#[0-9a-f]{6}$/.test(s)) return s;
  const m = s.match(/^rgba?\(\s*(\d+)[\s,]+(\d+)[\s,]+(\d+)(?:[\s,/]+([\d.]+%?))?\s*\)$/);
  if (!m) return null;
  if (m[4] !== undefined) {
    const a = m[4].endsWith("%") ? parseFloat(m[4]) / 100 : parseFloat(m[4]);
    if (a < 1) return null;
  }
  const hex = (v: string) => Math.min(255, Number(v)).toString(16).padStart(2, "0");
  return `#${hex(m[1])}${hex(m[2])}${hex(m[3])}`;
}

/** The look's background colour as the page shows it. */
export function pageBackground(doc: Document = document): string | null {
  const view = doc.defaultView;
  if (!view) return null;
  for (const el of [doc.body, doc.documentElement]) {
    if (!el) continue;
    const hex = toHex(view.getComputedStyle(el).backgroundColor);
    if (hex) return hex;
  }
  return null;
}

/** Wait for the next drawn frame, or the fallback, whichever is first. */
export function afterNextPaint(win: Window = window, fallbackMs = READY_FALLBACK_MS): Promise<void> {
  return new Promise((resolve) => {
    let done = false;
    const finish = () => {
      if (done) return;
      done = true;
      resolve();
    };
    const timer = win.setTimeout(finish, fallbackMs);
    if (typeof win.requestAnimationFrame === "function")
      win.requestAnimationFrame(() =>
        win.requestAnimationFrame(() => {
          win.clearTimeout(timer);
          finish();
        }),
      );
  });
}

/** Tell the core the page has drawn (once per page load). */
export async function signalReady(b: Backend = backend, win: Window = window): Promise<void> {
  if (sent) return;
  sent = true;
  await afterNextPaint(win);
  try {
    await b.windowReady(pageBackground(win.document));
  } catch {
    // The core shows the window on its own after a few seconds.
  }
}

/** Tests: allow another signal. */
export function resetReadyForTests(): void {
  sent = false;
}
