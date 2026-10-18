import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterAll, afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { applyCachedLook, applyLook, DEFAULT_PARTS, lookAttributes } from "../design/looks/apply";
import { CONTRAST_RULES, contrastReport, fixColour } from "../design/looks/contrast";
import { FONTS } from "../design/looks/fonts";
import { paletteFrom, paletteFromSeed } from "../design/looks/palette";
import { dataLooks, readyLooks } from "../design/looks/readymade";
import {
  BUILTIN_THEMES,
  CLASSIC_LAYOUT,
  LOOK,
  checkHeldTheme,
  layoutProblem,
  lookText,
  validateThemeText,
  type Layout,
} from "../design/themes";
import contract from "../design/themes/contract.json";
import lookCases from "../design/themes/look-validation-cases.json";
import oldCases from "../design/themes/validation-cases.json";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import type { HomeData, ThemeFile } from "../lib/types";
import { DEFAULT_SETTINGS } from "../lib/types";
import { HomePage } from "../pages/HomePage";
import { LookBuilder } from "../pages/looks/LookBuilder";
import { LookCards, PickLookDialog } from "../pages/looks/LookCards";
import { LookPreview } from "../pages/looks/LookPreview";
import { NOTICES, SettingsPage } from "../pages/SettingsPage";
import { addDroppedFile } from "../shell/LookDrop";
import { PlayerBar } from "../shell/PlayerBar";
import { looks, offerLookStep, useLooks } from "../stores/looks";
import { connectPlayer, usePlayer } from "../stores/player";
import { useRouter } from "../stores/router";
import { loadSettings, updateSettings, useSettings } from "../stores/settings";
import { installTheme, motionPreference } from "../stores/theme";
import { useToasts } from "../stores/toasts";
import appShell from "../shell/AppShell.tsx?raw";
import lazyHosts from "../shell/lazyHosts.tsx?raw";
import settingsPage from "../pages/SettingsPage.tsx?raw";
import pkg from "../../package.json";

const mock = backend as MockBackend;
const root = () => document.documentElement;
const layout = (over: Partial<Layout>): Layout => ({ ...structuredClone(CLASSIC_LAYOUT), ...over });
const lastToast = () => { const t = useToasts.getState().toasts; return t[t.length - 1]; };

let disconnect: () => void;
beforeAll(async () => {
  disconnect = connectPlayer(mock);
  await waitFor(() => expect(usePlayer.getState().state?.track).toBeTruthy());
});
afterAll(() => disconnect());

beforeEach(async () => {
  if (typeof window.matchMedia !== "function") window.matchMedia = (() => ({ matches: false, addEventListener() {}, removeEventListener() {} })) as never;
  // The builder starts from the look in use; these start from Classic dark.
  await backend.settingsSet({ ...DEFAULT_SETTINGS, theme: "dark", accent: "ivory", followSystem: false });
  useSettings.setState({ settings: null });
  useToasts.setState({ toasts: [] });
  useLooks.setState({ builder: null, pick: false, folderVersion: 0 });
  localStorage.clear();
  await loadSettings();
});
afterEach(() => {
  vi.restoreAllMocks();
  for (const k of Object.keys(root().dataset)) if (k.startsWith("look")) delete root().dataset[k];
});

