import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import css from "../design/tokens.css?raw";
import { cssVariableTable } from "../design/tokens";
import {
  ACCENT_PRESETS,
  BUILTIN_IDS,
  BUILTIN_THEMES,
  THEME_TOKENS,
  THEME_TOKEN_NAMES,
  accentProblem,
  accentTokens,
  applyCachedTheme,
  resolveTheme,
  validateThemeText,
} from "../design/themes";
import cases from "../design/themes/validation-cases.json";
import { backend } from "../lib/backend";
import { LIGHT_HEADER_TEXT, contrastRatio, headerColors, hslToRgb, lightHeader, parseColour, type RGB } from "../lib/color";
import type { MockBackend } from "../lib/mock";
import { DEFAULT_SETTINGS } from "../lib/types";
import { SettingsPage } from "../pages/SettingsPage";
import { loadSettings, updateSettings, useSettings } from "../stores/settings";
import { installTheme, useTheme } from "../stores/theme";
import { useToasts } from "../stores/toasts";

const mock = backend as MockBackend;
const root = () => document.documentElement;
const rgb = (v: string): RGB => {
  const c = parseColour(v);
  if (!c) throw new Error(`not a colour: ${v}`);
  return c;
};
const ratio = (a: string, b: string) => contrastRatio(rgb(a), rgb(b));

/** A matchMedia stand-in for prefers-color-scheme that tests can flip. */
function fakeScheme(dark: boolean) {
  const listeners = new Set<(e: MediaQueryListEvent) => void>();
  const mq = {
    matches: dark,
    media: "(prefers-color-scheme: dark)",
    addEventListener: (_: string, f: (e: MediaQueryListEvent) => void) => listeners.add(f),
    removeEventListener: (_: string, f: (e: MediaQueryListEvent) => void) => listeners.delete(f),
  };
  vi.spyOn(window, "matchMedia").mockImplementation(() => mq as unknown as MediaQueryList);
  return (next: boolean) => {
    mq.matches = next;
    for (const f of listeners) f({ matches: next } as MediaQueryListEvent);
  };
}

function resetDocument() {
  for (const n of THEME_TOKEN_NAMES) root().style.removeProperty(`--${n}`);
  delete root().dataset.theme;
  delete root().dataset.themeBase;
  localStorage.clear();
}

beforeEach(async () => {
  if (typeof window.matchMedia !== "function") window.matchMedia = (() => ({ matches: false, addEventListener() {}, removeEventListener() {} })) as never;
  await backend.settingsSet({ ...DEFAULT_SETTINGS });
  useSettings.setState({ settings: null });
  useToasts.setState({ toasts: [] });
  resetDocument();
  await loadSettings();
});
afterEach(() => {
  vi.restoreAllMocks();
  resetDocument();
});

