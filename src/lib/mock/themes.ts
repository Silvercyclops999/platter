/**
 * Mock themes folder: a few invented theme files, checked with the same
 * rules as the core. Import stands in for the file picker and adds one more.
 */
import { validateThemeText } from "../../design/themes/validate";
import { ACCENT_IDS, isCustomAccent } from "../../design/themes/accent";
import type { Settings, ThemeFile, ThemeFolder } from "../types";

export const MOCK_THEMES_FOLDER = "C:\\Users\\listener\\AppData\\Roaming\\Platter\\themes";

const FILES: Record<string, string> = {
  "Harbour.json": JSON.stringify({
    name: "Harbour",
    base: "dark",
    tokens: {
      "color-bg-base": "#08121A",
      "color-bg-raised": "#0E1A24",
      "color-bg-raised2": "#14232F",
      "color-bg-overlay": "#1B2D3B",
      "color-accent": "#8FD3E8",
      "radius-card": "12px",
    },
  }),
  "Morning paper.json": JSON.stringify({
    name: "Morning paper",
    base: "light",
    author: "A listener",
    tokens: { "color-bg-raised": "#FBFAF7", "color-bg-base": "#E9E6E0" },
  }),
  "glow.json": JSON.stringify({ name: "Glow", base: "dark", tokens: { "color-bg-base": "url(https://example.com/x.png)" } }),
  "sneaky.json": JSON.stringify({ name: "Sneaky", base: "dark", tokens: { "color-accent": "red; } body { display: none" } }),
  "extra.json": JSON.stringify({ name: "Extra", base: "dark", tokens: { "font-family": "Comic Sans" } }),
  "notes.json": "{ this isn't json",
};

const IMPORTED = {
  file: "Orchard.json",
  text: JSON.stringify({ name: "Orchard", base: "light", tokens: { "color-bg-raised": "#F6F8F1", "color-accent": "#3E6A2E" } }),
};

export class MockThemes {
  private files: Record<string, string> = { ...FILES };

  list(): ThemeFolder {
    const themes: ThemeFile[] = [];
    const invalid: ThemeFolder["invalid"] = [];
    for (const [file, text] of Object.entries(this.files).sort(([a], [b]) => a.localeCompare(b))) {
      const r = validateThemeText(file, text);
      if (r.ok) themes.push(r.theme);
      else invalid.push({ file, reason: r.reason });
    }
    return { folder: MOCK_THEMES_FOLDER, themes, invalid };
  }

  import(): ThemeFile {
    const r = validateThemeText(IMPORTED.file, IMPORTED.text);
    if (!r.ok) throw { kind: "invalid", message: r.reason };
    this.files[IMPORTED.file] = IMPORTED.text;
    return r.theme;
  }
}

/** The core's checks on the theme settings. Throws an invalid error. */
export function checkThemePatch(patch: Partial<Settings>): void {
  if (patch.theme !== undefined && !["dark", "light", "system", "midnight", "forest", "sand"].includes(patch.theme))
    throw { kind: "invalid", message: "That isn't one of the themes." };
  if (patch.accent !== undefined && !ACCENT_IDS.includes(patch.accent as never) && !isCustomAccent(patch.accent))
    throw { kind: "invalid", message: "Accent colours are a preset or a colour like #A1B2C3." };
  if (patch.themeFile) {
    const r = validateThemeText(patch.themeFile.file, JSON.stringify({ name: patch.themeFile.name, base: patch.themeFile.base, tokens: patch.themeFile.tokens }));
    if (!r.ok) throw { kind: "invalid", message: r.reason };
  }
}
