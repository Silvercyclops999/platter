/**
 * Mock themes folder: a few invented theme and look files, checked with the
 * same rules as the core. Import stands in for the file picker and adds one
 * more; export stands in for the save dialog and keeps the text it wrote.
 */
import { validateThemeText, checkHeldTheme, layoutProblem, isLook, lookText, LOOK_EXT } from "../../design/themes/validate";
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
  "Night shift.platter-look": JSON.stringify({
    version: 1,
    name: "Night shift",
    author: "A listener",
    base: "dark",
    tokens: { "color-bg-base": "#0B0B12", "color-bg-raised": "#13131D", "color-accent": "#B9A6F5", "radius-card": "14px" },
    style: { density: "comfortable", shadow: "subtle", blur: "low", border: 1 },
    fonts: { text: "manrope", headings: "manrope" },
    layout: { sidebarSide: "right", playerStyle: "largeArt", coverShape: "rounded", quickAccess: 6 },
    sparkle: true,
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

const invalid = (message: string) => ({ kind: "invalid", message });

function stem(name: string): string {
  const s = name
    .replace(/[^\p{L}\p{N} ._-]/gu, "_")
    .replace(/^[. ]+|[. ]+$/g, "");
  return s || "Theme";
}

export class MockThemes {
  private files: Record<string, string> = { ...FILES };
  /** The last look written by export (tests read it). */
  lastExport: { file: string; text: string } | null = null;

  list(): ThemeFolder {
    const themes: ThemeFile[] = [];
    const invalidFiles: ThemeFolder["invalid"] = [];
    for (const [file, text] of Object.entries(this.files).sort(([a], [b]) => a.localeCompare(b))) {
      const r = validateThemeText(file, text);
      if (r.ok) themes.push(r.theme);
      else invalidFiles.push({ file, reason: r.reason });
    }
    return { folder: MOCK_THEMES_FOLDER, themes, invalid: invalidFiles };
  }

  private freeName(name: string, ext: string): string {
    const s = stem(name);
    for (let n = 1; ; n++) {
      const f = n === 1 ? `${s}.${ext}` : `${s} (${n}).${ext}`;
      if (!(f in this.files)) return f;
    }
  }

  import(): ThemeFile {
    const r = validateThemeText(IMPORTED.file, IMPORTED.text);
    if (!r.ok) throw invalid(r.reason);
    this.files[IMPORTED.file] = IMPORTED.text;
    return r.theme;
  }

  importText(text: string): ThemeFile {
    const r = validateThemeText("", text);
    if (!r.ok) throw invalid(r.reason);
    const file = this.freeName(r.theme.name, isLook(r.theme) ? LOOK_EXT : "json");
    this.files[file] = text;
    return { ...r.theme, file };
  }

  save(look: ThemeFile, replace: boolean): ThemeFile {
    const held: ThemeFile = { ...look, notes: undefined, style: look.style ?? { density: "comfortable", shadow: "flat", blur: "off", border: 0 } };
    delete held.notes;
    const problem = checkHeldTheme(held);
    if (problem) throw invalid(problem);
    const own = replace && /^[^./\\:][^/\\:]*\.platter-look$/i.test(held.file) && held.file in this.files;
    const file = own ? held.file : this.freeName(held.name, LOOK_EXT);
    this.files[file] = lookText(held);
    return { ...held, file };
  }

  export(look: ThemeFile): string {
    const problem = checkHeldTheme({ ...look, notes: undefined });
    if (problem) throw invalid(problem);
    const file = `${stem(look.name)}.${LOOK_EXT}`;
    this.lastExport = { file, text: lookText(look) };
    return file;
  }
}

/** The core's checks on the theme settings. Throws an invalid error. */
export function checkThemePatch(patch: Partial<Settings>): void {
  if (patch.theme !== undefined && !["dark", "light", "system", "midnight", "forest", "sand"].includes(patch.theme))
    throw invalid("That isn't one of the themes.");
  if (patch.accent !== undefined && !ACCENT_IDS.includes(patch.accent as never) && !isCustomAccent(patch.accent))
    throw invalid("Accent colours are a preset or a colour like #A1B2C3.");
  if (patch.themeFile) {
    const problem = checkHeldTheme(patch.themeFile);
    if (problem) throw invalid(problem);
  }
  if (patch.layout !== undefined) {
    const problem = layoutProblem(patch.layout);
    if (problem) throw invalid(problem);
  }
}