describe("built-in themes", () => {
  it("has five, each setting every theme token", () => {
    expect(BUILTIN_IDS).toEqual(["dark", "light", "midnight", "forest", "sand"]);
    for (const id of BUILTIN_IDS) {
      const t = BUILTIN_THEMES[id].tokens;
      expect(Object.keys(t).sort(), id).toEqual([...THEME_TOKEN_NAMES].sort());
      for (const [name, value] of Object.entries(t)) {
        const r = validateThemeText("x.json", JSON.stringify({ name: "x", base: "dark", tokens: { [name]: value } }));
        expect(r.ok, `${id} ${name} ${value}`).toBe(true);
      }
    }
  });

  it("Dark is exactly the tokens in tokens.css, and every theme token is declared there", () => {
    const table = cssVariableTable();
    for (const [name, value] of Object.entries(BUILTIN_THEMES.dark.tokens)) expect(table[name]?.toLowerCase(), name).toBe(value.toLowerCase());
    for (const name of THEME_TOKEN_NAMES) expect(css, name).toMatch(new RegExp(`--${name}:`));
    expect(Object.values(THEME_TOKENS).every((k) => k === "colour" || k === "length")).toBe(true);
  });

  it("views over artwork take the Dark theme's own values on light themes", () => {
    const start = css.indexOf(':root[data-theme-base="light"] :is(');
    const block = css.slice(start, css.indexOf("}", start));
    const pinned = [...block.matchAll(/--([a-z0-9-]+):\s*([^;]+);/g)];
    expect(pinned.length).toBeGreaterThan(10);
    for (const [, name, value] of pinned) expect(value.toLowerCase(), name).toBe(BUILTIN_THEMES.dark.tokens[name]?.toLowerCase());
  });

  it.each(BUILTIN_IDS)("%s meets AA text contrast on every surface", (id) => {
    const t = BUILTIN_THEMES[id].tokens;
    const surfaces = ["color-bg-base", "color-bg-raised", "color-bg-raised2", "color-bg-overlay"];
    for (const s of surfaces) {
      expect(ratio(t["color-text-primary"], t[s]), `primary on ${s}`).toBeGreaterThanOrEqual(7);
      expect(ratio(t["color-text-secondary"], t[s]), `secondary on ${s}`).toBeGreaterThanOrEqual(4.5);
      expect(ratio(t["color-text-muted"], t[s]), `muted on ${s}`).toBeGreaterThanOrEqual(3.8);
    }
    for (const s of ["color-bg-base", "color-bg-raised"]) {
      expect(ratio(t["color-text-muted"], t[s]), `muted on ${s}`).toBeGreaterThanOrEqual(4.5);
      expect(ratio(t["color-accent"], t[s]), `accent on ${s}`).toBeGreaterThanOrEqual(3);
      expect(ratio(t["color-focus-ring"], t[s]), `focus ring on ${s}`).toBeGreaterThanOrEqual(3);
      expect(ratio(t["color-playing"], t[s]), `playing on ${s}`).toBeGreaterThanOrEqual(4.5);
      expect(ratio(t["color-error"], t[s]), `error on ${s}`).toBeGreaterThanOrEqual(4.5);
      expect(ratio(t["color-version"], t[s]), `version on ${s}`).toBeGreaterThanOrEqual(4.5);
    }
    expect(ratio(t["color-text-on-accent"], t["color-accent"]), "text on accent").toBeGreaterThanOrEqual(4.5);
    expect(ratio(t["color-text-on-accent"], t["color-accent-hover"]), "text on accent hover").toBeGreaterThanOrEqual(4.5);
    expect(ratio(t["color-error-text"], t["color-error-surface"]), "error toast").toBeGreaterThanOrEqual(4.5);
  });
});

describe("accent colours", () => {
  it("has ivory and six presets that work on every built-in theme", () => {
    expect(ACCENT_PRESETS.map((p) => p.id)).toEqual(["ivory", "sage", "sky", "lilac", "amber", "rose", "teal"]);
    for (const id of BUILTIN_IDS) {
      const theme = BUILTIN_THEMES[id];
      for (const p of ACCENT_PRESETS) {
        const colour = p[theme.base];
        expect(accentProblem(colour, theme.tokens), `${p.id} on ${id}`).toBeNull();
        const a = accentTokens(colour, theme.base, theme.tokens);
        expect(ratio(a["color-text-on-accent"], a["color-accent"]), `${p.id} text on ${id}`).toBeGreaterThanOrEqual(4.5);
      }
    }
  });

  it("refuses a colour under 3:1 against the background and says why", () => {
    const dark = BUILTIN_THEMES.dark.tokens;
    expect(accentProblem("#2A2622", dark)).toMatch(/too close to the background \(1\.\d:1\).*at least 3:1.*lighter/);
    expect(accentProblem("#F2F0EA", BUILTIN_THEMES.light.tokens)).toMatch(/darker/);
    expect(accentProblem("#ECE4D3", dark)).toBeNull();
  });

  it("uses the theme's own accent when a saved custom colour doesn't fit the theme", () => {
    const r = resolveTheme({ theme: "light", accent: "#F4F0E8", themeFile: null }, true);
    expect(r.tokens["color-accent"]).toBe(BUILTIN_THEMES.light.tokens["color-accent"]);
    expect(r.accentNote).toMatch(/too close/);
    const ok = resolveTheme({ theme: "dark", accent: "#7FD1C6", themeFile: null }, true);
    expect(ok.tokens["color-accent"]).toBe("#7FD1C6");
    expect(ok.accentNote).toBeNull();
  });
});

describe("theme files", () => {
  it.each(cases)("$case", ({ text, ok, reason }) => {
    const r = validateThemeText("t.json", text);
    expect(r.ok).toBe(ok);
    if (!r.ok) expect(r.reason).toContain(reason);
  });

  it("start from their base and take only checked values", () => {
    const r = resolveTheme(
      {
        theme: "dark",
        accent: "ivory",
        themeFile: { file: "a.json", name: "A", base: "light", tokens: { "color-bg-raised": "#FAFAFA", "color-accent": "url(x)", bogus: "#000" } },
      },
      true,
    );
    expect(r.base).toBe("light");
    expect(r.id).toBe("file");
    expect(r.tokens["color-bg-raised"]).toBe("#FAFAFA");
    expect(r.tokens["color-accent"]).toBe(BUILTIN_THEMES.light.tokens["color-accent"]);
    expect(r.tokens).not.toHaveProperty("bogus");
  });
});

