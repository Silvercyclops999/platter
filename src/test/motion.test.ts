import { afterEach, describe, expect, it } from "vitest";
import componentsCss from "../components/components.css?raw";
import { duration, installReducedMotion, prefersReducedMotion } from "../design/motion";
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

  it("CSS zeroes every motion token and kills transitions when reduced", () => {
    const css = tokensCss;
    const media = css.slice(css.indexOf("@media (prefers-reduced-motion: reduce)"));
    const attr = css.slice(css.indexOf(':root[data-reduced-motion="true"] {'));
    for (const block of [media, attr]) {
      const body = block.slice(0, block.indexOf("}"));
      expect(body).toMatch(/--motion-fast:\s*0ms/);
      expect(body).toMatch(/--motion-medium:\s*0ms/);
      expect(body).toMatch(/--motion-slow:\s*0ms/);
    }
    expect(css).toMatch(/:root\[data-reduced-motion="true"\] \*[\s\S]*transition-duration:\s*0ms !important/);
    expect(css).toMatch(/animation-duration:\s*0ms !important/);
  });

  it("component CSS only animates through the motion tokens", () => {
    const files = { "components.css": componentsCss, "shell.css": shellCss, "pages.css": pagesCss };
    for (const [f, text] of Object.entries(files)) {
      // Any `transition:` declaration must use var(--motion-*) durations.
      const transitions = text.match(/transition:[^;]+;/g) ?? [];
      for (const t of transitions) expect(t, `${f}: ${t}`).toMatch(/var\(--motion-(fast|medium|slow)\)/);
    }
  });
});
