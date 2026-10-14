import { afterEach, describe, expect, it, vi } from "vitest";
import componentsCss from "../components/components.css?raw";
import globalCss from "../design/global.css?raw";
import { duration, installReducedMotion, prefersReducedMotion } from "../design/motion";
import { SCROLLBAR_IDLE_MS, installOverlayScrollbars } from "../design/scrollbars";
import tokensCss from "../design/tokens.css?raw";
import pagesCss from "../pages/pages.css?raw";
import shellCss from "../shell/shell.css?raw";

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

  it("the thumb colour follows --sb-alpha, which fades", () => {
    expect(globalCss).toMatch(/scrollbar-color:[^;]*var\(--sb-alpha\)/);
    expect(globalCss).toMatch(/\.sb \{ transition: --sb-alpha var\(--motion-quick\)/);
  });
});
