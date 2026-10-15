import { useEffect, useState } from "react";
import { Skeleton } from "../components/Skeleton";
import { updateSettings, useSettings } from "../stores/settings";

/** "Invites expire after" choices (hours). */
export const INVITE_HOURS = [
  { value: "1", label: "1 hour" },
  { value: "24", label: "24 hours" },
  { value: "168", label: "7 days" },
] as const;

/**
 * A text setting saved on blur or Enter: the listen along relay or the
 * name others see. Empty means the default (ntfy.sh, or the Plex name).
 */
export function ListenTextSetting({
  field,
  placeholder,
  maxLength,
  ids,
}: {
  field: "listenRelay" | "listenName";
  placeholder: string;
  maxLength: number;
  ids: { title: string; desc: string };
}) {
  const saved = useSettings((s) => s.settings?.[field]);
  const [value, setValue] = useState(saved ?? "");
  useEffect(() => {
    if (saved !== undefined) setValue(saved);
  }, [saved]);
  if (saved === undefined) return <Skeleton width={220} height={32} />;
  const save = () => {
    const next = value.trim();
    setValue(next);
    if (next !== saved) void updateSettings({ [field]: next }).then(undefined, () => setValue(saved));
  };
  return (
    <input
      className="setting-text"
      value={value}
      maxLength={maxLength}
      placeholder={placeholder}
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
