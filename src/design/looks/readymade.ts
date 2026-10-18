/**
 * Ready-made looks: the built-in themes, plus every look file in ./data.
 * Adding a ready-made look is only adding a data file there; it is checked
 * with the same rules as any other look file (one that fails is skipped).
 */
import { BUILTIN_THEMES, baseTheme, isStudioLook, type BuiltinThemeId } from "../themes/builtins";
import { validateThemeText, type ThemeBase, type UserTheme } from "../themes/validate";
import { DEFAULT_PARTS, STUDIO_FONTS, type LookParts } from "./apply";

export interface ReadyLook {
  id: string;
  name: string;
  base: ThemeBase;
  /** Every theme token. */
  tokens: Record<string, string>;
  /** Style and fonts; the layout is the listener's own unless the look has one. */
  style: LookParts["style"];
  fonts: LookParts["fonts"];
  /** A built-in theme id, or the look file for a data look. */
  source: { kind: "builtin"; theme: BuiltinThemeId } | { kind: "file"; look: UserTheme };
}

/** Look files shipped with the app (text, checked before use). */
const DATA = import.meta.glob("./data/*.platter-look", { eager: true, query: "?raw", import: "default" }) as Record<string, string>;

/** Build ready-made looks from look file texts (by file name). */
export function dataLooks(files: Record<string, string>): ReadyLook[] {
  const out: ReadyLook[] = [];
  for (const [path, text] of Object.entries(files).sort(([a], [b]) => a.localeCompare(b))) {
    const file = path.split("/").pop() ?? path;
    const r = validateThemeText(file, text);
    if (!r.ok || r.theme.notes?.length) continue;
    const look = r.theme;
    out.push({
      id: `file:${file}`,
      name: look.name,
      base: look.base,
      tokens: { ...baseTheme(look.base).tokens, ...look.tokens },
      style: look.style ?? DEFAULT_PARTS.style,
      fonts: look.fonts ?? DEFAULT_PARTS.fonts,
      source: { kind: "file", look },
    });
  }
  return out;
}

export function readyLooks(files: Record<string, string> = DATA): ReadyLook[] {
  const builtins: ReadyLook[] = Object.values(BUILTIN_THEMES).map((t) => ({
    id: t.id,
    name: t.name,
    base: t.base,
    tokens: t.tokens,
    style: DEFAULT_PARTS.style,
    fonts: isStudioLook(t.id) ? STUDIO_FONTS : DEFAULT_PARTS.fonts,
    source: { kind: "builtin", theme: t.id },
  }));
  return [...builtins, ...dataLooks(files)];
}
