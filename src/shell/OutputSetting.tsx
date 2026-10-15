import { useEffect } from "react";
import { Select, type SelectOption } from "../components/Select";
import { Skeleton } from "../components/Skeleton";
import { loadOutput, outputChoices, selectOutput, useOutput } from "../stores/output";

/** Select value for "follow the system default". Device ids never look like this. */
export const SYSTEM_DEFAULT = "__system-default__";

/** Settings → Playback → Output device. Same list as the picker in the right panel. */
export function OutputDeviceSelect({ ids }: { ids: { title: string; desc: string } }) {
  const state = useOutput((s) => s.state);
  useEffect(() => {
    void loadOutput();
  }, []);
  if (!state) return <Skeleton width={160} height={32} radius="pill" />;
  const options: SelectOption<string>[] = outputChoices(state).map((c) => ({ value: c.id ?? SYSTEM_DEFAULT, label: c.label }));
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
