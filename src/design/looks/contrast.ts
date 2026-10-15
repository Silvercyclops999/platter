/**
 * Contrast checks for the look builder: text needs 4.5:1 (WCAG AA) against
 * the surfaces it sits on, controls and the focus ring 3:1. The fix-it
 * nudges a failing colour's lightness to the nearest value that passes,
 * keeping its hue and saturation.
 */
import { contrastRatio, hslToRgb, parseColour, rgbToHsl, type RGB } from "../../lib/color";
import { toHex } from "../themes/accent";

export interface ContrastRule {
  token: string;
  /** Surfaces it must stand out from (the lowest ratio counts). */
  against: string[];
  min: number;
  kind: "text" | "control";
}

const SURFACES = ["color-bg-base", "color-bg-raised"];

export const CONTRAST_RULES: ContrastRule[] = [
  { token: "color-text-primary", against: [...SURFACES, "color-bg-raised2"], min: 4.5, kind: "text" },
  { token: "color-text-secondary", against: SURFACES, min: 4.5, kind: "text" },
  { token: "color-text-muted", against: SURFACES, min: 4.5, kind: "text" },
  { token: "color-playing", against: SURFACES, min: 4.5, kind: "text" },
  { token: "color-text-on-accent", against: ["color-accent"], min: 4.5, kind: "text" },
  { token: "color-accent", against: SURFACES, min: 3, kind: "control" },
  { token: "color-focus-ring", against: SURFACES, min: 3, kind: "control" },
];

export function ruleFor(token: string): ContrastRule | undefined {
  return CONTRAST_RULES.find((r) => r.token === token);
}

/** The lowest contrast of a colour against the rule's surfaces (null when it can't be told). */
export function contrastAgainst(colour: string, against: string[], tokens: Record<string, string>): number | null {
  const c = parseColour(colour);
  if (!c) return null;
  const backs = against.map((k) => parseColour(tokens[k] ?? "")).filter((x): x is RGB => !!x);
  if (!backs.length) return null;
  return Math.min(...backs.map((b) => contrastRatio(c, b)));
}

export interface ContrastResult {
  rule: ContrastRule;
  ratio: number;
  passes: boolean;
}

/** Every rule's result for a token set. */
export function contrastReport(tokens: Record<string, string>): ContrastResult[] {
  const out: ContrastResult[] = [];
  for (const rule of CONTRAST_RULES) {
    const ratio = contrastAgainst(tokens[rule.token] ?? "", rule.against, tokens);
    if (ratio !== null) out.push({ rule, ratio, passes: ratio >= rule.min });
  }
  return out;
}

/**
 * The nearest colour to `colour` (same hue and saturation, lightness moved
 * as little as possible) with at least `min` contrast against every surface.
 * Null when no lightness passes.
 */
export function fixColour(colour: string, against: string[], tokens: Record<string, string>, min: number): string | null {
  const c = parseColour(colour);
  if (!c) return null;
  const [h, s, l] = rgbToHsl(c);
  const ok = (light: number) => (contrastAgainst(toHex(hslToRgb(h, s, light)), against, tokens) ?? 0) >= min + 0.05;
  if (ok(l)) return toHex(c);
  for (let step = 0.005; step <= 1; step += 0.005) {
    for (const light of [l + step, l - step]) {
      if (light < 0 || light > 1) continue;
      if (ok(light)) return toHex(hslToRgb(h, s, light));
    }
  }
  return null;
}
