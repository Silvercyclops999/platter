import { X } from "lucide-react";
import { useState } from "react";
import { Button } from "../components/Button";
import type { Settings } from "../lib/types";
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
