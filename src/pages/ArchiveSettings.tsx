import { X } from "lucide-react";
import { useEffect, useState } from "react";
import { Button } from "../components/Button";
import { Select, type SelectOption } from "../components/Select";
import type { FileRelease, ReleaseValueMap, Settings } from "../lib/types";
import { updateSettings } from "../stores/settings";
import "./mixes.css";

/** Most keywords kept, and the longest one (the core's limits). */
export const FOLDER_KEYWORDS_MAX = 20;
export const FOLDER_KEYWORD_LEN_MAX = 40;

/** A keyword as it would be kept, or null when it can't be (blank, too long, a slash, already there). */
export function cleanKeyword(text: string, current: string[]): string | null {
  const t = text.trim();
  if (!t || t.length > FOLDER_KEYWORD_LEN_MAX || /[\\/]/.test(t)) return null;
  if (current.some((k) => k.toLowerCase() === t.toLowerCase())) return null;
  return current.length >= FOLDER_KEYWORDS_MAX ? null : t;
}

/**
 * Folder names that hint a song is unreleased: each with a remove button,
 * and a field to add one. A hint only adds weight to "Why?".
 */
export function FolderKeywords({ settings, ids }: { settings: Settings; ids: { title: string; desc: string } }) {
  const [text, setText] = useState("");
  const list = settings.archiveFolderKeywords ?? [];
  const add = () => {
    const k = cleanKeyword(text, list);
    if (k) void updateSettings({ archiveFolderKeywords: [...list, k] });
    setText("");
  };
  return (
    <div className="mix-exclusions">
      {list.length > 0 && (
        <ul className="mix-exclusions__list" aria-label="Folder names">
          {list.map((k) => (
            <li key={k} className="chip mix-exclusions__chip">
              <span>{k}</span>
              <button
                type="button"
                className="mix-exclusions__remove"
                aria-label={`Remove ${k}`}
                onClick={() => void updateSettings({ archiveFolderKeywords: list.filter((x) => x !== k) })}
              >
                <X size={14} aria-hidden="true" />
              </button>
            </li>
          ))}
        </ul>
      )}
      <form
        className="mix-exclusions__add"
        onSubmit={(e) => {
          e.preventDefault();
          add();
        }}
      >
        <input
          className="setting-text"
          value={text}
          maxLength={FOLDER_KEYWORD_LEN_MAX}
          placeholder="A folder name, like Vault"
          aria-labelledby={ids.title}
          aria-describedby={ids.desc}
          onChange={(e) => setText(e.target.value)}
        />
        <Button variant="secondary" size="sm" type="submit" disabled={!cleanKeyword(text, list)}>
          Add
        </Button>
      </form>
    </div>
  );
}

/** The core's limits on the release status tag and its custom values. */
export const STATUS_TAG_LEN_MAX = 64;
export const STATUS_VALUES_MAX = 50;
export const STATUS_VALUE_LEN_MAX = 60;

export const FILE_RELEASE_OPTIONS: readonly SelectOption<FileRelease>[] = [
  { value: "released", label: "Released" },
  { value: "unreleased", label: "Unreleased" },
  { value: "leaked", label: "Leaked" },
  { value: "alternate", label: "Alternate" },
];

/** A tag name as it would be kept, or null for Auto (blank or "auto") or one the core refuses. */
export function cleanStatusTag(text: string): string | null {
  const t = text.trim();
  if (!t || t.toLowerCase() === "auto" || t.length > STATUS_TAG_LEN_MAX || t.includes("=")) return null;
  return t;
}

