/**
 * Make your own look: colours, shape and type, and layout, with a live
 * picture of the app beside the editor. Loaded the first time it opens.
 *
 * Every change goes on an undo stack (quick runs of changes to one control
 * count as one step). Saving writes a .platter-look file to the themes
 * folder; "Use this look" saves and puts it to use.
 */
import { ArrowDown, ArrowUp, Dices, Image as ImageIcon, Redo2, RotateCcw, Undo2, Wand2, X } from "lucide-react";
import { useCallback, useEffect, useId, useMemo, useRef, useState, type ReactNode } from "react";
import { Button, IconButton } from "../../components/Button";
import { Dialog } from "../../components/Dialog";
import { Select, type SelectOption } from "../../components/Select";
import { Slider } from "../../components/Slider";
import { Switch } from "../../components/Switch";
import { FilterChips, Tabs } from "../../components/Tabs";
import { contrastAgainst, fixColour, ruleFor } from "../../design/looks/contrast";
import { FONTS } from "../../design/looks/fonts";
import { paletteFrom, paletteFromSeed } from "../../design/looks/palette";
import type { LookParts } from "../../design/looks/apply";
import {
  BUILTIN_THEMES,
  CLASSIC_LAYOUT,
  DEFAULT_FONTS,
  DEFAULT_STYLE,
  LOOK,
  THEME_NAME_MAX,
  THEME_TOKEN_NAMES,
  THEME_TOKENS,
  baseTheme,
  isColourValue,
  tokenValueProblem,
  type Layout,
  type ThemeBase,
} from "../../design/themes";
import { artUrl, backend } from "../../lib/backend";
import { headerColorsFromImageUrl } from "../../lib/color";
import type { ThemeFile } from "../../lib/types";
import { lookPatch, looks, useLooks, type BuilderTab } from "../../stores/looks";
import { usePlayer } from "../../stores/player";
import { updateSettings, useSettings } from "../../stores/settings";
import { useTheme } from "../../stores/theme";
import { toast } from "../../stores/toasts";
import { PreviewFrame, type ReorderList } from "./LookPreview";
import "./look-builder.css";
import { failText } from "../../lib/errorText";

/** What the builder edits. */
export interface Draft {
  /** The look file it was saved as ("" until saved). */
  file: string;
  name: string;
  author: string;
  base: ThemeBase;
  /** Every theme token. */
  tokens: Record<string, string>;
  style: LookParts["style"];
  fonts: LookParts["fonts"];
  layout: Layout;
}

/** Colour groups, by what they colour. "Advanced" lists every token. */
export const GROUPS: { id: string; label: string; tokens: string[] }[] = [
  { id: "surfaces", label: "Background and surfaces", tokens: ["color-bg-base", "color-bg-raised", "color-bg-raised2", "color-bg-overlay"] },
  { id: "text", label: "Text", tokens: ["color-text-primary", "color-text-secondary", "color-text-muted", "color-ink"] },
  { id: "accent", label: "Accent", tokens: ["color-accent", "color-accent-hover", "color-accent-pressed", "color-accent-strong", "color-text-on-accent"] },
  { id: "borders", label: "Borders and dividers", tokens: ["color-border"] },
  { id: "states", label: "Cards, chips and buttons", tokens: ["color-state-hover", "color-state-pressed", "color-state-selected"] },
  { id: "playing", label: "Playing song and lyrics", tokens: ["color-playing"] },
  { id: "focus", label: "Focus ring", tokens: ["color-focus-ring"] },
];

const TOKEN_LABEL: Record<string, string> = {
  "color-bg-base": "Window background",
  "color-bg-raised": "Panels",
  "color-bg-raised2": "Raised panels",
  "color-bg-overlay": "Menus and dialogs",
  "color-text-primary": "Main text",
  "color-text-secondary": "Secondary text",
  "color-text-muted": "Quiet text",
  "color-ink": "Ink (lines and tints)",
  "color-accent": "Accent",
  "color-accent-hover": "Accent, pointed at",
  "color-accent-pressed": "Accent, pressed",
  "color-accent-strong": "Accent, strong",
  "color-text-on-accent": "Text on the accent",
  "color-border": "Borders",
  "color-state-hover": "Pointed at",
  "color-state-pressed": "Pressed",
  "color-state-selected": "Selected",
  "color-playing": "Playing song",
  "color-focus-ring": "Focus ring",
};

