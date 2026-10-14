import { useEffect, useState } from "react";
import { Skeleton } from "../components/Skeleton";
import { updateSettings, useSettings } from "../stores/settings";

/** The longest name, same as the core. */
export const COMPUTER_NAME_MAX = 60;

/**
 * The name your other devices see for this computer. Empty uses the
 * computer's own name. Saved on Enter or when the field loses focus.
 */
export function ComputerNameSetting({ ids }: { ids: { title: string; desc: string } }) {
  const saved = useSettings((s) => s.settings?.computerName);
  const [value, setValue] = useState(saved ?? "");
  useEffect(() => {
    if (saved !== undefined) setValue(saved);
  }, [saved]);
  if (saved === undefined) return <Skeleton width={220} height={32} />;
  const save = () => {
    const next = value.trim();
    setValue(next);
    if (next !== saved) void updateSettings({ computerName: next });
  };
  return (
    <input
      className="setting-text"
      value={value}
      maxLength={COMPUTER_NAME_MAX}
      placeholder="This computer's name"
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
