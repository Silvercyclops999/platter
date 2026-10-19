import { act, render } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import instrumentsCss from "../design/instruments.css?raw";
import tokensCss from "../design/tokens.css?raw";
import { COVER_LIGHT_MIX, coverLightColour, mixRgb } from "../design/coverLight";
import { GLASS, PLATES, PLATE_MIN_CONTRAST, type InstrumentLook } from "../design/instruments";
import { STUDIO_FONTS } from "../design/looks/apply";
import { READOUT_FAMILY } from "../design/looks/fonts";
import { BUILTIN_THEMES } from "../design/themes";
import { backend } from "../lib/backend";
import { contrastRatio, parseColour, type RGB } from "../lib/color";
import { DEFAULT_SETTINGS, type Settings } from "../lib/types";
import { NOTICES } from "../pages/SettingsPage";
import { CoverLightSource } from "../shell/CoverLightSource";
import { usePlayer } from "../stores/player";
import { loadSettings, updateSettings, useSettings } from "../stores/settings";
import { installTheme, lookPartsFrom } from "../stores/theme";

// The cover's colour, as the image sampler would give it.
let coverVivid = "rgb(200 40 160)";
vi.mock("../shell/nowPlaying", async (orig) => ({
  ...(await orig<typeof import("../shell/nowPlaying")>()),
  useCoverColors: (thumb: string | null | undefined) => (thumb ? { vivid: coverVivid, shade: "rgb(30 10 25)" } : null),
}));

const LOOKS: InstrumentLook[] = ["studio", "studioBlack", "sleeve"];
const root = () => document.documentElement;
const rgb = (v: string): RGB => {
  const c = parseColour(v);
  if (!c) throw new Error(`not a colour: ${v}`);
  return c;
};
const ratio = (a: string, b: string) => contrastRatio(rgb(a), rgb(b));

/** The declarations of the first rule whose selector is exactly `selector`. */
function block(css: string, selector: string): Record<string, string> {
  const start = css.indexOf(`${selector} {`);
  expect(start, selector).toBeGreaterThanOrEqual(0);
  const body = css.slice(start, css.indexOf("}", start));
  return Object.fromEntries([...body.matchAll(/--([a-z-]+):\s*([^;]+);/g)].map(([, k, v]) => [k, v.trim()]));
}

beforeEach(async () => {
  if (typeof window.matchMedia !== "function") window.matchMedia = (() => ({ matches: true, addEventListener() {}, removeEventListener() {} })) as never;
  await backend.settingsSet({ ...DEFAULT_SETTINGS });
  useSettings.setState({ settings: null });
  await loadSettings();
});
afterEach(() => {
  for (const k of ["theme", "themeBase", "identity", "readout", "coverLight"]) delete root().dataset[k];
  root().removeAttribute("style");
  usePlayer.setState({ state: null });
  localStorage.clear();
});