const tokenLabel = (t: string) => TOKEN_LABEL[t] ?? t.replace(/^color-|^radius-/, "").replace(/-/g, " ");

/** The look a draft saves as. */
export function draftToLook(d: Draft): ThemeFile {
  const look: ThemeFile = { file: d.file, name: d.name.trim() || "My look", base: d.base, tokens: { ...d.tokens }, style: d.style, fonts: d.fonts, layout: d.layout };
  if (d.author.trim()) look.author = d.author.trim();
  return look;
}

/** A draft from a theme or look (tokens filled in from its base). */
export function draftFrom(src: { name: string; base: ThemeBase; tokens: Record<string, string>; file?: string; author?: string; style?: LookParts["style"]; fonts?: LookParts["fonts"]; layout?: Layout }, layout: Layout): Draft {
  return {
    file: src.file?.toLowerCase().endsWith(".platter-look") ? src.file : "",
    name: src.name,
    author: src.author ?? "",
    base: src.base,
    tokens: { ...baseTheme(src.base).tokens, ...src.tokens },
    style: src.style ?? DEFAULT_STYLE,
    fonts: src.fonts ?? DEFAULT_FONTS,
    layout: src.layout ?? layout,
  };
}

interface History {
  past: Draft[];
  now: Draft;
  future: Draft[];
  /** The last control changed and when, so a drag counts as one step. */
  last: { key: string; at: number } | null;
}

function useHistory(initial: Draft) {
  const [h, setH] = useState<History>({ past: [], now: initial, future: [], last: null });
  const change = useCallback((key: string, fn: (d: Draft) => Draft) => {
    setH((h) => {
      const next = fn(h.now);
      if (next === h.now) return h;
      const now = Date.now();
      const merge = h.last && h.last.key === key && now - h.last.at < 800;
      return { past: merge ? h.past : [...h.past.slice(-99), h.now], now: next, future: [], last: { key, at: now } };
    });
  }, []);
  const undo = useCallback(() => setH((h) => (h.past.length ? { past: h.past.slice(0, -1), now: h.past[h.past.length - 1], future: [h.now, ...h.future], last: null } : h)), []);
  const redo = useCallback(() => setH((h) => (h.future.length ? { past: [...h.past, h.now], now: h.future[0], future: h.future.slice(1), last: null } : h)), []);
  return { draft: h.now, change, undo, redo, canUndo: h.past.length > 0, canRedo: h.future.length > 0 };
}

/** A pill row for one choice. */
function Choice<T extends string>({ label, value, options, onChange }: { label: string; value: T; options: { id: T; label: string }[]; onChange: (v: T) => void }) {
  return (
    <div className="lb-row">
      <span className="lb-row__label">{label}</span>
      <FilterChips label={label} items={options} value={value} onChange={onChange} className="lb-choice" />
    </div>
  );
}

function Toggle({ label, checked, onChange }: { label: string; checked: boolean; onChange: (v: boolean) => void }) {
  const id = useId();
  return (
    <div className="lb-row">
      <span className="lb-row__label" id={id}>
        {label}
      </span>
      <Switch checked={checked} onChange={onChange} labelledBy={id} />
    </div>
  );
}

