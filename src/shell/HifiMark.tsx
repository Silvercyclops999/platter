/**
 * Shows when bit-perfect output or headphone correction is on for the
 * output in use: a small mark in the player bar, and a line in the device
 * picker.
 */
import { AudioWaveform, Headphones } from "lucide-react";
import { useEffect } from "react";
import { Tooltip } from "../components/Tooltip";
import type { Exclusive, Settings } from "../lib/types";
import { bindingFor, exclusiveText, formatRate } from "../lib/hifi";
import { loadOutput, useOutput } from "../stores/output";
import { useSettings } from "../stores/settings";
import { useUi } from "../stores/ui";
import "./hifi-mark.css";

interface Hifi {
  exclusive: Exclusive | undefined;
  /** The headphone whose correction is on, unless bit-perfect bypasses it. */
  correction: string | null;
  /** Bit-perfect is set for the device (whatever the stream managed). */
  wanted: boolean;
}

function useHifi(): Hifi | null {
  const s: Settings | null = useSettings((x) => x.settings);
  const state = useOutput((x) => x.state);
  const any = !!s && ((s.bitPerfectOutputs?.length ?? 0) > 0 || (s.headphoneCorrection?.length ?? 0) > 0);
  // Only looked up when one of them is set somewhere.
  useEffect(() => {
    if (any && !state) void loadOutput();
  }, [any, state]);
  const d = state?.current;
  if (!s || !any || !d) return null;
  const exclusive = state?.exclusive;
  const wanted = (s.bitPerfectOutputs ?? []).some((b) => b.deviceId === d.id);
  const c = bindingFor(s, d);
  const correction = c && exclusive?.state !== "on" ? c.name : null;
  if (!wanted && !correction) return null;
  return { exclusive, correction, wanted };
}

/** Player bar: an icon that opens the device picker. */
export function HifiMark() {
  const h = useHifi();
  if (!h) return null;
  const on = h.exclusive?.state === "on";
  const fallback = h.exclusive?.state === "fallback";
  let label: string;
  if (on && h.exclusive?.state === "on") label = `Bit-perfect output, ${formatRate(h.exclusive.rate)}`;
  else if (fallback) label = "Bit-perfect output is playing shared for now";
  else if (h.wanted) label = "Bit-perfect output is on for this device";
  else label = `Headphone correction: ${h.correction}`;
  const Icon = h.wanted ? AudioWaveform : Headphones;
  return (
    <Tooltip label={label}>
      <button
        type="button"
        className={`hifi-mark ${fallback ? "is-fallback" : ""} ${on ? "is-on" : ""}`}
        aria-label={`${label}. Open Connect.`}
        onClick={() => useUi.getState().toggleOutput()}
      >
        <Icon size={16} aria-hidden="true" />
        {on && h.exclusive?.state === "on" && <span className="hifi-mark__rate">{formatRate(h.exclusive.rate)}</span>}
      </button>
    </Tooltip>
  );
}

/** Device picker: what's on for the device in use, in words. */
export function HifiLine() {
  const h = useHifi();
  if (!h) return null;
  const text = exclusiveText(h.exclusive) ?? (h.wanted ? "Bit-perfect output takes the device when music plays." : null);
  return (
    <div className="hifi-line">
      {text && (
        <p className={`hifi-line__row ${h.exclusive?.state === "fallback" ? "is-fallback" : ""}`}>
          <AudioWaveform size={14} aria-hidden="true" />
          <span>{text}</span>
        </p>
      )}
      {h.correction && (
        <p className="hifi-line__row">
          <Headphones size={14} aria-hidden="true" />
          <span>Headphone correction: {h.correction}</span>
        </p>
      )}
    </div>
  );
}
