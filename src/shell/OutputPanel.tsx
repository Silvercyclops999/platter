import { Check, ChevronDown, Laptop, Speaker, X } from "lucide-react";
import { useEffect, useState } from "react";
import { IconButton } from "../components/Button";
import { Skeleton } from "../components/Skeleton";
import type { OutputDevice } from "../lib/types";
import { useApp } from "../stores/app";
import { loadOutput, outputChoices, selectOutput, useOutput } from "../stores/output";
import { usePlayer } from "../stores/player";
import { useSettings } from "../stores/settings";
import { useUi } from "../stores/ui";
import { ListenOn, RemoteCurrent } from "./ListenOn";
import { ResumeFrom } from "./ResumeFrom";
import { SessionCard } from "./ListenStrip";
import { HifiLine } from "./HifiMark";
import { streamQuality } from "./streamQuality";
import "./output.css";

/** How often the list refreshes while the picker is open (devices plugged in). */
export const OUTPUT_REFRESH_MS = 5000;

/**
 * Splits a system device name like "Headphones (4- Brand Model)" into the
 * endpoint name and the hardware behind it, so long driver names don't crowd
 * the list. Names without a trailing "(...)" come back whole.
 */
export function splitDeviceName(name: string): { name: string; detail?: string } {
  const m = /^(.*?)\s*\(([^()]*)\)\s*$/.exec(name);
  if (!m || !m[1].trim()) return { name };
  const detail = m[2].replace(/^\d+\s*-\s*/, "").trim();
  return { name: m[1].trim(), detail: detail || undefined };
}

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
        <div className="output-current__name truncate" title={device.name}>
          {splitDeviceName(device.name).name}
        </div>
        <div className="output-current__sub truncate" title={device.name}>
          {["This computer", splitDeviceName(device.name).detail, quality].filter(Boolean).join(" • ")}
        </div>
        <HifiLine />
      </div>
    </div>
  );
}

function DeviceRow({ name, title, sub, onPick, busy, selected }: { name: string; title?: string; sub?: string; onPick: () => void; busy: boolean; selected: boolean }) {
  const parts = splitDeviceName(name);
  const line = busy ? "Switching…" : [parts.detail, sub].filter(Boolean).join(" • ");
  return (
    <li>
      <button
        type="button"
        className={`output-row output-row--compact ${selected ? "is-selected" : ""}`}
        aria-label={`Play on ${title ?? name}`}
        aria-current={selected ? "true" : undefined}
        title={title ?? name}
        aria-busy={busy || undefined}
        onClick={onPick}
      >
        <span className="output-row__icon" aria-hidden="true">
          <Speaker size={18} />
        </span>
        <span className="output-row__text">
          <span className="output-row__name output-row__name--one truncate">{parts.name}</span>
          {line && <span className="output-row__sub output-row__sub--one truncate">{line}</span>}
        </span>
        {selected && (
          <span className="output-row__check" aria-hidden="true">
            <Check size={18} />
          </span>
        )}
      </button>
    </li>
  );
}

/** Right panel: the output in use, and the computer's outputs to switch to. */
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
  const choices = state ? outputChoices(state) : [];
  const [outputsOpen, setOutputsOpen] = useState(false);

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
            <ListenOn />
            <ResumeFrom />
            <section className="output-section" aria-labelledby="output-others">
              <h3 id="output-others" className="output-section__title">
                This computer
              </h3>
              <button
                type="button"
                className="output-disclosure"
                aria-expanded={outputsOpen}
                aria-controls="output-others-list"
                onClick={() => setOutputsOpen((o) => !o)}
              >
                <span className="truncate">
                  Sound output: {current ? splitDeviceName(current.name).name : "none"}
                </span>
                <ChevronDown size={16} aria-hidden="true" className="output-disclosure__chev" />
              </button>
              {outputsOpen && (
                <ul className="output-list" id="output-others-list">
                  {choices.map((c) =>
                    c.id === null ? (
                      <DeviceRow
                        key="system-default"
                        name="System default"
                        title={c.label}
                        sub={c.defaultName ? splitDeviceName(c.defaultName).name : "Follows your computer's setting"}
                        selected={c.selected}
                        busy={switching === null}
                        onPick={() => void selectOutput(null)}
                      />
                    ) : (
                      <DeviceRow
                        key={c.id}
                        name={c.label}
                        sub={c.isDefault ? "Your computer's default" : undefined}
                        selected={c.selected}
                        busy={switching === c.id}
                        onPick={() => void selectOutput(c.id)}
                      />
                    ),
                  )}
                </ul>
              )}
            </section>
            {!remote && <SessionCard />}
          </>
        )}
      </div>
    </aside>
  );
}