/** An ordered list with show/hide and move up/down (the keyboard way to arrange; the preview takes drags). */
function OrderList<T extends string>({ label, all, shown, names, onChange }: { label: string; all: readonly T[]; shown: T[]; names: Record<T, string>; onChange: (next: T[]) => void }) {
  const rest = all.filter((x) => !shown.includes(x));
  const move = (id: T, by: number) => {
    const i = shown.indexOf(id);
    const j = i + by;
    if (i < 0 || j < 0 || j >= shown.length) return;
    const next = [...shown];
    [next[i], next[j]] = [next[j], next[i]];
    onChange(next);
  };
  return (
    <fieldset className="lb-order">
      <legend className="lb-row__label">{label}</legend>
      <ul className="lb-order__list">
        {[...shown, ...rest].map((id) => {
          const on = shown.includes(id);
          const i = shown.indexOf(id);
          return (
            <li key={id} className={`lb-order__item ${on ? "" : "is-off"}`}>
              <label className="lb-order__check">
                <input type="checkbox" checked={on} onChange={(e) => onChange(e.target.checked ? [...shown, id] : shown.filter((x) => x !== id))} />
                <span>{names[id]}</span>
              </label>
              <IconButton size="sm" label={`Move ${names[id]} up`} icon={<ArrowUp size={14} />} disabled={!on || i === 0} onClick={() => move(id, -1)} />
              <IconButton size="sm" label={`Move ${names[id]} down`} icon={<ArrowDown size={14} />} disabled={!on || i === shown.length - 1} onClick={() => move(id, 1)} />
            </li>
          );
        })}
      </ul>
    </fieldset>
  );
}

/** One colour: picker, value, contrast badge and fix-it. */
function ColourRow({ token, d, set }: { token: string; d: Draft; set: (token: string, v: string, step?: string) => void }) {
  const id = useId();
  const value = d.tokens[token] ?? "";
  const [text, setText] = useState(value);
  useEffect(() => setText(value), [value]);
  const rule = ruleFor(token);
  const ratio = rule ? contrastAgainst(value, rule.against, d.tokens) : null;
  const passes = rule && ratio !== null ? ratio >= rule.min : true;
  const problem = text === value ? null : tokenValueProblem(token, text);
  const hex = /^#[0-9a-f]{6}$/i.test(value) ? value : null;
  const fix = () => {
    if (!rule) return;
    const fixed = fixColour(value, rule.against, d.tokens, rule.min);
    // Its own undo step, apart from typing in the same colour.
    if (fixed) set(token, fixed, "fix");
  };
  return (
    <div className="lb-colour">
      <label className="lb-colour__label" htmlFor={id}>
        {tokenLabel(token)}
      </label>
      {hex ? (
        <input className="lb-colour__swatch" type="color" aria-label={`${tokenLabel(token)} colour`} value={hex.toLowerCase()} onChange={(e) => set(token, e.target.value.toUpperCase())} />
      ) : (
        <span className="lb-colour__swatch lb-colour__swatch--fixed" style={{ background: value }} aria-hidden="true" />
      )}
      <input
        id={id}
        className={`lb-colour__text ${problem ? "is-bad" : ""}`}
        value={text}
        spellCheck={false}
        aria-invalid={!!problem}
        onChange={(e) => {
          setText(e.target.value);
          if (!tokenValueProblem(token, e.target.value)) set(token, e.target.value.trim());
        }}
        onBlur={() => setText(value)}
      />
      {rule && ratio !== null && (
        <span className={`lb-badge ${passes ? "is-ok" : "is-bad"}`} title={`${rule.kind === "text" ? "Text" : "Controls"} need ${rule.min}:1`}>
          {ratio.toFixed(1)}:1 {passes ? (rule.kind === "text" ? "AA" : "OK") : "Too low"}
        </span>
      )}
      {rule && !passes && (
        <Button size="sm" variant="ghost" icon={<Wand2 size={14} />} onClick={fix}>
          Fix
        </Button>
      )}
      {problem && <span className="lb-colour__problem">{problem}</span>}
    </div>
  );
}

const SIDEBAR_NAMES = { servers: "Server", filters: "Filter chips", tools: "Search and sort", status: "Connection status" } as const;
const BUTTON_NAMES = { like: "Like", lyrics: "Lyrics", queue: "Queue", devices: "Devices", volume: "Volume", miniplayer: "Miniplayer", fullWindow: "Full window" } as const;
const SHELF_NAMES = {
  madeForYou: "Made for you",
  recentlyPlayed: "Recently played",
  mixMoods: "Mixes by mood",
  mixGenres: "Genre mixes",
  jumpBackIn: "Jump back in",
  playlists: "Your playlists",
  recentlyAdded: "Recently added",
  topArtists: "Your top artists",
} as const;
const COLUMN_NAMES = { album: "Album", info: "Quality, plays or date", duration: "Time" } as const;

