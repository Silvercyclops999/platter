/**
 * Accent colours: brass (Studio's own), ivory (Classic's own), six more
 * presets and a custom colour.
 * Every preset has a tone for dark themes and one for light themes. A custom
 * colour must keep at least 3:1 contrast with the theme's background, or the
 * buttons it colours would get lost.
 */
import { contrastRatio, parseColour, type RGB } from "../../lib/color";

export { parseColour };
import contract from "./contract.json";
import type { ThemeBase } from "./validate";

export type AccentPresetId = "ivory" | "brass" | "sage" | "sky" | "lilac" | "amber" | "rose" | "teal";

export interface AccentPreset {
  id: AccentPresetId;
  label: string;
  dark: string;
  light: string;
}

export const ACCENT_PRESETS: AccentPreset[] = [
  { id: "ivory", label: "Ivory", dark: "#ECE4D3", light: "#5E574D" },
  { id: "brass", label: "Brass", dark: "#CDAE6E", light: "#7A5A1C" },
  { id: "sage", label: "Sage", dark: "#A9C8A0", light: "#3D6A38" },
  { id: "sky", label: "Sky", dark: "#9CC6F2", light: "#1F5F99" },
  { id: "lilac", label: "Lilac", dark: "#C6B3F0", light: "#5B43A3" },
  { id: "amber", label: "Amber", dark: "#F2C46D", light: "#875800" },
  { id: "rose", label: "Rose", dark: "#F2A7C3", light: "#A3365F" },
  { id: "teal", label: "Teal", dark: "#7FD1C6", light: "#1C6B62" },
];

export const ACCENT_IDS = contract.accents as AccentPresetId[];

/** The least contrast an accent needs against the background. */
export const ACCENT_MIN_CONTRAST = 3;

export function toHex([r, g, b]: RGB): string {
  return "#" + [r, g, b].map((c) => Math.round(Math.max(0, Math.min(255, c))).toString(16).padStart(2, "0")).join("").toUpperCase();
}

/** Mix two colours: t of b into a. */
export function mix(a: RGB, b: RGB, t: number): RGB {
  return [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t];
}

const WHITE: RGB = [255, 255, 255];
const BLACK: RGB = [0, 0, 0];
const DARK_TEXT: RGB = [18, 17, 16];
const LIGHT_TEXT: RGB = [255, 253, 248];

/** True when the setting value is a custom colour rather than a preset. */
export function isCustomAccent(v: string): boolean {
  return /^#[0-9a-f]{6}$/i.test(v);
}

/** The colour a setting value stands for on a base (null for an unknown value). */
export function accentColour(value: string, base: ThemeBase): string | null {
  if (isCustomAccent(value)) return value.toUpperCase();
  const p = ACCENT_PRESETS.find((x) => x.id === value);
  return p ? p[base] : null;
}

/** Contrast of a colour with the background surfaces it sits on (the lower of the two). */
export function accentContrast(accent: string, tokens: Record<string, string>): number {
  const a = parseColour(accent);
  if (!a) return 1;
  const backs = ["color-bg-base", "color-bg-raised"].map((k) => parseColour(tokens[k] ?? "")).filter((x): x is RGB => !!x);
  if (!backs.length) return 21;
  return Math.min(...backs.map((b) => contrastRatio(a, b)));
}

/** Why an accent can't be used on these tokens, or null when it can. */
export function accentProblem(accent: string, tokens: Record<string, string>): string | null {
  const c = accentContrast(accent, tokens);
  if (c >= ACCENT_MIN_CONTRAST) return null;
  return `That colour is too close to the background (${c.toFixed(1)}:1). Buttons and sliders need at least ${ACCENT_MIN_CONTRAST}:1 to stay visible, so pick a ${
    accentContrastNeedsDarker(tokens) ? "darker" : "lighter"
  } one.`;
}

function accentContrastNeedsDarker(tokens: Record<string, string>): boolean {
  const bg = parseColour(tokens["color-bg-raised"] ?? "");
  return !!bg && contrastRatio(bg, WHITE) < contrastRatio(bg, BLACK);
}

/** The accent tokens that follow from one accent colour. */
export function accentTokens(accent: string, base: ThemeBase, tokens: Record<string, string>): Record<string, string> {
  const a = parseColour(accent) ?? [236, 228, 211];
  const bg = parseColour(tokens["color-bg-base"] ?? "") ?? (base === "light" ? WHITE : BLACK);
  const onAccent = contrastRatio(a, DARK_TEXT) >= contrastRatio(a, LIGHT_TEXT) ? DARK_TEXT : LIGHT_TEXT;
  const hover = base === "light" ? mix(a, BLACK, 0.18) : mix(a, WHITE, 0.4);
  const pressed = base === "light" ? mix(a, BLACK, 0.3) : mix(a, BLACK, 0.1);
  return {
    "color-accent": toHex(a),
    "color-accent-hover": toHex(hover),
    "color-accent-pressed": toHex(pressed),
    "color-focus-ring": toHex(hover),
    "color-accent-strong": toHex(mix(a, bg, 0.6)),
    "color-text-on-accent": toHex(onAccent),
  };
}
