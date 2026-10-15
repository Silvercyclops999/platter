/**
 * Appearance rows for themes: Theme, Accent colour and Theme file.
 */
import { Check, Pipette } from "lucide-react";
import { useCallback, useEffect, useRef, useState, type CSSProperties } from "react";
import { Button } from "../components/Button";
import { Select, type SelectOption } from "../components/Select";
import { Skeleton } from "../components/Skeleton";
import { ACCENT_PRESETS, accentColour, accentProblem, isCustomAccent } from "../design/themes";
import { backend } from "../lib/backend";
import type { AppError, Settings, ThemeFile, ThemeFolder, ThemeSetting } from "../lib/types";
import { updateSettings } from "../stores/settings";
import { useTheme } from "../stores/theme";
import { toast } from "../stores/toasts";
import "./theme-settings.css";

type Ids = { title: string; desc: string };

const errText = (e: unknown) => (e && typeof e === "object" && "message" in e ? String((e as AppError).message) : "");

export const THEMES: SelectOption<ThemeSetting>[] = [
  { value: "dark", label: "Dark" },
  { value: "light", label: "Light" },
  { value: "system", label: "Match system" },
  { value: "midnight", label: "Midnight" },
  { value: "forest", label: "Forest" },
  { value: "sand", label: "Sand" },
];

export function ThemeSelect({ s, ids }: { s: Settings; ids: Ids }) {
  return (
    <Select
      value={s.theme ?? "dark"}
      options={THEMES}
      onChange={(v) => void updateSettings({ theme: v })}
      labelledBy={ids.title}
      describedBy={ids.desc}
      disabled={!!s.themeFile}
    />
  );
}

/** The Theme row's note while a theme file is in use. */
export function themeNote(s: Settings | null): string | undefined {
  return s?.themeFile ? `The theme file ${s.themeFile.name} is in use. Set Theme file to None to pick one of these.` : undefined;
}

/** Swatches for ivory and the presets, plus a custom colour. */
export function AccentPicker({ s, ids, onProblem }: { s: Settings; ids: Ids; onProblem: (reason: string | null) => void }) {
  const resolved = useTheme((t) => t.resolved);
  const value = s.accent ?? "ivory";
  const custom = isCustomAccent(value) ? value : null;
  const pick = (v: string) => {
    onProblem(null);
    void updateSettings({ accent: v });
  };
  const pickCustom = (hex: string) => {
    const problem = accentProblem(hex, resolved.tokens);
    if (problem) {
      onProblem(problem);
      return;
    }
    pick(hex.toUpperCase());
  };
  // The picker reports every step while dragging; only the colour it closes on is saved.
  const [draft, setDraft] = useState<string | null>(null);
  const input = useRef<HTMLInputElement>(null);
  const commit = useRef(pickCustom);
  commit.current = pickCustom;
  useEffect(() => {
    const el = input.current;
    if (!el) return;
    const onCommit = () => {
      setDraft(null);
      commit.current(el.value);
    };
    el.addEventListener("change", onCommit);
    return () => el.removeEventListener("change", onCommit);
  }, []);
  return (
    <div className="accent-picker">
      <div className="accent-picker__presets" role="radiogroup" aria-labelledby={ids.title} aria-describedby={ids.desc}>
      {ACCENT_PRESETS.map((p) => {
        const on = value === p.id;
        const colour = p[resolved.base];
        return (
          <button
            key={p.id}
            type="button"
            role="radio"
            aria-checked={on}
            aria-label={p.label}
            title={p.label}
            className={`accent-swatch ${on ? "is-on" : ""}`}
            style={{ "--swatch": colour } as CSSProperties}
            onClick={() => pick(p.id)}
          >
            {on && <Check size={14} aria-hidden="true" />}
          </button>
        );
      })}
      </div>
      <label
        className={`accent-swatch accent-swatch--custom ${custom ? "is-on" : ""}`}
        title="Custom colour"
        style={custom || draft ? ({ "--swatch": draft ?? custom } as CSSProperties) : undefined}
      >
        <input
          ref={input}
          type="color"
          className="accent-swatch__input"
          aria-label={custom ? `Custom colour, ${custom}, in use` : "Custom colour"}
          aria-describedby={ids.desc}
          value={(draft ?? custom ?? accentColour("ivory", resolved.base) ?? "#ECE4D3").toLowerCase()}
          onChange={(e) => setDraft(e.target.value)}
        />
        {custom ? <Check size={14} aria-hidden="true" /> : <Pipette size={14} aria-hidden="true" />}
      </label>
    </div>
  );
}

/** The themes folder's files, reloaded when the window gets focus back (files added by hand). */
export function useThemeFolder(): [ThemeFolder | null, () => void] {
  const [folder, setFolder] = useState<ThemeFolder | null>(null);
  const reload = useCallback(() => {
    backend.themesList().then(setFolder, () => setFolder({ folder: "", themes: [], invalid: [] }));
  }, []);
  useEffect(() => {
    reload();
    window.addEventListener("focus", reload);
    return () => window.removeEventListener("focus", reload);
  }, [reload]);
  return [folder, reload];
}

const NONE = "";

export function ThemeFileControl({ s, ids, folder, reload }: { s: Settings; ids: Ids; folder: ThemeFolder | null; reload: () => void }) {
  if (!folder) return <Skeleton width={160} height={32} radius="pill" />;
  const current = s.themeFile;
  const themes: ThemeFile[] = [...folder.themes];
  // A file that came through the sync file may not be in this computer's folder.
  if (current && !themes.some((t) => t.file === current.file)) themes.unshift(current);
  const options: SelectOption<string>[] = [{ value: NONE, label: "None" }, ...themes.map((t) => ({ value: t.file, label: t.name }))];
  const choose = (file: string) => void updateSettings({ themeFile: themes.find((t) => t.file === file) ?? null });
  const importFile = () =>
    backend.themesImport().then(
      (t) => {
        reload();
        if (t) {
          toast.info(`Added ${t.name}`);
          void updateSettings({ themeFile: t });
        }
      },
      (e) => toast.error(`Couldn't add that theme${errText(e) ? `: ${errText(e)}` : ""}`),
    );
  return (
    <>
      <Select value={current?.file ?? NONE} options={options} onChange={choose} labelledBy={ids.title} describedBy={ids.desc} />
      <Button size="sm" onClick={() => void importFile()}>
        Import
      </Button>
      <Button size="sm" onClick={() => void backend.themesOpenFolder().catch((e) => toast.error(`Couldn't open the folder${errText(e) ? `: ${errText(e)}` : ""}`))}>
        Open themes folder
      </Button>
    </>
  );
}

/** Theme files in the folder that can't be used, each with the reason. */
export function ThemeFileProblems({ folder }: { folder: ThemeFolder | null }) {
  if (!folder?.invalid.length) return null;
  return (
    <div className="theme-problems">
      <p className="theme-problems__title">These files in the themes folder can't be used:</p>
      <ul className="theme-problems__list">
        {folder.invalid.map((f) => (
          <li key={f.file} className="theme-problems__item">
            <span className="theme-problems__file">{f.file}</span>
            <span className="theme-problems__reason">{f.reason}</span>
          </li>
        ))}
      </ul>
    </div>
  );
}