function LayoutEditor({ d, change }: { d: Draft; change: (key: string, fn: (d: Draft) => Draft) => void }) {
  const l = d.layout;
  const set = <K extends keyof Layout>(k: K, v: Layout[K]) => change(`layout.${k}`, (x) => ({ ...x, layout: { ...x.layout, [k]: v } }));
  const c = LOOK.layout;
  return (
    <div className="lb-panel">
      <section className="lb-group">
        <h3 className="lb-group__title">Sidebar</h3>
        <Choice label="Side" value={l.sidebarSide} options={[{ id: "left", label: "Left" }, { id: "right", label: "Right" }]} onChange={(v) => set("sidebarSide", v)} />
        <Toggle label="Collapsed to start with" checked={l.sidebarCollapsed} onChange={(v) => set("sidebarCollapsed", v)} />
        <OrderList label="Sections" all={c.sidebarSections as Layout["sidebarSections"]} shown={l.sidebarSections} names={SIDEBAR_NAMES} onChange={(v) => set("sidebarSections", v)} />
      </section>
      <section className="lb-group">
        <h3 className="lb-group__title">Now playing panel</h3>
        <Choice
          label="Where it opens"
          value={l.nowPlaying}
          options={[
            { id: "right", label: "Right" },
            { id: "left", label: "Left" },
            { id: "off", label: "Only when asked" },
          ]}
          onChange={(v) => set("nowPlaying", v)}
        />
      </section>
      <section className="lb-group">
        <h3 className="lb-group__title">Player bar</h3>
        <Choice
          label="Style"
          value={l.playerStyle}
          options={[
            { id: "classic", label: "Classic" },
            { id: "compact", label: "Compact" },
            { id: "largeArt", label: "Large art" },
          ]}
          onChange={(v) => set("playerStyle", v)}
        />
        <OrderList label="Buttons" all={c.playerButtons as Layout["playerButtons"]} shown={l.playerButtons} names={BUTTON_NAMES} onChange={(v) => set("playerButtons", v)} />
      </section>
      <section className="lb-group">
        <h3 className="lb-group__title">Home</h3>
        <Toggle label="Greeting at the top" checked={l.homeGreeting} onChange={(v) => set("homeGreeting", v)} />
        <Choice
          label="Quick access grid"
          value={String(l.quickAccess) as "4" | "6" | "8"}
          options={c.quickAccess.map((n) => ({ id: String(n) as "4" | "6" | "8", label: `${n} tiles` }))}
          onChange={(v) => set("quickAccess", Number(v) as Layout["quickAccess"])}
        />
        <OrderList label="Shelves" all={c.homeShelves as Layout["homeShelves"]} shown={l.homeShelves} names={SHELF_NAMES} onChange={(v) => set("homeShelves", v)} />
      </section>
      <section className="lb-group">
        <h3 className="lb-group__title">Album and playlist pages</h3>
        <Choice
          label="Header"
          value={l.headerStyle}
          options={[
            { id: "bigArt", label: "Big art" },
            { id: "banner", label: "Banner" },
            { id: "minimal", label: "Minimal" },
          ]}
          onChange={(v) => set("headerStyle", v)}
        />
        <OrderList label="Song list columns" all={c.trackColumns as Layout["trackColumns"]} shown={l.trackColumns} names={COLUMN_NAMES} onChange={(v) => set("trackColumns", v)} />
      </section>
      <section className="lb-group">
        <h3 className="lb-group__title">Cover art</h3>
        <Choice
          label="Shape"
          value={l.coverShape}
          options={[
            { id: "square", label: "Square" },
            { id: "rounded", label: "Rounded" },
            { id: "circle", label: "Circle" },
            { id: "vinyl", label: "Vinyl" },
            { id: "sleeve", label: "Sleeve" },
          ]}
          onChange={(v) => set("coverShape", v)}
        />
        <Toggle label="Spinning record in now playing" checked={l.spinRecord} onChange={(v) => set("spinRecord", v)} />
      </section>
      <section className="lb-group">
        <h3 className="lb-group__title">Motion</h3>
        <Choice
          label="Movement"
          value={l.motion}
          options={[
            { id: "full", label: "Full" },
            { id: "reduced", label: "Reduced" },
            { id: "off", label: "Off" },
          ]}
          onChange={(v) => set("motion", v)}
        />
        <p className="lb-note">Your computer's reduce motion setting is always respected.</p>
      </section>
      <div className="lb-actions">
        <Button size="sm" icon={<RotateCcw size={14} />} onClick={() => change("layout.reset", (x) => ({ ...x, layout: structuredClone(CLASSIC_LAYOUT) }))}>
          Revert to default layout
        </Button>
        <Button
          size="sm"
          variant="primary"
          onClick={() =>
            void updateSettings({ layout: d.layout }).then(
              () => toast.info("Layout applied"),
              (e) => toast.error(failText(`Couldn't apply the layout`, e)),
            )
          }
        >
          Apply layout
        </Button>
      </div>
    </div>
  );
}

