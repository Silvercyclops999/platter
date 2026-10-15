import { X } from "lucide-react";
import { useId, useMemo, useState } from "react";
import { Button } from "../components/Button";
import { backend } from "../lib/backend";
import type { LibraryItem, Settings } from "../lib/types";
import { useLoad } from "../lib/useLoad";
import { updateSettings } from "../stores/settings";
import "./mixes.css";

type Field = "mixExcludeArtists" | "mixExcludeAlbums" | "mixExcludeGenres";

/**
 * Artists, albums and genres left out of mixes and radio: what's left out,
 * each with a remove button, and a field to add one (artists and albums
 * by name from the library; genres, styles and moods as typed).
 */
export function MixExclusions({ settings, ids }: { settings: Settings; ids: { title: string; desc: string } }) {
  const { data: items } = useLoad(() => backend.libraryItems(), []);
  const listId = useId();
  const [text, setText] = useState("");
  const byId = useMemo(() => new Map((items ?? []).map((i: LibraryItem) => [`${i.kind}:${i.id}`, i])), [items]);
  const options = useMemo(
    () => (items ?? []).filter((i) => i.kind === "artist" || i.kind === "album").map((i) => ({ label: i.kind === "album" ? `${i.title} (album)` : i.title, item: i })),
    [items],
  );
  const chips: { field: Field; value: string; label: string }[] = [
    ...settings.mixExcludeArtists.map((v) => ({ field: "mixExcludeArtists" as const, value: v, label: byId.get(`artist:${v}`)?.title ?? v })),
    ...settings.mixExcludeAlbums.map((v) => ({ field: "mixExcludeAlbums" as const, value: v, label: `${byId.get(`album:${v}`)?.title ?? v} (album)` })),
    ...settings.mixExcludeGenres.map((v) => ({ field: "mixExcludeGenres" as const, value: v, label: v })),
  ];
  const remove = (field: Field, value: string) => void updateSettings({ [field]: settings[field].filter((x) => x !== value) });
  const add = () => {
    const t = text.trim();
    if (!t) return;
    const hit = options.find((o) => o.label.toLowerCase() === t.toLowerCase());
    if (hit) {
      const field: Field = hit.item.kind === "album" ? "mixExcludeAlbums" : "mixExcludeArtists";
      if (!settings[field].includes(hit.item.id)) void updateSettings({ [field]: [...settings[field], hit.item.id] });
    } else if (!settings.mixExcludeGenres.some((g) => g.toLowerCase() === t.toLowerCase())) {
      void updateSettings({ mixExcludeGenres: [...settings.mixExcludeGenres, t] });
    }
    setText("");
  };
  return (
    <div className="mix-exclusions">
      {chips.length > 0 && (
        <ul className="mix-exclusions__list" aria-label="Left out of mixes">
          {chips.map((c) => (
            <li key={`${c.field}:${c.value}`} className="chip mix-exclusions__chip">
              <span>{c.label}</span>
              <button type="button" className="mix-exclusions__remove" aria-label={`Include ${c.label} again`} onClick={() => remove(c.field, c.value)}>
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
          list={listId}
          value={text}
          placeholder="An artist, album or genre"
          aria-labelledby={ids.title}
          aria-describedby={ids.desc}
          onChange={(e) => setText(e.target.value)}
        />
        <datalist id={listId}>
          {options.map((o) => (
            <option key={`${o.item.kind}:${o.item.id}`} value={o.label} />
          ))}
        </datalist>
        <Button variant="secondary" size="sm" type="submit" disabled={!text.trim()}>
          Leave out
        </Button>
      </form>
    </div>
  );
}
