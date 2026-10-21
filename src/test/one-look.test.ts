import { describe, expect, it } from "vitest";
import phoneCss from "../phone/phone.css?raw";
import identityCss from "../design/identity.css?raw";
import { BUILTIN_THEMES } from "../design/themes";
import { contrastAgainst } from "../design/looks/contrast";

describe("one look on the phone and the computer", () => {
  it("the phone sets no colour tokens of its own", () => {
    // Every colour comes from the look; a phone-only palette is how the two drifted apart.
    const sets = [...phoneCss.matchAll(/--color-[a-z0-9-]+\s*:/g)].map((m) => m[0]);
    expect(sets).toEqual([]);
  });

  it("Sleeve is clean paper on every screen: no grain outside print", () => {
    expect(identityCss).toMatch(/--paper-grain:\s*none;/);
    expect(BUILTIN_THEMES.sleeve.tokens["color-bg-base"]).toBe("#F4F0E8");
  });

  it("Sleeve's text and brass read on all its surfaces", () => {
    const t = BUILTIN_THEMES.sleeve.tokens;
    for (const bg of ["color-bg-base", "color-bg-raised", "color-bg-raised2", "color-bg-overlay"]) {
      for (const fg of ["color-text-primary", "color-text-secondary", "color-text-muted", "color-accent"]) {
        expect(contrastAgainst(t[fg], [bg], t), `${fg} on ${bg}`).toBeGreaterThanOrEqual(4.5);
      }
    }
  });

  it("a chosen chip on the phone is brass, like the computer's", () => {
    const rule = phoneCss.match(/\.ph-chip\.is-on\s*\{([^}]*)\}/)?.[1] ?? "";
    expect(rule).toMatch(/background:\s*var\(--color-accent\)/);
    expect(rule).toMatch(/color:\s*var\(--color-text-on-accent\)/);
  });
});