function ShapeEditor({ d, change }: { d: Draft; change: (key: string, fn: (d: Draft) => Draft) => void }) {
  const radius = parseFloat(d.tokens["radius-card"] ?? "8") || 0;
  const setStyle = <K extends keyof Draft["style"]>(k: K, v: Draft["style"][K]) => change(`style.${k}`, (x) => ({ ...x, style: { ...x.style, [k]: v } }));
  const fontOptions: SelectOption<Draft["fonts"]["text"]>[] = FONTS.map((f) => ({ value: f.id, label: f.label }));
  const textId = useId();
  const headId = useId();
  return (
    <div className="lb-panel">
      <section className="lb-group">
        <h3 className="lb-group__title">Shape</h3>
        <div className="lb-row lb-row--slider">
          <span className="lb-row__label">Corner radius</span>
          <Slider
            label="Corner radius"
            value={radius}
            max={24}
            step={1}
            valueText={(v) => `${Math.round(v)} pixels`}
            onChange={(v) =>
              change("radius", (x) => ({
                ...x,
                tokens: { ...x.tokens, "radius-card": `${Math.round(v)}px`, "radius-row": `${Math.round(v / 2)}px` },
              }))
            }
          />
          <span className="lb-row__value">{Math.round(radius)}px</span>
        </div>
        <div className="lb-row lb-row--slider">
          <span className="lb-row__label">Border weight</span>
          <Slider label="Border weight" value={d.style.border} max={LOOK.borderMax} step={1} valueText={(v) => `${Math.round(v)} pixels`} onChange={(v) => setStyle("border", Math.round(v))} />
          <span className="lb-row__value">{d.style.border}px</span>
        </div>
        <Choice
          label="Density"
          value={d.style.density}
          options={[
            { id: "compact", label: "Compact" },
            { id: "comfortable", label: "Comfortable" },
            { id: "spacious", label: "Spacious" },
          ]}
          onChange={(v) => setStyle("density", v)}
        />
        <Choice
          label="Shadow"
          value={d.style.shadow}
          options={[
            { id: "flat", label: "Flat" },
            { id: "subtle", label: "Subtle" },
            { id: "soft", label: "Soft" },
          ]}
          onChange={(v) => setStyle("shadow", v)}
        />
        <Choice
          label="Blur on menus and dialogs"
          value={d.style.blur}
          options={[
            { id: "off", label: "Off" },
            { id: "low", label: "Low" },
            { id: "medium", label: "Medium" },
          ]}
          onChange={(v) => setStyle("blur", v)}
        />
      </section>
      <section className="lb-group">
        <h3 className="lb-group__title">Type</h3>
        <div className="lb-row">
          <span className="lb-row__label" id={textId}>
            Text font
          </span>
          <Select value={d.fonts.text} options={fontOptions} labelledBy={textId} onChange={(v) => change("fonts.text", (x) => ({ ...x, fonts: { ...x.fonts, text: v } }))} />
        </div>
        <div className="lb-row">
          <span className="lb-row__label" id={headId}>
            Heading font
          </span>
          <Select value={d.fonts.headings} options={fontOptions} labelledBy={headId} onChange={(v) => change("fonts.headings", (x) => ({ ...x, fonts: { ...x.fonts, headings: v } }))} />
        </div>
        <p className="lb-note">Fonts come with Platter and are free to share. Nothing is downloaded.</p>
      </section>
    </div>
  );
}