/** How the core compares values: any case, `_` and `-` as spaces, one space between words. */
export function foldStatusValue(v: string): string {
  return v
    .trim()
    .toLowerCase()
    .replace(/[_-]/g, " ")
    .replace(/^[\s"'.:;,!*]+|[\s"'.:;,!*]+$/g, "")
    .split(/\s+/)
    .filter(Boolean)
    .join(" ");
}

/** The list with `value` mapped to `status`: the same value again replaces the earlier one. */
export function withStatusValue(list: ReleaseValueMap[], value: string, status: FileRelease): ReleaseValueMap[] | null {
  const v = value.trim();
  if (!v || v.length > STATUS_VALUE_LEN_MAX) return null;
  const rest = list.filter((m) => foldStatusValue(m.value) !== foldStatusValue(v));
  return rest.length >= STATUS_VALUES_MAX ? null : [...rest, { value: v, status }];
}

/** Auto, or a custom tag the release status is read from. */
export function ReleaseStatusTag({ settings, ids }: { settings: Settings; ids: { title: string; desc: string } }) {
  const current = settings.releaseStatusTag ?? "";
  const [mode, setMode] = useState<"auto" | "custom">(current ? "custom" : "auto");
  const [text, setText] = useState(current);
  useEffect(() => {
    setText(current);
    if (current) setMode("custom");
  }, [current]);
  const tag = cleanStatusTag(text);
  return (
    <div className="mix-exclusions">
      <Select
        value={mode}
        options={[
          { value: "auto", label: "Auto" },
          { value: "custom", label: "Custom tag" },
        ]}
        onChange={(v) => {
          setMode(v);
          if (v === "auto" && current) void updateSettings({ releaseStatusTag: "" });
        }}
        labelledBy={ids.title}
        describedBy={ids.desc}
      />
      {mode === "custom" && (
        <form
          className="mix-exclusions__add"
          onSubmit={(e) => {
            e.preventDefault();
            if (tag) void updateSettings({ releaseStatusTag: tag });
          }}
        >
          <input
            className="setting-text"
            value={text}
            maxLength={STATUS_TAG_LEN_MAX}
            placeholder="A tag name, like PROVENANCE"
            aria-label="Release status tag name"
            onChange={(e) => setText(e.target.value)}
          />
          <Button variant="secondary" size="sm" type="submit" disabled={!tag || tag === current}>
            Use
          </Button>
        </form>
      )}
    </div>
  );
}

const STATUS_LABEL: Record<FileRelease, string> = { released: "Released", unreleased: "Unreleased", leaked: "Leaked", alternate: "Alternate" };

/** Custom release status values: each with what it means and a remove button, and a field to add one. */
export function ReleaseStatusValues({ settings, ids }: { settings: Settings; ids: { title: string; desc: string } }) {
  const [text, setText] = useState("");
  const [status, setStatus] = useState<FileRelease>("released");
  const list = settings.releaseStatusValues ?? [];
  const next = withStatusValue(list, text, status);
  return (
    <div className="mix-exclusions">
      {list.length > 0 && (
        <ul className="mix-exclusions__list" aria-label="Release status values">
          {list.map((m) => (
            <li key={m.value} className="chip mix-exclusions__chip">
              <span>
                {m.value}: {STATUS_LABEL[m.status]}
              </span>
              <button
                type="button"
                className="mix-exclusions__remove"
                aria-label={`Remove ${m.value}`}
                onClick={() => void updateSettings({ releaseStatusValues: list.filter((x) => x.value !== m.value) })}
              >
                <X size={14} aria-hidden="true" />
              </button>
            </li>
          ))}
        </ul>
      )}
      <form
        className="mix-exclusions__add"
        onSubmit={(e) => {
          e.preventDefault();
          if (next) void updateSettings({ releaseStatusValues: next });
          setText("");
        }}
      >
        <input
          className="setting-text"
          value={text}
          maxLength={STATUS_VALUE_LEN_MAX}
          placeholder="A value, like Vault copy"
          aria-labelledby={ids.title}
          aria-describedby={ids.desc}
          onChange={(e) => setText(e.target.value)}
        />
        <Select value={status} options={FILE_RELEASE_OPTIONS} onChange={setStatus} label="Means" />
        <Button variant="secondary" size="sm" type="submit" disabled={!next}>
          Add
        </Button>
      </form>
    </div>
  );
}