describe("applying themes", () => {
  it("Dark sets nothing inline; Light sets its tokens and the light base", async () => {
    fakeScheme(true);
    const un = installTheme();
    expect(root().dataset.theme).toBe("dark");
    expect(root().dataset.themeBase).toBe("dark");
    expect(root().getAttribute("style") ?? "").not.toContain("--color");
    await act(() => updateSettings({ theme: "light" }));
    expect(root().dataset.themeBase).toBe("light");
    expect(root().style.getPropertyValue("--color-bg-raised")).toBe(BUILTIN_THEMES.light.tokens["color-bg-raised"]);
    await act(() => updateSettings({ theme: "dark" }));
    expect(root().style.getPropertyValue("--color-bg-raised")).toBe("");
    un();
  });

  it("Match system follows the computer's light or dark mode as it changes", async () => {
    const flip = fakeScheme(false);
    const un = installTheme();
    await act(() => updateSettings({ theme: "system" }));
    expect(root().dataset.theme).toBe("light");
    act(() => flip(true));
    expect(root().dataset.theme).toBe("dark");
    expect(useTheme.getState().resolved.base).toBe("dark");
    act(() => flip(false));
    expect(root().dataset.themeBase).toBe("light");
    // A fixed theme ignores the computer.
    await act(() => updateSettings({ theme: "midnight" }));
    act(() => flip(true));
    expect(root().dataset.theme).toBe("midnight");
    un();
  });

  it("keeps the choice: saved in settings, and cached for the next start before settings load", async () => {
    fakeScheme(true);
    const un = installTheme();
    await act(() => updateSettings({ theme: "sand", accent: "teal" }));
    un();
    const saved = await backend.settingsGet();
    expect([saved.theme, saved.accent]).toEqual(["sand", "teal"]);
    const accent = root().style.getPropertyValue("--color-accent");
    expect(accent).toBe(ACCENT_PRESETS.find((p) => p.id === "teal")!.light);
    for (const n of THEME_TOKEN_NAMES) root().style.removeProperty(`--${n}`);
    delete root().dataset.theme;
    applyCachedTheme();
    expect(root().dataset.theme).toBe("sand");
    expect(root().style.getPropertyValue("--color-accent")).toBe(accent);
  });

  it("ignores a tampered cache", () => {
    localStorage.setItem("platter.theme", JSON.stringify({ id: "x", base: "light", tokens: { "color-bg-base": "url(evil)", "font-family": "x" } }));
    applyCachedTheme();
    expect(root().style.getPropertyValue("--color-bg-base")).toBe("");
    expect(root().dataset.theme).toBe("dark");
  });
});

describe("artwork colours on light themes", () => {
  it("pale header colours keep dark header text readable for every hue", () => {
    for (let h = 0; h < 360; h += 20) {
      for (const l of [0.2, 0.5, 0.8]) {
        const { vivid, shade } = lightHeader(headerColors(hslToRgb(h, 1, l)));
        expect(contrastRatio(LIGHT_HEADER_TEXT, rgb(vivid)), `vivid h${h} l${l}`).toBeGreaterThanOrEqual(4.5);
        expect(contrastRatio(LIGHT_HEADER_TEXT, rgb(shade)), `shade h${h} l${l}`).toBeGreaterThanOrEqual(7);
      }
    }
    const grey = lightHeader(headerColors(null));
    expect(contrastRatio(LIGHT_HEADER_TEXT, rgb(grey.shade))).toBeGreaterThanOrEqual(7);
  });
});

