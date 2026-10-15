import { afterEach, describe, expect, it, vi } from "vitest";
import componentsCss from "../components/components.css?raw";
import globalCss from "../design/global.css?raw";
import { duration, installReducedMotion, prefersReducedMotion } from "../design/motion";
import { SCROLLBAR_IDLE_MS, covered, installOverlayScrollbars, thumbBox, thumbZ } from "../design/scrollbars";
import tokensCss from "../design/tokens.css?raw";
import { layer } from "../design/tokens";
import pagesCss from "../pages/pages.css?raw";
import fullWindowCss from "../shell/full-window.css?raw";
import shellCss from "../shell/shell.css?raw";
import topbarCss from "../shell/topbar.css?raw";

function fakeMatchMedia(initial: boolean) {
  let listener: ((e: MediaQueryListEvent) => void) | null = null;
  const mq = {
    matches: initial,
    media: "(prefers-reduced-motion: reduce)",
    addEventListener: (_: string, cb: (e: MediaQueryListEvent) => void) => (listener = cb),
    removeEventListener: () => (listener = null),
  };
  return {
    matchMedia: () => mq as unknown as MediaQueryList,
    fire: (matches: boolean) => listener?.({ matches } as MediaQueryListEvent),
    hasListener: () => listener !== null,
  };
}

