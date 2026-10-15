import { useId } from "react";
import { Switch } from "../components/Switch";
import { useServerName } from "../stores/caps";
import { useUi } from "../stores/ui";

function Row({ title, description, checked, onChange }: { title: string; description: string; checked: boolean; onChange: (v: boolean) => void }) {
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
      <Switch checked={checked} onChange={onChange} labelledBy={`${id}-t`} describedBy={`${id}-d`} />
    </div>
  );
}

/** Settings page group for the now-playing panel. Kept on this device, like panel widths. */
export function NowPlayingSettings() {
  const prefs = useUi((s) => s.nowPlayingPrefs);
  const set = useUi((s) => s.setNowPlayingPrefs);
  const server = useServerName() ?? "your server";
  return (
    <section className="settings__group" aria-labelledby="settings-now-playing">
      <h2 id="settings-now-playing" className="settings__heading">
        Now playing
      </h2>
      <Row
        title="Open now playing when a song starts"
        description="Shows the now playing view on the right when a new song starts, unless the queue is open there."
        checked={prefs.openOnStart}
        onChange={(v) => set({ openOnStart: v })}
      />
      <Row
        title="Lyrics card in now playing"
        description={`Shows the first lines of a song's lyrics in the now playing view, when ${server} has lyrics for it.`}
        checked={prefs.lyricsCard}
        onChange={(v) => set({ lyricsCard: v })}
      />
    </section>
  );
}