function ColourEditor({ d, change, startTokens }: { d: Draft; change: (key: string, fn: (d: Draft) => Draft) => void; startTokens: Record<string, string> }) {
  const [advanced, setAdvanced] = useState(false);
  const [seed, setSeed] = useState(() => Math.floor(Math.random() * 100000));
  const thumb = usePlayer((s) => s.state?.track?.thumb ?? null);
  const set = useCallback(
    (token: string, v: string, step = "edit") => change(`${step}.${token}`, (x) => ({ ...x, tokens: { ...x.tokens, [token]: v } })),
    [change],
  );
  const fromCover = async () => {
    const url = artUrl(thumb, 64);
    const colors = url ? await headerColorsFromImageUrl(url) : null;
    if (!colors) {
      toast.error("Couldn't read colours from that cover");
      return;
    }
    change("palette", (x) => ({ ...x, tokens: paletteFrom(colors.vivid, x.base) }));
  };
  const randomise = (n: number) => {
    setSeed(n);
    change("palette", (x) => ({ ...x, tokens: paletteFromSeed(n, x.base) }));
  };
  const seedId = useId();
  const grouped = new Set(GROUPS.flatMap((g) => g.tokens));
  const others = THEME_TOKEN_NAMES.filter((t) => !grouped.has(t) && THEME_TOKENS[t] === "colour");
  return (
    <div className="lb-panel">
      <section className="lb-group">
        <h3 className="lb-group__title">Make a palette</h3>
        <div className="lb-actions lb-actions--start">
          <Button size="sm" icon={<ImageIcon size={14} />} disabled={!thumb} onClick={() => void fromCover()}>
            Generate from cover
          </Button>
          <Button size="sm" icon={<Dices size={14} />} onClick={() => randomise(Math.floor(Math.random() * 100000))}>
            Randomise
          </Button>
          <label className="lb-seed" htmlFor={seedId}>
            Seed
            <input
              id={seedId}
              className="lb-seed__input"
              inputMode="numeric"
              value={seed}
              onChange={(e) => {
                const n = Number(e.target.value.replace(/\D/g, "").slice(0, 9));
                if (Number.isFinite(n)) randomise(n);
              }}
            />
          </label>
        </div>
        {!thumb && <p className="lb-note">Play something to build a palette from its cover.</p>}
      </section>
      {GROUPS.map((g) => (
        <section key={g.id} className="lb-group" aria-label={g.label}>
          <div className="lb-group__head">
            <h3 className="lb-group__title">{g.label}</h3>
            <Button
              size="sm"
              variant="ghost"
              onClick={() => change(`reset.${g.id}`, (x) => ({ ...x, tokens: { ...x.tokens, ...Object.fromEntries(g.tokens.map((t) => [t, startTokens[t]])) } }))}
            >
              Reset group
            </Button>
          </div>
          {g.tokens.map((t) => (
            <ColourRow key={t} token={t} d={d} set={set} />
          ))}
        </section>
      ))}
      <div className="lb-row">
        <span className="lb-row__label">Advanced: every colour</span>
        <Switch checked={advanced} onChange={setAdvanced} label="Advanced: every colour" />
      </div>
      {advanced && (
        <section className="lb-group" aria-label="Every other colour">
          <h3 className="lb-group__title">Status and other colours</h3>
          {others.map((t) => (
            <ColourRow key={t} token={t} d={d} set={set} />
          ))}
        </section>
      )}
    </div>
  );
}

