import { useEffect } from "react";
import { Select, type SelectOption } from "../components/Select";
import { Skeleton } from "../components/Skeleton";
import { loadOutput, selectOutput, useOutput } from "../stores/output";

/** Select value for "follow the system default". Device ids never look like this. */
export const SYSTEM_DEFAULT = "__system-default__";

/** Settings → Playback → Output device. Same choice as the picker in the right panel. */
export function OutputDeviceSelect({ ids }: { ids: { title: string; desc: string } }) {
  const state = useOutput((s) => s.state);
  useEffect(() => {
    void loadOutput();
  }, []);
  if (!state) return <Skeleton width={160} height={32} radius="pill" />;
  const def = state.devices.find((d) => d.isDefault);
  const options: SelectOption<string>[] = [
    { value: SYSTEM_DEFAULT, label: def ? `System default (${def.name})` : "System default" },
    ...state.devices.map((d) => ({ value: d.id, label: d.name })),
  ];
  // A chosen device that's unplugged still shows until another is picked.
  if (state.selected && !state.devices.some((d) => d.id === state.selected)) {
    options.push({ value: state.selected, label: "Disconnected device" });
  }
  return (
    <Select
      value={state.selected ?? SYSTEM_DEFAULT}
      options={options}
      labelledBy={ids.title}
      describedBy={ids.desc}
      onChange={(v) => void selectOutput(v === SYSTEM_DEFAULT ? null : v)}
    />
  );
}