describe("look files", () => {
  it("pass the shared cases: good, unknown keys noted, CSS, url() and script refused, wrong version", () => {
    for (const c of lookCases as { case: string; text: string; ok: boolean; reason?: string; notes?: number }[]) {
      const r = validateThemeText("x.platter-look", c.text);
      expect(r.ok, c.case).toBe(c.ok);
      if (r.ok && c.notes !== undefined) expect(r.theme.notes?.length ?? 0, c.case).toBe(c.notes);
      if (!r.ok) expect(r.reason, c.case).toContain(c.reason);
    }
  });

  it("refuse an oversized file", () => {
    const big = JSON.stringify({ version: 1, name: "Big", base: "dark", padding: "x".repeat(70 * 1024) });
    const r = validateThemeText("big.platter-look", big);
    expect(r.ok).toBe(false);
    expect(!r.ok && r.reason).toContain("too big");
  });

  it("old theme files still load exactly as before", () => {
    for (const c of oldCases as { case: string; text: string; ok: boolean; reason?: string }[]) {
      const r = validateThemeText("t.json", c.text);
      expect(r.ok, c.case).toBe(c.ok);
      if (!r.ok) expect(r.reason, c.case).toContain(c.reason);
    }
  });

  it("export writes only the look: no file path, notes or anything else", async () => {
    const look: ThemeFile = {
      file: "C:\\Users\\someone\\AppData\\themes\\Dusk.platter-look",
      name: "Dusk",
      base: "dark",
      tokens: { "color-accent": "#AABBCC" },
      style: { density: "compact", shadow: "soft", blur: "low", border: 1 },
      fonts: { text: "manrope", headings: "fraunces" },
      layout: layout({ sidebarSide: "right" }),
      notes: ["Left out x"],
    };
    const text = lookText(look);
    expect(Object.keys(JSON.parse(text)).sort()).toEqual(["base", "fonts", "layout", "name", "style", "tokens", "version"]);
    expect(text).not.toContain("someone");
    expect(text).not.toContain("Left out");
    await mock.themesExport(look);
    expect(mock.themes.lastExport?.file).toBe("Dusk.platter-look");
    expect(mock.themes.lastExport?.text).toBe(text);
    // What was exported reads back the same.
    const back = validateThemeText("Dusk.platter-look", text);
    expect(back.ok && back.theme.layout?.sidebarSide).toBe("right");
  });

  it("a look held in settings must have nothing left out", () => {
    expect(checkHeldTheme({ file: "a.platter-look", name: "A", base: "dark", tokens: { "font-family": "x" }, style: DEFAULT_PARTS.style })).toContain("font-family");
    expect(layoutProblem(layout({}))).toBeNull();
    expect(layoutProblem({ ...layout({}), quickAccess: 5 })).toContain("4, 6 or 8");
    expect(layoutProblem({ ...layout({}), rocket: true })).toContain("rocket");
  });

  it("the contract lists every layout option the app draws, and fonts are bundled only", () => {
    expect(Object.keys(contract.look.layout).sort()).toEqual(
      ["coverShape", "headerStyle", "homeShelves", "motion", "nowPlaying", "playerButtons", "playerStyle", "quickAccess", "sidebarSections", "sidebarSide", "trackColumns"].sort(),
    );
    expect(FONTS.map((f) => f.id)).toEqual(LOOK.fonts);
    expect(FONTS.every((f) => !/url|http/i.test(f.family))).toBe(true);
  });
});

