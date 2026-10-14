import { useEffect, useState } from "react";
import { Skeleton } from "../components/Skeleton";
import { DEFAULT_SETTINGS } from "../lib/types";
import { updateSettings, useSettings } from "../stores/settings";

/**
 * The name new playlists start with. "#N" becomes the next free number.
 * Saved on Enter or when the field loses focus; empty goes back to the
 * default.
 */
export function PlaylistNameSetting({ ids }: { ids: { title: string; desc: string } }) {
  const saved = useSettings((s) => s.settings?.playlistNewName);
  const [value, setValue] = useState(saved ?? "");
  useEffect(() => {
    if (saved !== undefined) setValue(saved);
  }, [saved]);
  if (saved === undefined) return <Skeleton width={220} height={32} />;
  const save = () => {
    const next = value.trim() || DEFAULT_SETTINGS.playlistNewName;
    setValue(next);
    if (next !== saved) void updateSettings({ playlistNewName: next });
  };
  return (
    <input
      className="setting-text"
      value={value}
      maxLength={100}
      spellCheck={false}
      aria-labelledby={ids.title}
      aria-describedby={ids.desc}
      onChange={(e) => setValue(e.target.value)}
      onBlur={save}
      onKeyDown={(e) => {
        if (e.key === "Enter") {
          e.preventDefault();
          save();
        }
      }}
    />
  );
}