/** The builder's body (also rendered alone in tests). */
export function LookBuilder({ tab, onTab, onClose }: { tab: BuilderTab; onTab: (t: BuilderTab) => void; onClose: () => void }) {
  const settings = useSettings((s) => s.settings);
  const resolved = useTheme((s) => s.resolved);
  const layout = settings?.layout ?? CLASSIC_LAYOUT;
  const initial = useMemo(() => {
    const f = settings?.themeFile;
    if (f) return draftFrom({ ...f, layout: f.layout ?? layout }, layout);
    return draftFrom({ name: "My look", base: resolved.base, tokens: resolved.tokens }, layout);
    // Only the look in use when the builder opened.
  }, []);
  const { draft: d, change, undo, redo, canUndo, canRedo } = useHistory(initial);
  const [startTokens, setStartTokens] = useState(initial.tokens);
  const [page, setPage] = useState<"home" | "album">(tab === "layout" ? "album" : "home");
  const [folder, setFolder] = useState<ThemeFile[]>([]);
  const folderVersion = useLooks((s) => s.folderVersion);
  const nameId = useId();
  const authorId = useId();
  const startId = useId();
  const busy = useRef(false);

  useEffect(() => {
    backend.themesList().then(
      (f) => setFolder(f.themes),
      () => setFolder([]),
    );
  }, [folderVersion]);

  // Ctrl+Z / Ctrl+Shift+Z (Cmd on a Mac) while the builder is open, outside text fields.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const t = e.target as HTMLElement | null;
      if (t && (t.tagName === "INPUT" || t.tagName === "TEXTAREA")) return;
      if (!(e.ctrlKey || e.metaKey) || e.key.toLowerCase() !== "z") return;
      e.preventDefault();
      if (e.shiftKey) redo();
      else undo();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [undo, redo]);

  const startOptions: SelectOption<string>[] = [
    { value: "", label: "Choose a starting point" },
    ...Object.values(BUILTIN_THEMES).map((t) => ({ value: `builtin:${t.id}`, label: t.name })),
    ...folder.map((t) => ({ value: `file:${t.file}`, label: t.name })),
  ];
  const startFrom = (v: string) => {
    if (!v) return;
    const [kind, id] = [v.slice(0, v.indexOf(":")), v.slice(v.indexOf(":") + 1)];
    const src = kind === "builtin" ? BUILTIN_THEMES[id as keyof typeof BUILTIN_THEMES] : folder.find((t) => t.file === id);
    if (!src) return;
    const next = draftFrom({ ...src, name: kind === "builtin" ? `My ${src.name}` : src.name }, d.layout);
    setStartTokens(next.tokens);
    change("start", () => ({ ...next, layout: next.layout }));
  };

  const setBase = (base: ThemeBase) =>
    change("base", (x) => (x.base === base ? x : { ...x, base, tokens: paletteFrom(isColourValue(x.tokens["color-accent"] ?? "") ? x.tokens["color-accent"] : "#888888", base) }));

  const nameProblem = !d.name.trim() ? "Give your look a name." : d.name.trim().length > THEME_NAME_MAX || /[<>]/.test(d.name) ? `Names can be up to ${THEME_NAME_MAX} plain characters.` : null;

  const save = async (use: boolean, copy = false): Promise<void> => {
    if (nameProblem || busy.current) return;
    busy.current = true;
    try {
      const look = draftToLook(copy ? { ...d, file: "", name: `${d.name.trim()} copy`.slice(0, THEME_NAME_MAX) } : d);
      const saved = await backend.themesSave(look, !copy);
      looks.folderChanged();
      change("saved", (x) => ({ ...x, file: saved.file, name: saved.name }));
      if (use) await updateSettings(lookPatch(saved));
      toast.info(copy ? `Saved a copy: ${saved.name}` : use ? `Using ${saved.name}` : `Saved ${saved.name}`);
    } catch (e) {
      toast.error(failText(`Couldn't save the look`, e));
    } finally {
      busy.current = false;
    }
  };

  const exportLook = async () => {
    if (nameProblem) return;
    try {
      const file = await backend.themesExport(draftToLook(d));
      if (file) toast.info(`Exported ${file}`);
    } catch (e) {
      toast.error(failText(`Couldn't export the look`, e));
    }
  };

  const reorder = (list: ReorderList, id: string, before: string | null) =>
    change(`drag.${list}`, (x) => {
      const cur = x.layout[list] as string[];
      const without = cur.filter((i) => i !== id);
      const at = before === null ? without.length : without.indexOf(before);
      without.splice(at < 0 ? without.length : at, 0, id);
      return { ...x, layout: { ...x.layout, [list]: without } };
    });

  const parts: LookParts = { style: d.style, fonts: d.fonts, layout: d.layout };
  const panel: ReactNode =
    tab === "colours" ? <ColourEditor d={d} change={change} startTokens={startTokens} /> : tab === "shape" ? <ShapeEditor d={d} change={change} /> : <LayoutEditor d={d} change={change} />;

  return (
    <div className="lb">
      <header className="lb-head">
        <h2 className="lb-head__title">Make your own look</h2>
        <div className="lb-head__history">
          <IconButton label="Undo" icon={<Undo2 size={18} />} disabled={!canUndo} onClick={undo} />
          <IconButton label="Redo" icon={<Redo2 size={18} />} disabled={!canRedo} onClick={redo} />
        </div>
        <div className="lb-head__actions">
          <Button size="sm" disabled={!!nameProblem} onClick={() => void save(false, true)}>
            Duplicate
          </Button>
          <Button size="sm" disabled={!!nameProblem} onClick={() => void exportLook()}>
            Export
          </Button>
          <Button size="sm" disabled={!!nameProblem} onClick={() => void save(false)}>
            Save
          </Button>
          <Button size="sm" variant="primary" disabled={!!nameProblem} onClick={() => void save(true)}>
            Use this look
          </Button>
          <IconButton label="Close" icon={<X size={18} />} onClick={onClose} />
        </div>
      </header>
      <div className="lb-main">
        <div className="lb-editor">
          <div className="lb-fields">
            <label className="lb-field" htmlFor={nameId}>
              <span className="lb-row__label">Name</span>
              <input id={nameId} className="lb-input" value={d.name} maxLength={THEME_NAME_MAX} aria-invalid={!!nameProblem} onChange={(e) => change("name", (x) => ({ ...x, name: e.target.value }))} />
            </label>
            <label className="lb-field" htmlFor={authorId}>
              <span className="lb-row__label">Made by (optional)</span>
              <input id={authorId} className="lb-input" value={d.author} maxLength={THEME_NAME_MAX} onChange={(e) => change("author", (x) => ({ ...x, author: e.target.value }))} />
            </label>
            {nameProblem && <p className="lb-colour__problem">{nameProblem}</p>}
            <div className="lb-field">
              <span className="lb-row__label" id={startId}>
                Start from
              </span>
              <Select value="" options={startOptions} labelledBy={startId} onChange={startFrom} />
            </div>
          </div>
          <Tabs
            label="Look builder"
            idPrefix="look-builder"
            value={tab}
            onChange={onTab}
            items={[
              { id: "colours", label: "Colours" },
              { id: "shape", label: "Shape and type" },
              { id: "layout", label: "Layout" },
            ]}
          />
          <div id="look-builder-panel" role="tabpanel" aria-labelledby={`look-builder-tab-${tab}`} className="lb-scroll">
            {panel}
          </div>
        </div>
        <div className="lb-preview">
          <div className="lb-preview__bar">
            <FilterChips
              label="Base"
              value={d.base}
              items={[
                { id: "dark", label: "Dark" },
                { id: "light", label: "Light" },
              ]}
              onChange={setBase}
            />
            <FilterChips
              label="Page shown"
              value={page}
              items={[
                { id: "home", label: "Home" },
                { id: "album", label: "Album" },
              ]}
              onChange={setPage}
            />
          </div>
          <PreviewFrame base={d.base} tokens={d.tokens} parts={parts} page={page} onReorder={reorder} label="Preview of your look" />
          <p className="lb-note">Drag sidebar sections, Home shelves, player buttons and song list columns in the preview to arrange them. The preview uses made-up music.</p>
        </div>
      </div>
    </div>
  );
}

/** The builder as a full-window dialog, opened from Settings, the first-run step or the command palette. */
export function LookBuilderHost() {
  const tab = useLooks((s) => s.builder);
  return (
    <Dialog open={tab !== null} onClose={() => looks.closeBuilder()} label="Make your own look" className="look-builder">
      {tab && <LookBuilder tab={tab} onTab={(t) => looks.openBuilder(t)} onClose={() => looks.closeBuilder()} />}
    </Dialog>
  );
}