describe("contrast", () => {
  it("fix-it moves a failing colour to the nearest one that passes, keeping its hue", () => {
    const tokens = { ...BUILTIN_THEMES.dark.tokens, "color-text-muted": "#3A3530" };
    const rule = CONTRAST_RULES.find((r) => r.token === "color-text-muted")!;
    expect(contrastReport(tokens).find((r) => r.rule.token === "color-text-muted")?.passes).toBe(false);
    const fixed = fixColour("#3A3530", rule.against, tokens, rule.min)!;
    expect(fixed).toMatch(/^#[0-9A-F]{6}$/);
    const after = { ...tokens, "color-text-muted": fixed };
    const res = contrastReport(after).find((r) => r.rule.token === "color-text-muted")!;
    expect(res.passes).toBe(true);
    // Only just over the line: the nearest passing lightness, not white.
    expect(res.ratio).toBeLessThan(5);
    expect(fixColour("#ECE4D3", rule.against, BUILTIN_THEMES.dark.tokens, rule.min)).toBe("#ECE4D3");
  });

  it("every generated palette passes every rule, dark and light, and a seed gives the same palette", () => {
    for (const base of ["dark", "light"] as const) {
      for (const seed of [1, 42, 9001, 123456]) {
        const t = paletteFromSeed(seed, base);
        for (const r of contrastReport(t)) expect(r.passes, `${base} ${seed} ${r.rule.token} ${r.ratio}`).toBe(true);
        expect(validateThemeText("p.platter-look", JSON.stringify({ version: 1, name: "P", base, tokens: t })).ok).toBe(true);
      }
      expect(paletteFromSeed(7, base)).toEqual(paletteFromSeed(7, base));
      for (const r of contrastReport(paletteFrom("#C0392B", base))) expect(r.passes).toBe(true);
    }
  });

  it("every built-in theme passes", () => {
    for (const t of Object.values(BUILTIN_THEMES)) for (const r of contrastReport(t.tokens)) expect(r.passes, `${t.id} ${r.rule.token}`).toBe(true);
  });
});

describe("layout in the app", () => {
  it("puts style and layout on the document; Classic sets the defaults", () => {
    applyLook(root(), DEFAULT_PARTS);
    expect(root().dataset.lookSidebarSide).toBe("left");
    expect(root().dataset.lookCols).toBe("classic");
    expect(root().dataset.lookColsHide).toBeUndefined();
    const parts = { ...DEFAULT_PARTS, layout: layout({ sidebarSide: "right", trackColumns: ["duration", "album"], coverShape: "circle", motion: "off" }) };
    applyLook(root(), parts);
    expect(root().dataset).toMatchObject({ lookSidebarSide: "right", lookCols: "custom", lookColsHide: "info", lookCoverShape: "circle", lookMotion: "off" });
    expect(root().style.getPropertyValue("--col-duration")).toBe("2");
    expect(root().style.getPropertyValue("--col-album")).toBe("3");
    // Every option in the contract becomes an attribute.
    const { data } = lookAttributes(parts);
    for (const k of ["sidebarSide", "nowPlaying", "playerStyle", "headerStyle", "coverShape", "motion", "density", "shadow", "blur"]) expect(data[k], k).toBeTruthy();
  });

  it("follows the settings and comes back from the cache before settings load", async () => {
    const un = installTheme();
    await act(() => updateSettings({ layout: layout({ playerStyle: "compact", headerStyle: "minimal" }) }));
    expect(root().dataset.lookPlayerStyle).toBe("compact");
    expect(root().dataset.lookHeaderStyle).toBe("minimal");
    un();
    for (const k of Object.keys(root().dataset)) if (k.startsWith("look")) delete root().dataset[k];
    applyCachedLook();
    expect(root().dataset.lookPlayerStyle).toBe("compact");
    // A tampered cache is ignored.
    localStorage.setItem("platter.look", JSON.stringify({ layout: { playerStyle: "<script>" } }));
    delete root().dataset.lookPlayerStyle;
    applyCachedLook();
    expect(root().dataset.lookPlayerStyle).toBeUndefined();
  });

  it("player bar buttons show in the layout's order, and hidden ones go", async () => {
    await act(() => updateSettings({ layout: layout({ playerButtons: ["volume", "queue", "lyrics"] }) }));
    render(<PlayerBar />);
    const extras = document.querySelector(".player-bar__extras") as HTMLElement;
    const names = within(extras)
      .getAllByRole("button")
      .map((b) => b.getAttribute("aria-label"))
      .filter((n) => n && /mute|queue|lyrics|device|miniplayer|full/i.test(n));
    expect(names[0]).toMatch(/mute/i);
    expect(names.findIndex((n) => /queue/i.test(n!))).toBeLessThan(names.findIndex((n) => /lyrics/i.test(n!)));
    expect(within(extras).queryByRole("button", { name: "Connect to a device" })).toBeNull();
    expect(within(extras).queryByRole("button", { name: /miniplayer/i })).toBeNull();
  });

  it("Home shows the layout's shelves in order, the greeting and the grid size", async () => {
    const d: HomeData = {
      shortcuts: Array.from({ length: 8 }, (_, i) => ({ kind: "playlist" as const, playlist: { id: `pl${i}`, title: `Playlist ${i}`, smart: false, trackCount: 1, durationMs: 1, thumb: null } })),
      recentlyPlayed: [{ kind: "playlist", playlist: { id: "pl1", title: "Playlist 1", smart: false, trackCount: 1, durationMs: 1, thumb: null } }],
      jumpBackIn: [],
      playlists: [{ id: "pl3", title: "Playlist 3", smart: false, trackCount: 1, durationMs: 1, thumb: null }],
      recentlyAdded: [],
      topArtists: [{ id: "ar1", title: "Artist 1", sortTitle: "Artist 1", thumb: null, art: null, viewCount: 1 }],
    };
    vi.spyOn(backend, "home").mockResolvedValue(d);
    useRouter.getState().reset();
    Element.prototype.scrollTo = vi.fn();
    await act(() => updateSettings({ homeMixes: false, layout: layout({ homeShelves: ["topArtists", "playlists"], homeGreeting: true, quickAccess: 4 }) }));
    render(<HomePage />);
    await waitFor(() => expect(screen.getAllByRole("heading", { level: 2 }).map((h) => h.textContent)).toEqual(["Your top artists", "Your playlists"]));
    expect(screen.getByRole("heading", { level: 1 })).toHaveClass("home__greeting");
    expect(within(screen.getByRole("list", { name: "Shortcuts" })).getAllByRole("listitem")).toHaveLength(4);
  });

  it("the Motion option reduces motion; Full leaves the Reduce motion setting in charge", () => {
    const s = (motion: Layout["motion"], reduceMotion: "system" | "on" | "off") => ({ settings: { ...DEFAULT_SETTINGS, reduceMotion, layout: layout({ motion }) } });
    expect(motionPreference(s("reduced", "off"))).toBe("on");
    expect(motionPreference(s("off", "system"))).toBe("on");
    expect(motionPreference(s("full", "system"))).toBe("system");
    expect(motionPreference({ settings: null })).toBe("system");
  });
});

describe("preview", () => {
  it("draws the look with made-up music and rearranges by drag", () => {
    const onReorder = vi.fn();
    render(<LookPreview base="dark" tokens={BUILTIN_THEMES.dark.tokens} parts={{ ...DEFAULT_PARTS, layout: layout({ playerButtons: ["queue", "volume", "lyrics"] }) }} onReorder={onReorder} />);
    const pv = screen.getByRole("group", { name: "Preview" });
    expect(pv).toHaveAttribute("data-look-sidebar-side", "left");
    expect(pv.style.getPropertyValue("--color-bg-base")).toBe(BUILTIN_THEMES.dark.tokens["color-bg-base"]);
    const lyrics = pv.querySelector('[data-drag-id="lyrics"]')!;
    const queue = pv.querySelector('[data-drag-id="queue"]')!;
    fireEvent.dragStart(lyrics, { dataTransfer: { setData() {}, effectAllowed: "" } });
    fireEvent.dragOver(queue, { dataTransfer: { dropEffect: "" } });
    fireEvent.drop(queue, { dataTransfer: {} });
    expect(onReorder).toHaveBeenCalledWith("playerButtons", "lyrics", "queue");
    // No headings or landmarks: it's a picture.
    expect(pv.querySelector("h1, h2, main, aside, nav, header, footer")).toBeNull();
  });
});

describe("look builder", () => {
  const renderBuilder = (tab: "colours" | "shape" | "layout" = "colours") => {
    const onTab = vi.fn();
    const r = render(<LookBuilder tab={tab} onTab={onTab} onClose={() => {}} />);
    return { ...r, onTab };
  };

  it("edits a colour live, shows its contrast, fixes it, and undoes and redoes", async () => {
    const user = userEvent.setup();
    renderBuilder();
    const muted = screen.getByRole("textbox", { name: "Quiet text" });
    fireEvent.change(muted, { target: { value: "#2A2622" } });
    const row = muted.closest(".lb-colour") as HTMLElement;
    expect(within(row).getByText(/Too low/)).toBeInTheDocument();
    const pv = screen.getByRole("group", { name: "Preview of your look" });
    expect(pv.style.getPropertyValue("--color-text-muted")).toBe("#2A2622");
    await user.click(within(row).getByRole("button", { name: "Fix" }));
    expect(within(row).getByText(/AA/)).toBeInTheDocument();
    const fixed = (screen.getByRole("textbox", { name: "Quiet text" }) as HTMLInputElement).value;
    expect(fixed).not.toBe("#2A2622");
    await user.click(screen.getByRole("button", { name: "Undo" }));
    expect(screen.getByRole("textbox", { name: "Quiet text" })).toHaveValue("#2A2622");
    await user.click(screen.getByRole("button", { name: "Redo" }));
    expect(screen.getByRole("textbox", { name: "Quiet text" })).toHaveValue(fixed);
  });

  it("refuses CSS typed into a colour and says why", () => {
    renderBuilder();
    const accent = screen.getByRole("textbox", { name: "Accent" });
    const before = screen.getByRole("group", { name: "Preview of your look" }).style.getPropertyValue("--color-accent");
    fireEvent.change(accent, { target: { value: "red; } body { display: none" } });
    expect(screen.getByText(/has CSS in it/)).toBeInTheDocument();
    expect(screen.getByRole("group", { name: "Preview of your look" }).style.getPropertyValue("--color-accent")).toBe(before);
  });

  it("randomises from a seed and the palette passes contrast", async () => {
    const user = userEvent.setup();
    renderBuilder();
    const seed = screen.getByRole("textbox", { name: "Seed" });
    await user.clear(seed);
    await user.type(seed, "42");
    const pv = screen.getByRole("group", { name: "Preview of your look" });
    expect(pv.style.getPropertyValue("--color-bg-base")).toBe(paletteFromSeed(42, "dark")["color-bg-base"]);
    expect(screen.queryByText(/Too low/)).toBeNull();
  });

  it("saves, duplicates and exports, and Use this look puts it to use", async () => {
    const user = userEvent.setup();
    renderBuilder("shape");
    await user.click(within(screen.getByRole("group", { name: "Density" })).getByRole("button", { name: "Compact" }));
    await user.selectOptions(screen.getByRole("combobox", { name: "Text font" }), "atkinson");
    const name = screen.getByRole("textbox", { name: "Name" });
    await user.clear(name);
    await user.type(name, "Night drive");
    await user.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(mock.themes.list().themes.map((t) => t.file)).toContain("Night drive.platter-look"));
    // Saving again writes over its own file.
    await user.click(screen.getByRole("button", { name: "Save" }));
    await user.click(screen.getByRole("button", { name: "Duplicate" }));
    await waitFor(() => expect(mock.themes.list().themes.map((t) => t.name)).toContain("Night drive copy"));
    expect(mock.themes.list().themes.filter((t) => t.name === "Night drive")).toHaveLength(1);
    // Editing carries on in the copy.
    expect(screen.getByRole("textbox", { name: "Name" })).toHaveValue("Night drive copy");
    await user.click(screen.getByRole("button", { name: "Export" }));
    await waitFor(() => expect(mock.themes.lastExport?.file).toBe("Night drive copy.platter-look"));
    await user.click(screen.getByRole("button", { name: "Use this look" }));
    await waitFor(() => expect(useSettings.getState().settings?.themeFile?.name).toBe("Night drive copy"));
    expect(mock.themes.list().themes.filter((t) => t.name === "Night drive copy")).toHaveLength(1);
    const held = useSettings.getState().settings!.themeFile!;
    expect(held.style?.density).toBe("compact");
    expect(held.fonts?.text).toBe("atkinson");
  });

  it("arranges the layout by drag and by keyboard, applies it, and reverts to Classic", async () => {
    const user = userEvent.setup();
    renderBuilder("layout");
    const pv = screen.getByRole("group", { name: "Preview of your look" });
    // Drag Time in front of Album in the song list preview.
    const time = pv.querySelector('[data-drag-id="duration"]')!;
    const album = pv.querySelector('[data-drag-id="album"]')!;
    fireEvent.dragStart(time, { dataTransfer: { setData() {}, effectAllowed: "" } });
    fireEvent.dragOver(album, { dataTransfer: { dropEffect: "" } });
    fireEvent.drop(album, { dataTransfer: {} });
    // Keyboard: move the Queue button up, hide Like.
    await user.click(screen.getByRole("button", { name: "Move Queue up" }));
    await user.click(screen.getByRole("checkbox", { name: "Like" }));
    await user.click(within(screen.getByRole("group", { name: "Side" })).getByRole("button", { name: "Right" }));
    await user.click(screen.getByRole("button", { name: "Apply layout" }));
    await waitFor(() => expect(useSettings.getState().settings?.layout.sidebarSide).toBe("right"));
    const l = useSettings.getState().settings!.layout;
    expect(l.trackColumns).toEqual(["duration", "album", "info"]);
    expect(l.playerButtons).toEqual(["queue", "lyrics", "devices", "volume", "miniplayer", "fullWindow"]);
    // Kept by the core (the setting persists).
    expect((await backend.settingsGet()).layout.trackColumns).toEqual(["duration", "album", "info"]);
    await user.click(screen.getByRole("button", { name: "Revert to default layout" }));
    await user.click(screen.getByRole("button", { name: "Apply layout" }));
    await waitFor(() => expect(useSettings.getState().settings?.layout).toEqual(CLASSIC_LAYOUT));
  });
});

