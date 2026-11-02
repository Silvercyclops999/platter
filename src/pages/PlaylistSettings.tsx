import { useEffect, useState } from "react";
import { Button } from "../components/Button";
import { Skeleton } from "../components/Skeleton";
import { timeAgo } from "../lib/relativeTime";
import { DEFAULT_SETTINGS } from "../lib/types";
import { bringBack, useDeletedPlaylists } from "../stores/playlists";
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

/** Deleted playlists still kept, each with Bring back (Settings, Library). */
export function DeletedPlaylists() {
  const list = useDeletedPlaylists((s) => s.list) ?? [];
  const [busy, setBusy] = useState<number | null>(null);
  return (
    <ul className="dismissed-list" aria-label="Deleted playlists">
      {list.map((k) => (
        <li key={k.id} className="dismissed-list__item">
          <span className="dismissed-list__name">
            {k.title}
            <span className="setting-row__note">
              {" "}
              {k.trackIds.length === 1 ? "1 song" : `${k.trackIds.length} songs`}, {k.gone ? "gone from your server" : "deleted"}{" "}
              {timeAgo(k.deletedAt).toLowerCase()}
            </span>
          </span>
          <Button
            size="sm"
            variant="ghost"
            disabled={busy !== null}
            onClick={() => {
              setBusy(k.id);
              void bringBack(k.id).finally(() => setBusy(null));
            }}
            aria-label={`Bring back ${k.title}`}
          >
            Bring back
          </Button>
        </li>
      ))}
    </ul>
  );
}
