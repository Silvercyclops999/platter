/**
 * The album page's edition picker: albums sharing a group tag are editions
 * of one album (Standard, Deluxe, Build v2...). Picking one switches the
 * page to that edition's songs, in place (Back skips the switch).
 */
import { useId } from "react";
import { Select } from "../components/Select";
import type { Edition } from "../lib/types";
import { useEditions } from "../stores/fileTags";
import { useRouter } from "../stores/router";
import { useSettings } from "../stores/settings";
import "../components/file-tags.css";

/** What an edition is called in the picker: its tag, else its title, with the year. */
export function editionName(e: Edition, all: Edition[]): string {
  const base = e.label?.trim() || e.title;
  // Two editions with the same name: the year tells them apart.
  const clash = all.filter((x) => (x.label?.trim() || x.title) === base).length > 1;
  return clash && e.year ? `${base} (${e.year})` : base;
}

export function EditionPicker({ albumId }: { albumId: string }) {
  const editions = useEditions(albumId);
  const on = useSettings((s) => s.settings?.groupEditions !== false);
  const id = useId();
  if (!on || editions.length < 2) return null;
  return (
    <div className="edition-picker">
      <span id={id} className="edition-picker__label">
        Edition
      </span>
      <Select
        value={albumId}
        labelledBy={id}
        options={editions.map((e) => ({ value: e.albumId, label: editionName(e, editions) }))}
        onChange={(next) => {
          if (next !== albumId) useRouter.getState().replace({ name: "album", id: next });
        }}
      />
    </div>
  );
}