describe("ready-made looks and first run", () => {
  it("shows the built-in themes and the data looks as cards; one click applies", async () => {
    const user = userEvent.setup();
    await act(() => updateSettings({ theme: "studio", followSystem: false }));
    render(<LookCards />);
    const cards = screen.getAllByRole("radio");
    expect(cards.map((c) => c.getAttribute("aria-label"))).toEqual([
      "Studio",
      "Studio Black",
      "Sleeve",
      "Classic",
      "Classic light",
      "Midnight",
      "Forest",
      "Sand",
      "High contrast",
      "OLED black",
    ]);
    expect(screen.getByRole("radio", { name: "Studio" })).toHaveAttribute("aria-checked", "true");
    await user.click(screen.getByRole("radio", { name: "Classic light" }));
    await waitFor(() => expect(useSettings.getState().settings?.theme).toBe("light"));
    await user.click(screen.getByRole("radio", { name: "High contrast" }));
    await waitFor(() => expect(useSettings.getState().settings?.themeFile?.name).toBe("High contrast"));
    expect(screen.getByRole("radio", { name: "High contrast" })).toHaveAttribute("aria-checked", "true");
    // The layout the listener has stays.
    expect(useSettings.getState().settings?.layout).toEqual(CLASSIC_LAYOUT);
  });

  it("a new ready-made look is only a data file; one that fails the checks is skipped", () => {
    const list = dataLooks({
      "./data/a.platter-look": JSON.stringify({ version: 1, name: "Calm", base: "light", tokens: { "color-accent": "#335577" } }),
      "./data/b.platter-look": JSON.stringify({ version: 1, name: "Bad", base: "dark", tokens: { "color-accent": "url(x)" } }),
      "./data/c.platter-look": JSON.stringify({ version: 1, name: "Extra", base: "dark", sparkle: true }),
    });
    expect(list.map((l) => l.name)).toEqual(["Calm"]);
    expect(list[0].tokens["color-bg-base"]).toBe(BUILTIN_THEMES.light.tokens["color-bg-base"]);
    expect(readyLooks().length).toBeGreaterThanOrEqual(7);
  });

  it("no ready-made look copies another brand: names are plain and every one passes contrast", () => {
    for (const r of readyLooks()) {
      for (const c of contrastReport(r.tokens)) expect(c.passes, `${r.name} ${c.rule.token}`).toBe(true);
      expect(r.name).toMatch(/^[A-Za-z ]+$/);
    }
  });

  it("first run offers Pick a look once; skipping remembers it on this computer", async () => {
    const user = userEvent.setup();
    render(<PickLookDialog />);
    offerLookStep();
    await screen.findByRole("dialog", { name: "Pick a look" });
    await user.click(screen.getByRole("button", { name: "Done" }));
    await waitFor(() => expect(useSettings.getState().settings?.lookStepDone).toBe(true));
    offerLookStep();
    expect(useLooks.getState().pick).toBe(false);
    // Make your own leads on to the builder.
    act(() => looks.openPick());
    await user.click(await screen.findByRole("button", { name: "Make your own" }));
    expect(useLooks.getState().builder).toBe("colours");
  });
});

