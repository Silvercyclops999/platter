import { describe, expect, it } from "vitest";
import tauriConf from "../../src-tauri/tauri.conf.json";
import css from "../design/tokens.css?raw";
import { cssVariableTable, displaySize, fontSize, fontWeight, motion, radius, space } from "../design/tokens";

function rootBlock(): string {
  const start = css.indexOf(":root {");
  return css.slice(start, css.indexOf("\n}", start));
}

describe("design tokens", () => {
  it("declares every TS token as a CSS variable with the same value", () => {
    const block = rootBlock();
    for (const [name, value] of Object.entries(cssVariableTable())) {
      const m = block.match(new RegExp(`--${name.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}:\\s*([^;]+);`));
      expect(m, `--${name} missing from tokens.css`).not.toBeNull();
      expect(m![1].trim().toLowerCase()).toBe(value.toLowerCase());
    }
  });

  it("uses the type scale, weights, spacing grid, radii and motion", () => {
    expect(Object.values(fontSize)).toEqual([12, 13, 14, 16, 20, 24, 32, 48]);
    expect(Object.values(fontWeight)).toEqual([400, 600, 700]);
    expect(Object.values(space)).toEqual([4, 8, 12, 16, 24, 32, 48, 64]);
    for (const v of Object.values(space)) expect(v % 4).toBe(0);
    expect(radius).toEqual({ row: 4, card: 8, pill: 999 });
    const bands = [motion.instant, motion.veryQuick, motion.quick, motion.medium, motion.slow, motion.long];
    expect(bands[0]).toBe(0);
    for (let i = 1; i < bands.length; i++) expect(bands[i]).toBeGreaterThan(bands[i - 1]);
    expect(motion.veryQuick).toBeLessThan(100);
    expect(motion.long).toBeGreaterThanOrEqual(800);
    // Display steps: five, each about two thirds of the one above.
    expect(displaySize).toHaveLength(5);
    for (let i = 1; i < 5; i++) expect(displaySize[i] / displaySize[i - 1]).toBeGreaterThan(0.6);
    for (let i = 1; i < 4; i++) expect(displaySize[i] / displaySize[i - 1]).toBeLessThan(0.75);
  });

  it("matches the window background set in tauri.conf.json", () => {
    expect(cssVariableTable()["color-bg-base"].toLowerCase()).toBe(tauriConf.app.windows[0].backgroundColor.toLowerCase());
  });

  it("defines the connection status colors", () => {
    const t = cssVariableTable();
    for (const k of ["local", "remote", "relay", "offline"]) expect(t[`color-status-${k}`]).toMatch(/^#[0-9A-F]{6}$/i);
  });
});
