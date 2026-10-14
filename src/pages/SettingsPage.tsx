import { useId } from "react";
import { Skeleton } from "../components/Skeleton";
import { Switch } from "../components/Switch";
import { modKey } from "../lib/platform";
import { updateSettings, useSettings } from "../stores/settings";

function ToggleRow({
  title,
  description,
  checked,
  onChange,
}: {
  title: string;
  description: string;
  checked: boolean | undefined;
  onChange: (v: boolean) => void;
}) {
  const id = useId();
  return (
    <div className="setting-row">
      <div className="setting-row__text">
        <div id={`${id}-t`} className="setting-row__title">
          {title}
        </div>
        <p id={`${id}-d`} className="setting-row__desc">
          {description}
        </p>
      </div>
      {checked === undefined ? (
        <Skeleton width={40} height={24} radius="pill" />
      ) : (
        <Switch checked={checked} onChange={onChange} labelledBy={`${id}-t`} describedBy={`${id}-d`} />
      )}
    </div>
  );
}

/** Settings (opened from the header or with Ctrl/Cmd+,). */
export function SettingsPage() {
  const s = useSettings((x) => x.settings);
  return (
    <div className="page-pad settings">
      <h1 className="page-title">Settings</h1>
      <section className="settings__group" aria-labelledby="settings-listening">
        <h2 id="settings-listening" className="settings__heading">
          Listening
        </h2>
        <ToggleRow
          title="Private session"
          description="Plays aren't saved to your listening history on this device. Plex still counts them unless you turn that off below."
          checked={s?.privateSession}
          onChange={(v) => updateSettings({ privateSession: v })}
        />
        <ToggleRow
          title="Report plays to Plex"
          description="Lets Plex count plays and show what's playing in your other Plex apps."
          checked={s?.reportToPlex}
          onChange={(v) => updateSettings({ reportToPlex: v })}
        />
      </section>
      <p className="settings__hint">Shortcut: {modKey}+,</p>
    </div>
  );
}