describe("reduced motion", () => {
  const original = window.matchMedia;
  afterEach(() => {
    window.matchMedia = original;
    delete document.documentElement.dataset.reducedMotion;
  });

  it("mirrors the OS setting onto <html data-reduced-motion> and follows changes", () => {
    const fake = fakeMatchMedia(true);
    window.matchMedia = fake.matchMedia;
    const stop = installReducedMotion(window);
    expect(document.documentElement.dataset.reducedMotion).toBe("true");
    expect(prefersReducedMotion()).toBe(true);
    expect(duration(350)).toBe(0);

    fake.fire(false);
    expect(document.documentElement.dataset.reducedMotion).toBe("false");
    expect(duration(350)).toBe(350);

    stop();
    expect(fake.hasListener()).toBe(false);
  });

  it("reduced motion drops movement but keeps fades and colour changes", () => {
    const css = tokensCss;
    // Movement distances and scales are multiplied by --motion-move, which is 0 when reduced.
    const attr = css.slice(css.indexOf(':root[data-reduced-motion="true"] {'));
    expect(attr.slice(0, attr.indexOf("}"))).toMatch(/--motion-move:\s*0;/);
    // Durations are not zeroed (fades stay)...
    expect(css).not.toMatch(/--motion-(quick|medium|slow|long):\s*0ms/);
    expect(css).not.toMatch(/transition-duration:\s*0ms !important/);
    // ...and transitions are limited to opacity and colour, so transforms jump.
    const all = css.slice(css.indexOf(':root[data-reduced-motion="true"] *,'));
    const props = all.slice(all.indexOf("transition-property:"), all.indexOf(";"));
    for (const p of ["opacity", "color", "background-color", "--tint", "--tint-shade"]) expect(props).toContain(p);
    expect(props).not.toContain("transform");
  });

  it("every movement in the motion CSS scales with --motion-move", () => {
    const files = { "components.css": componentsCss, "shell.css": shellCss };
    for (const [f, text] of Object.entries(files)) {
      // Card plate, card play rise, icon hover scale, panel slide, rise-in.
      const moving = text.match(/(translate[XY]?|scale)\(calc\([^;]+/g) ?? [];
      expect(moving.length, f).toBeGreaterThan(0);
      for (const m of moving) expect(m, `${f}: ${m}`).toContain("var(--motion-move)");
    }
  });

  it("component CSS only animates through the motion tokens", () => {
    const files = { "components.css": componentsCss, "shell.css": shellCss, "pages.css": pagesCss };
    for (const [f, text] of Object.entries(files)) {
      // Any `transition:` declaration must use var(--motion-*) durations.
      const transitions = text.match(/transition:[^;]+;/g) ?? [];
      for (const t of transitions) expect(t, `${f}: ${t}`).toMatch(/var\(--motion-(very-quick|quick|fast|medium|slow|long)\)/);
    }
  });

  it("structural changes are instant: no page fade, view transition or menu fade", () => {
    expect(shellCss).not.toMatch(/\.page\s*\{[^}]*animation/);
    expect(shellCss).not.toContain("view-transition");
    const menu = componentsCss.slice(componentsCss.indexOf(".context-menu {"));
    expect(menu.slice(0, menu.indexOf("}"))).not.toMatch(/animation|transition/);
    const row = componentsCss.slice(componentsCss.indexOf(".track-row {\n  height"));
    expect(row.slice(0, row.indexOf("}"))).not.toMatch(/transition/);
    // The header colour is the one thing that eases across pages, over the long band.
    expect(shellCss).toMatch(/--tint var\(--motion-long\)/);
  });
});

describe("overlay scrollbars", () => {
  afterEach(() => vi.useRealTimers());

  it("show on scroll and hide after just under a second idle", () => {
    vi.useFakeTimers();
    const stop = installOverlayScrollbars(window);
    const el = document.createElement("div");
    document.body.appendChild(el);
    try {
      el.dispatchEvent(new Event("scroll"));
      expect(el).toHaveClass("sb", "sb-on");
      vi.advanceTimersByTime(SCROLLBAR_IDLE_MS - 100);
      el.dispatchEvent(new Event("scroll")); // still active: the timer restarts
      vi.advanceTimersByTime(SCROLLBAR_IDLE_MS - 100);
      expect(el).toHaveClass("sb-on");
      vi.advanceTimersByTime(100);
      expect(el).not.toHaveClass("sb-on");
      expect(el).toHaveClass("sb"); // keeps the transition, so hiding fades too
      expect(SCROLLBAR_IDLE_MS).toBeLessThan(1000);
    } finally {
      stop();
      el.remove();
    }
  });

  it("native bars take no width; the drawn thumb fades", () => {
    expect(globalCss).toMatch(/\* \{ scrollbar-width: none; \}/);
    expect(globalCss).toMatch(/::-webkit-scrollbar \{ width: 0; height: 0; \}/);
    expect(globalCss).toMatch(/\.sb-thumb \{[^}]*transition: opacity var\(--motion-quick\)/);
    expect(shellCss).not.toMatch(/scrollbar-gutter/);
  });

  it("draws a thumb over the scroller's right edge, sized and placed by scroll position", () => {
    const m = { clientWidth: 300, clientHeight: 400, scrollHeight: 1600, scrollTop: 0 };
    const top = thumbBox({ top: 100, left: 20 }, m)!;
    expect(top.left).toBe(20 + 300 - 8 - 2);
    expect(top.top).toBe(102);
    expect(top.height).toBe(Math.round((396 * 400) / 1600));
    const end = thumbBox({ top: 100, left: 20 }, { ...m, scrollTop: 1200 })!;
    expect(end.top + end.height).toBe(100 + 400 - 2);
    expect(thumbBox({ top: 0, left: 0 }, { ...m, scrollHeight: 400 })).toBeNull();
    // A very long list keeps a thumb big enough to grab.
    expect(thumbBox({ top: 0, left: 0 }, { ...m, scrollHeight: 400000 })!.height).toBe(24);
  });

  it("the thumb follows its scroller and is removed after it fades out", () => {
    vi.useFakeTimers();
    const stop = installOverlayScrollbars(window);
    const el = document.createElement("div");
    Object.defineProperties(el, {
      clientHeight: { value: 400 },
      clientWidth: { value: 300 },
      scrollHeight: { value: 1600 },
    });
    document.body.appendChild(el);
    try {
      el.dispatchEvent(new Event("scroll"));
      const thumb = document.querySelector(".sb-layer .sb-thumb");
      expect(thumb).toHaveClass("is-on");
      vi.advanceTimersByTime(SCROLLBAR_IDLE_MS);
      expect(thumb).not.toHaveClass("is-on");
      vi.advanceTimersByTime(500);
      expect(document.querySelector(".sb-thumb")).toBeNull();
    } finally {
      stop();
      el.remove();
    }
  });
});

describe("scroll thumbs under menus", () => {
  it("counts a thumb as covered when another element is on top of its spot", () => {
    const scroller = document.createElement("div");
    const row = document.createElement("div");
    scroller.appendChild(row);
    const menu = document.createElement("div");
    document.body.append(scroller, menu);
    const box = { top: 10, left: 100, height: 50 };
    const orig = document.elementsFromPoint;
    try {
      document.elementsFromPoint = () => [row, scroller];
      expect(covered(scroller, box)).toBe(false);
      document.elementsFromPoint = () => [menu, row, scroller];
      expect(covered(scroller, box)).toBe(true);
    } finally {
      document.elementsFromPoint = orig;
      scroller.remove();
      menu.remove();
    }
  });
});

describe("scroll thumbs stack under every floating layer", () => {
  /** The z-index a rule sets, with `var(--z-*)` read from tokens.css. */
  function zOf(css: string, selector: string): number {
    const at = css.indexOf(`${selector} {`);
    expect(at, `${selector} not found`).toBeGreaterThanOrEqual(0);
    const rule = css.slice(at, css.indexOf("}", at));
    const m = rule.match(/z-index:\s*([^;]+);/);
    expect(m, `${selector} sets no z-index`).not.toBeNull();
    const v = m![1].trim();
    const ref = v.match(/^var\((--[a-z-]+)\)$/);
    if (!ref) return Number(v);
    const def = tokensCss.match(new RegExp(`${ref[1]}:\\s*(\\d+);`));
    expect(def, `${ref[1]} missing from tokens.css`).not.toBeNull();
    return Number(def![1]);
  }

  it("menus, dialogs, toasts and tooltips are above the scrollbar layer", () => {
    const thumb = zOf(globalCss, ".sb-thumb");
    expect(thumb).toBe(layer.scrollbar);
    const above: [string, string][] = [
      [topbarCss, ".topbar"], // the app (profile) menu and search panel hang from it over the sidebar
      [shellCss, ".status-popover"],
      [fullWindowCss, ".fw"],
      [componentsCss, ".dialog-backdrop"],
      [pagesCss, ".cover-popup"],
      [componentsCss, ".context-menu"], // Sort by / View as and every right-click menu
      [componentsCss, ".toaster"],
      [componentsCss, ".tooltip"],
    ];
    for (const [css, sel] of above) expect(zOf(css, sel), sel).toBeGreaterThan(thumb);
    const order = Object.values(layer);
    expect([...order].sort((a, b) => a - b)).toEqual(order);
  });

  it("the thumb holder makes no stacking context of its own", () => {
    const at = globalCss.indexOf(".sb-layer {");
    const rule = globalCss.slice(at, globalCss.indexOf("}", at));
    expect(rule).not.toMatch(/z-index/);
    expect(rule).not.toMatch(/position:\s*(fixed|absolute|relative|sticky)/);
  });

  it("a page scroller's thumb sits at the scrollbar layer, a dialog's just over its dialog", () => {
    const sidebar = document.createElement("nav");
    sidebar.style.position = "relative";
    sidebar.style.zIndex = "2";
    const dialog = document.createElement("div");
    dialog.style.position = "fixed";
    dialog.style.zIndex = String(layer.dialog);
    const body = document.createElement("div");
    body.style.position = "relative";
    body.style.zIndex = "1";
    dialog.appendChild(body);
    document.body.append(sidebar, dialog);
    try {
      expect(thumbZ(sidebar)).toBe(layer.scrollbar);
      expect(thumbZ(body)).toBe(layer.dialog + 1);
      expect(thumbZ(body)).toBeLessThan(layer.menu);
    } finally {
      sidebar.remove();
      dialog.remove();
    }
  });

  it("a shown thumb carries its z-index", () => {
    vi.useFakeTimers();
    const stop = installOverlayScrollbars(window);
    const el = document.createElement("div");
    Object.defineProperties(el, { clientHeight: { value: 400 }, clientWidth: { value: 300 }, scrollHeight: { value: 1600 } });
    document.body.appendChild(el);
    try {
      el.dispatchEvent(new Event("scroll"));
      const thumb = document.querySelector<HTMLElement>(".sb-thumb")!;
      expect(Number(thumb.style.zIndex)).toBe(layer.scrollbar);
      expect(Number(thumb.style.zIndex)).toBeLessThan(layer.menu);
    } finally {
      stop();
      el.remove();
      vi.useRealTimers();
    }
  });
});