describe("theme settings", () => {
  const row = (title: string) => {
    const el = screen.getByText(title, { selector: ".setting-row__title" }).closest(".setting-row");
    if (!el) throw new Error(`no row ${title}`);
    return within(el as HTMLElement);
  };

  it("shows Theme, Accent colour and Theme file in Appearance with their defaults", async () => {
    render(<SettingsPage />);
    const theme = screen.getByRole("combobox", { name: "Theme" });
    expect(theme).toHaveValue("dark");
    expect(within(theme).getAllByRole("option").map((o) => o.textContent)).toEqual(["Dark", "Light", "Match system", "Midnight", "Forest", "Sand"]);
    expect(row("Accent colour").getByRole("radio", { name: "Ivory" })).toHaveAttribute("aria-checked", "true");
    expect(row("Accent colour").getAllByRole("radio")).toHaveLength(7);
    expect(row("Accent colour").getByLabelText("Custom colour")).toBeInTheDocument();
    await waitFor(() => expect(screen.getByRole("combobox", { name: "Your looks and theme files" })).toHaveValue(""));
    // The rows sit in Appearance.
    const appearance = screen.getByRole("heading", { level: 2, name: "Appearance" }).closest("section") ?? document.body;
    expect(within(appearance as HTMLElement).getByText("Your looks and theme files")).toBeInTheDocument();
  });

  it("changes the theme and the accent", async () => {
    const user = userEvent.setup();
    render(<SettingsPage />);
    await user.selectOptions(screen.getByRole("combobox", { name: "Theme" }), "light");
    expect(useSettings.getState().settings?.theme).toBe("light");
    await user.click(row("Accent colour").getByRole("radio", { name: "Sky" }));
    expect(useSettings.getState().settings?.accent).toBe("sky");
    expect((await backend.settingsGet()).accent).toBe("sky");
  });

  it("refuses a custom colour too close to the background, with the reason", async () => {
    fakeScheme(true);
    const un = installTheme();
    render(<SettingsPage />);
    const input = row("Accent colour").getByLabelText("Custom colour");
    fireEvent.change(input, { target: { value: "#2a2622" } });
    expect(await row("Accent colour").findByRole("alert")).toHaveTextContent(/too close to the background.*3:1/);
    expect(useSettings.getState().settings?.accent).toBe("ivory");
    fireEvent.change(input, { target: { value: "#7fd1c6" } });
    await waitFor(() => expect(useSettings.getState().settings?.accent).toBe("#7FD1C6"));
    expect(row("Accent colour").queryByRole("alert")).toBeNull();
    expect(root().style.getPropertyValue("--color-accent")).toBe("#7FD1C6");
    un();
  });

  it("lists theme files, explains the ones that can't be used, and applies one", async () => {
    const user = userEvent.setup();
    fakeScheme(true);
    const un = installTheme();
    render(<SettingsPage />);
    const pick = await screen.findByRole("combobox", { name: "Your looks and theme files" });
    await waitFor(() => expect(within(pick).getAllByRole("option").map((o) => o.textContent)).toEqual(["None", "Harbour", "Morning paper", "Night shift"]));
    const problems = row("Your looks and theme files");
    expect(problems.getByText("glow.json").nextSibling).toHaveTextContent("url()");
    expect(problems.getByText("sneaky.json").nextSibling).toHaveTextContent("CSS");
    expect(problems.getByText("extra.json").nextSibling).toHaveTextContent('Unknown token "font-family"');
    expect(problems.getByText("notes.json").nextSibling).toHaveTextContent("isn't valid JSON");

    await user.selectOptions(pick, "Harbour.json");
    expect(useSettings.getState().settings?.themeFile?.name).toBe("Harbour");
    expect(root().style.getPropertyValue("--color-bg-base")).toBe("#08121A");
    expect(root().style.getPropertyValue("--radius-card")).toBe("12px");
    // The Theme row says the file wins.
    expect(screen.getByRole("combobox", { name: "Theme" })).toBeDisabled();
    expect(row("Theme").getByText(/Harbour is in use/)).toBeInTheDocument();
    expect(row("Accent colour").getByText(/sets its own accent/)).toBeInTheDocument();

    await user.selectOptions(pick, "");
    expect(useSettings.getState().settings?.themeFile).toBeNull();
    expect(root().style.getPropertyValue("--color-bg-base")).toBe("");
    un();
  });

  it("imports a theme file and uses it", async () => {
    const user = userEvent.setup();
    render(<SettingsPage />);
    await screen.findByRole("combobox", { name: "Your looks and theme files" });
    await user.click(row("Your looks and theme files").getByRole("button", { name: "Import" }));
    await waitFor(() => expect(useSettings.getState().settings?.themeFile?.name).toBe("Orchard"));
    expect(within(screen.getByRole("combobox", { name: "Your looks and theme files" })).getByRole("option", { name: "Orchard" })).toBeInTheDocument();
    const toasts = useToasts.getState().toasts;
    expect(toasts[toasts.length - 1]?.message).toMatch(/Added Orchard/);
    expect(row("Your looks and theme files").getByRole("button", { name: "Open themes folder" })).toBeEnabled();
  });

  it("the core refuses a bad theme file or accent sent to it", async () => {
    await expect(backend.settingsSet({ accent: "neon" })).rejects.toMatchObject({ kind: "invalid" });
    await expect(
      backend.settingsSet({ themeFile: { file: "x.json", name: "X", base: "dark", tokens: { "color-bg-base": "url(x)" } } }),
    ).rejects.toMatchObject({ kind: "invalid" });
    expect(mock).toBeTruthy();
  });
});
