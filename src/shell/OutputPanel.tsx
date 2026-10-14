import { Laptop, Speaker, X } from "lucide-react";
import { useEffect } from "react";
import { IconButton } from "../components/Button";
import { Skeleton } from "../components/Skeleton";
import type { OutputDevice } from "../lib/types";
import { useApp } from "../stores/app";
import { loadOutput, selectOutput, useOutput } from "../stores/output";
import { usePlayer } from "../stores/player";
import { useSettings } from "../stores/settings";
import { useUi } from "../stores/ui";
import { ListenOn, RemoteCurrent } from "./ListenOn";
import { streamQuality } from "./streamQuality";
import "./output.css";

/** How often the list refreshes while the picker is open (devices plugged in). */
export const OUTPUT_REFRESH_MS = 5000;

function CurrentDevice({ device, followsDefault }: { device: OutputDevice; followsDefault: boolean }) {
  const track = usePlayer((s) => s.state?.track);
  const settings = useSettings((s) => s.settings);
  const connection = useApp((s) => s.health?.connection ?? s.auth?.server?.connection ?? null);
  const quality = track ? streamQuality(track, connection, settings) : null;
  return (
    <div className="output-current" aria-labelledby="output-current-label" role="group">
      <span className="output-current__icon" aria-hidden="true">
        <Laptop size={28} />
      </span>
      <div className="output-current__text">
        <div id="output-current-label" className="output-current__label">
          {followsDefault ? "Current device, system default" : "Current device"}
        </div>
        <div className="output-current__name" title={device.name}>
          {device.name}
        </div>
        <div className="output-current__sub">
          {["This computer", quality].filter(Boolean).join(" • ")}
        </div>
      </div>
    </div>
  );
}

function DeviceRow({ name, sub, onPick, busy }: { name: string; sub?: string; onPick: () => void; busy: boolean }) {
  return (
    <li>
      <button type="button" className="output-row" aria-label={`Play on ${name}`} aria-busy={busy || undefined} onClick={onPick}>
        <span className="output-row__icon" aria-hidden="true">
          <Speaker size={20} />
        </span>
        <span className="output-row__text">
          <span className="output-row__name truncate">{name}</span>
          {(busy || sub) && <span className="output-row__sub truncate">{busy ? "Switching…" : sub}</span>}
        </span>
      </button>
    </li>
  );
}

/** Right panel: the output in use, and the computer's other outputs to switch to. */
export function OutputPanel() {
  const state = useOutput((s) => s.state);
  const switching = useOutput((s) => s.switching);
  const remote = usePlayer((s) => s.remote);
  const close = () => useUi.getState().setOutputOpen(false);

  useEffect(() => {
    void loadOutput();
    const t = setInterval(() => void loadOutput(), OUTPUT_REFRESH_MS);
    return () => clearInterval(t);
  }, []);

  const current = state?.current ?? null;
  const followsDefault = state?.selected == null;
  const systemDefault = state?.devices.find((d) => d.isDefault) ?? null;
  const others = state ? state.devices.filter((d) => d.id !== current?.id) : [];

  return (
    <aside className="output-panel" aria-label="Connect to a device">
      <header className="output-panel__head">
        <h2 className="output-panel__title">Connect</h2>
        <IconButton label="Close" tooltip="Close" icon={<X size={18} />} onClick={close} />
      </header>
      <div className="output-panel__body">
        {!state ? (
          <>
            <Skeleton height={96} radius="card" />
            <Skeleton width="60%" height={16} style={{ marginTop: 32 }} />
            <Skeleton height={48} radius="card" style={{ marginTop: 12 }} />
          </>
        ) : (
          <>
            {remote ? (
              <RemoteCurrent session={remote} />
            ) : current ? (
              <CurrentDevice device={current} followsDefault={followsDefault} />
            ) : (
              <p className="output-empty">No audio output found on this computer.</p>
            )}
            <section className="output-section" aria-labelledby="output-others">
              <h3 id="output-others" className="output-section__title">
                Select another output
              </h3>
              {others.length === 0 && followsDefault ? (
                <p className="output-empty">No other outputs found on this computer.</p>
              ) : (
                <ul className="output-list">
                  {!followsDefault && (
                    <DeviceRow
                      name="System default"
                      sub={systemDefault ? `Follows your computer, now ${systemDefault.name}` : "Follows your computer's setting"}
                      busy={switching === null}
                      onPick={() => void selectOutput(null)}
                    />
                  )}
                  {others.map((d) => (
                    <DeviceRow
                      key={d.id}
                      name={d.name}
                      sub={d.isDefault ? "Your computer's default" : undefined}
                      busy={switching === d.id}
                      onPick={() => void selectOutput(d.id)}
                    />
                  ))}
                </ul>
              )}
            </section>
            <ListenOn />
          </>
        )}
      </div>
    </aside>
  );
}