describe("settings", () => {
  const row = (title: string) => {
    const el = screen.getByText(title, { selector: ".setting-row__title" }).closest(".setting-row");
    if (!el) throw new Error(`no row ${title}`);
    return within(el as HTMLElement);
  };

  it("Appearance has Look, Make your own, Layout and Show Make your own, with defaults", async () => {
    const user = userEvent.setup();
    render(<SettingsPage />);
    expect(row("Look").getAllByRole("radio")).toHaveLength(10);
    expect(row("Layout").getByText("Classic layout.")).toBeInTheDocument();
    expect(row("Layout").getByRole("button", { name: "Revert to default layout" })).toBeDisabled();
    expect(row("Show Make your own").getByRole("switch")).toHaveAttribute("aria-checked", "true");
    await user.click(row("Make your own").getByRole("button", { name: "Make your own" }));
    expect(useLooks.getState().builder).toBe("colours");
    await user.click(row("Layout").getByRole("button", { name: "Change layout" }));
    expect(useLooks.getState().builder).toBe("layout");
    await user.click(row("Show Make your own").getByRole("switch"));
    await waitFor(() => expect(useSettings.getState().settings?.showLookBuilder).toBe(false));
    expect(screen.queryByText("Make your own", { selector: ".setting-row__title" })).toBeNull();
  });

  it("lists bundled fonts with their licences under About", () => {
    for (const f of ["Atkinson Hyperlegible typeface", "Manrope typeface", "Fraunces typeface"])
      expect(NOTICES.find((n) => n.name === f)?.licence).toBe("SIL Open Font License 1.1");
  });

  it("the core refuses a bad layout", async () => {
    await expect(backend.settingsSet({ layout: { ...CLASSIC_LAYOUT, headerStyle: "huge" } as unknown as Layout })).rejects.toMatchObject({ kind: "invalid" });
  });
});