describe("type", () => {
  it("sets titles in the display serif on Studio, Studio Black and Sleeve, and not on Classic", () => {
    for (const theme of ["studio", "studioBlack", "sleeve"] as const) {
      expect(lookPartsFrom({ ...DEFAULT_SETTINGS, theme, themeFile: null } as Settings).fonts, theme).toEqual(STUDIO_FONTS);
    }
    expect(STUDIO_FONTS.headings).toBe("fraunces");
    expect(lookPartsFrom({ ...DEFAULT_SETTINGS, theme: "dark", themeFile: null } as Settings).fonts.headings).toBe("inter");
    // A theme file's own fonts win over the look's.
    const file = { version: 1, name: "Mine", base: "dark", fonts: { text: "inter", headings: "manrope" } } as unknown as Settings["themeFile"];
    expect(lookPartsFrom({ ...DEFAULT_SETTINGS, theme: "studio", themeFile: file } as Settings).fonts.headings).toBe("manrope");
  });

  it("puts the Readout face setting on the page and uses the bundled face only while it's on", async () => {
    const un = installTheme();
    expect(root().dataset.identity).toBe("studio");
    expect(root().dataset.readout).toBe("on");
    await act(() => updateSettings({ readoutFace: false }));
    expect(root().dataset.readout).toBe("off");
    await act(() => updateSettings({ readoutFace: true }));
    expect(root().dataset.readout).toBe("on");
    un();
    // The face is set only for Platter's looks with the setting on; off keeps the body face.
    const rule = instrumentsCss.match(/:root\[data-identity\]:not\(\[data-readout="off"\]\) \{\s*--font-readout:\s*([^;]+);/);
    expect(rule?.[1]).toContain(`"${READOUT_FAMILY}"`);
    expect(rule?.[1]).toContain("ui-monospace");
    expect(block(instrumentsCss, ":root")["font-readout"]).toBe("var(--font-family)");
    // Its licence is listed under About.
    expect(NOTICES.some((n) => n.name === `${READOUT_FAMILY} typeface` && /Open Font License/.test(n.licence))).toBe(true);
  });
});

describe("plates and glass", () => {
  it.each(LOOKS)("%s: instruments.css carries the same plate and glass values as instruments.ts", (look) => {
    const b = block(instrumentsCss, `:root[data-theme="${look}"]`);
    const p = PLATES[look];
    expect([b["plate-bg"], b["plate-text"], b["plate-lit"], b["plate-warn"], b["plate-edge"]]).toEqual([p.bg, p.text, p.lit, p.warn, p.edge]);
    const g = GLASS[look];
    expect([b["glass-top"], b["glass-bottom"], b["glass-text"], b["glass-dim"], b["glass-lit"]]).toEqual([g.top, g.bottom, g.text, g.dim, g.lit]);
  });

  it.each(LOOKS)("%s: plate text is at least AA on its plate", (look) => {
    const p = PLATES[look];
    for (const [name, colour] of Object.entries({ text: p.text, lit: p.lit, warn: p.warn })) {
      expect(ratio(colour, p.bg), `${look} plate ${name}`).toBeGreaterThanOrEqual(PLATE_MIN_CONTRAST);
    }
  });

  it.each(LOOKS)("%s: readout text is at least AA on both ends of the glass", (look) => {
    const g = GLASS[look];
    for (const back of [g.top, g.bottom]) {
      for (const [name, colour] of Object.entries({ text: g.text, dim: g.dim, lit: g.lit })) {
        expect(ratio(colour, back), `${look} glass ${name}`).toBeGreaterThanOrEqual(4.5);
      }
    }
  });

  it("Sleeve's plates over artwork (expanded view, miniplayer) are Studio's dark ones", () => {
    const start = instrumentsCss.indexOf(':root[data-theme="sleeve"] :is(.fw');
    const body = instrumentsCss.slice(start, instrumentsCss.indexOf("}", start));
    expect(body).toContain(`--plate-bg: ${PLATES.studio.bg};`);
    expect(body).toContain(`--plate-text: ${PLATES.studio.text};`);
  });

  it("uses no blur, no backdrop filter and no looping animation", () => {
    expect(instrumentsCss).not.toMatch(/backdrop-filter|filter:\s*blur|@keyframes|animation\s*:|infinite/);
  });
});

describe("cover light", () => {
  const covers = { white: "#FFFFFF", black: "#000000", neon: "#39FF14", yellow: "#FFE600", grey: "#808080" };

  it.each(LOOKS)("%s: text stays AA over the brightest pool for any cover", (look) => {
    const t = BUILTIN_THEMES[look].tokens;
    const base = BUILTIN_THEMES[look].base;
    for (const [name, cover] of Object.entries(covers)) {
      const light = coverLightColour(cover, base);
      expect(light, name).not.toBeNull();
      for (const surface of [t["color-bg-raised"], t["color-bg-base"]]) {
        const lit = mixRgb(rgb(light!), rgb(surface), COVER_LIGHT_MIX[base].on);
        for (const text of [t["color-text-primary"], t["color-text-secondary"], t["color-text-muted"]]) {
          expect(contrastRatio(rgb(text), lit), `${look} ${name} ${text}`).toBeGreaterThanOrEqual(4.5);
        }
      }
    }
  });

  it("clamps saturation, so a neon cover glows rather than floods", () => {
    const [, s] = (() => {
      const c = rgb(coverLightColour("#39FF14", "dark")!);
      const max = Math.max(...c) / 255;
      const min = Math.min(...c) / 255;
      const l = (max + min) / 2;
      return [l, (max - min) / (1 - Math.abs(2 * l - 1))];
    })();
    expect(s).toBeLessThanOrEqual(0.56);
    expect(COVER_LIGHT_MIX.dark.less).toBeLessThan(COVER_LIGHT_MIX.dark.on);
    expect(COVER_LIGHT_MIX.light.less).toBeLessThan(COVER_LIGHT_MIX.light.on);
    // The CSS uses the same amounts.
    expect(block(instrumentsCss, ":root[data-identity]")["cover-light-mix"]).toBe(`${COVER_LIGHT_MIX.dark.on}%`);
    expect(block(instrumentsCss, ':root[data-identity][data-cover-light="less"]')["cover-light-mix"]).toBe(`${COVER_LIGHT_MIX.dark.less}%`);
    expect(block(instrumentsCss, ':root[data-identity][data-theme-base="light"]')["cover-light-mix"]).toBe(`${COVER_LIGHT_MIX.light.on}%`);
  });

  it("lights the page from the playing cover when on or less, and not when off or nothing plays", async () => {
    const un = installTheme();
    render(<CoverLightSource />);
    expect(root().style.getPropertyValue("--cover-light")).toBe("");
    act(() => usePlayer.setState({ state: { track: { id: "t1", thumb: "/art/1" } } as never }));
    const on = root().style.getPropertyValue("--cover-light");
    expect(on).toBe(coverLightColour(coverVivid, "dark"));
    expect(root().dataset.coverLight).toBe("on");
    await act(() => updateSettings({ coverLight: "less" }));
    expect(root().dataset.coverLight).toBe("less");
    expect(root().style.getPropertyValue("--cover-light")).toBe(on);
    await act(() => updateSettings({ coverLight: "off" }));
    expect(root().dataset.coverLight).toBe("off");
    expect(root().style.getPropertyValue("--cover-light")).toBe("");
    await act(() => updateSettings({ coverLight: "on", artworkColours: false }));
    expect(root().style.getPropertyValue("--cover-light")).toBe("");
    await act(() => updateSettings({ artworkColours: true }));
    coverVivid = "rgb(20 200 220)";
    act(() => usePlayer.setState({ state: { track: { id: "t2", thumb: "/art/2" } } as never }));
    expect(root().style.getPropertyValue("--cover-light")).toBe(coverLightColour(coverVivid, "dark"));
    act(() => usePlayer.setState({ state: null }));
    expect(root().style.getPropertyValue("--cover-light")).toBe("");
    // Off draws no pool at all: every lit rule waits on the setting.
    for (const m of instrumentsCss.matchAll(/^(.*)\{[^}]*var\(--cover-pool\)/gm)) expect(m[1]).toContain(':not([data-cover-light="off"])');
    un();
  });

  it("changes with the song only, and at once under reduced motion", () => {
    // The light eases only through its own transition on the lit panels.
    expect(instrumentsCss).toMatch(/transition: --cover-light var\(--motion-long\)/);
    // Reduced motion limits transitions to a list without the cover light, so it switches at once.
    const start = tokensCss.indexOf(':root[data-reduced-motion="true"] *,');
    const rule = tokensCss.slice(start, tokensCss.indexOf("}", start));
    expect(rule).toMatch(/transition-property:/);
    expect(rule).not.toContain("--cover-light");
  });
});
