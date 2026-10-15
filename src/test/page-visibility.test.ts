import { afterEach, describe, expect, it } from "vitest";
import tokens from "../design/tokens.css?raw";
import { installPageVisibility, isPageHidden } from "../design/pageVisibility";

function setHidden(hidden: boolean) {
  Object.defineProperty(document, "visibilityState", { configurable: true, get: () => (hidden ? "hidden" : "visible") });
  Object.defineProperty(document, "hidden", { configurable: true, get: () => hidden });
  document.dispatchEvent(new Event("visibilitychange"));
}

afterEach(() => setHidden(false));

describe("page visibility", () => {
  it("mirrors a hidden window onto <html data-page-hidden>", () => {
    const off = installPageVisibility();
    expect(document.documentElement.dataset.pageHidden).toBe("false");
    setHidden(true);
    expect(isPageHidden()).toBe(true);
    expect(document.documentElement.dataset.pageHidden).toBe("true");
    setHidden(false);
    expect(document.documentElement.dataset.pageHidden).toBe("false");
    off();
  });

  it("pauses every animation while hidden", () => {
    expect(tokens).toMatch(/:root\[data-page-hidden="true"\][^{]*\{\s*animation-play-state: paused !important;/);
  });
});