describe("dropping a look file on the window", () => {
  it("adds a good look with a Use it button, and says why a bad one can't be used", async () => {
    const text = JSON.stringify({ version: 1, name: "Dropped", base: "dark", tokens: { "color-accent": "#AABBCC" }, extra: 1 });
    await addDroppedFile(new File([text], "Dropped.platter-look"));
    expect(mock.themes.list().themes.map((t) => t.name)).toContain("Dropped");
    const t = lastToast();
    expect(t.message).toMatch(/Added Dropped.*left out/);
    await act(async () => t.action!.run());
    await waitFor(() => expect(useSettings.getState().settings?.themeFile?.name).toBe("Dropped"));

    await addDroppedFile(new File([JSON.stringify({ version: 1, name: "X", base: "dark", tokens: { "color-accent": "url(x)" } })], "x.platter-look"));
    expect(lastToast().message).toMatch(/Couldn't add that look.*url\(\)/);
    await addDroppedFile(new File(["hello"], "notes.txt"));
    expect(lastToast().message).toMatch(/Drop a .platter-look file/);
  });
});

describe("staying light", () => {
  it("the builder and the first-run step load only when opened, and no heavy package came in", () => {
    expect(lazyHosts).toMatch(/lazy\(\(\) => import\("..\/pages\/looks\/LookBuilder"\)/);
    expect(lazyHosts).toMatch(/lazy\(\(\) => import\("..\/pages\/looks\/LookCards"\)/);
    expect(appShell).not.toMatch(/from "..\/pages\/looks\//);
    expect(settingsPage).not.toMatch(/import .*LookBuilder/);
    const deps = Object.keys(pkg.dependencies);
    const fonts = deps.filter((d) => d.startsWith("@fontsource"));
    expect(fonts.sort()).toEqual(["@fontsource-variable/fraunces", "@fontsource-variable/inter", "@fontsource-variable/manrope", "@fontsource/atkinson-hyperlegible"]);
    expect(deps.length - fonts.length).toBe(6);
  });
});
