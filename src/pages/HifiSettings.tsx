/**
 * Settings rows for headphone correction and bit-perfect output, both per
 * output device. Headphone profiles come from the AutoEq project: the core
 * searches its bundled index and fetches a profile when one is picked.
 */
import { Check, Headphones } from "lucide-react";
import { useEffect, useRef, useState, type ReactNode } from "react";
import { create } from "zustand";
import { Button } from "../components/Button";
import { SearchField } from "../components/SearchField";
import { Skeleton } from "../components/Skeleton";
import { Switch } from "../components/Switch";
import { backend } from "../lib/backend";
import { exclusiveText } from "../lib/hifi";
import type { AppError, DeviceCorrection, HeadphoneEntry, OutputDevice, Settings } from "../lib/types";
import { splitDeviceName } from "../shell/OutputPanel";
import { loadOutput, useOutput } from "../stores/output";
import { useSettings } from "../stores/settings";
import { toast } from "../stores/toasts";
import "./hifi-settings.css";

type Ids = { title: string; desc: string };

/** Same shape as the Settings page's rows. */
export interface HifiRow {
  key: string;
  section: "Sound";
  more?: string;
  title: string;
  description: string;
  note?: ReactNode;
  control: (ids: Ids) => ReactNode;
  block?: () => ReactNode;
}

/** The output device in use on this computer (null while loading or none). */
function useDevice(): OutputDevice | null | undefined {
  const state = useOutput((s) => s.state);
  useEffect(() => {
    if (!state) void loadOutput();
  }, [state]);
  if (!state) return undefined;
  const d = state.current;
  return d && !d.id.startsWith("net:") ? d : null;
}

const deviceLabel = (d: OutputDevice) => splitDeviceName(d.name).name;
const errorText = (e: unknown) => ((e as AppError)?.message ? `: ${(e as AppError).message}` : "");
const apply = (p: Promise<Settings>) => p.then((next) => useSettings.getState().setLocal(next));

/** The search shows while picking a first profile or changing one. */
const usePicking = create<{ picking: boolean; set: (v: boolean) => void }>()((set) => ({
  picking: false,
  set: (picking) => set({ picking }),
}));

function bindingFor(s: Settings | null, d: OutputDevice | null | undefined): DeviceCorrection | undefined {
  return d ? s?.headphoneCorrection?.find((c) => c.deviceId === d.id) : undefined;
}

function CorrectionSwitch({ ids }: { ids: Ids }) {
  const s = useSettings((x) => x.settings);
  const device = useDevice();
  const picking = usePicking((x) => x.picking);
  if (!s || device === undefined) return <Skeleton width={40} height={24} radius="pill" />;
  const binding = bindingFor(s, device);
  return (
    <Switch
      checked={!!binding || picking}
      disabled={!device}
      labelledBy={ids.title}
      describedBy={ids.desc}
      onChange={(on) => {
        if (on) {
          usePicking.getState().set(true);
          return;
        }
        usePicking.getState().set(false);
        if (binding) void apply(backend.headphoneCorrectionSet(null)).catch((e) => toast.error(`Couldn't turn it off${errorText(e)}`));
      }}
    />
  );
}

function ResultRow({ h, chosen, busy, onPick }: { h: HeadphoneEntry; chosen: boolean; busy: boolean; onPick: () => void }) {
  return (
    <li>
      <button type="button" className={`hifi-result ${chosen ? "is-selected" : ""}`} aria-current={chosen || undefined} aria-busy={busy || undefined} onClick={onPick}>
        <span className="hifi-result__text">
          <span className="hifi-result__name truncate">{h.name}</span>
          <span className="hifi-result__sub truncate">{busy ? "Getting the profile…" : `${h.source} • ${h.form}`}</span>
        </span>
        {chosen && <Check size={18} aria-hidden="true" className="hifi-result__check" />}
      </button>
    </li>
  );
}

/** Under the switch: the chosen profile with A/B, and the headphone search. */
function CorrectionBlock() {
  const s = useSettings((x) => x.settings);
  const device = useDevice();
  const picking = usePicking((x) => x.picking);
  const [query, setQuery] = useState("");
  const [results, setResults] = useState<HeadphoneEntry[] | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [bypass, setBypass] = useState(false);
  const bypassRef = useRef(false);
  const binding = bindingFor(s, device);

  useEffect(() => {
    const q = query.trim();
    if (!q) {
      setResults(null);
      return;
    }
    let alive = true;
    const t = setTimeout(() => {
      backend.headphoneSearch(q).then(
        (r) => alive && setResults(r),
        () => alive && setResults([]),
      );
    }, 150);
    return () => {
      alive = false;
      clearTimeout(t);
    };
  }, [query]);

  // A/B is only for listening here: leaving turns the correction back on.
  // A search left open closes too.
  useEffect(
    () => () => {
      usePicking.getState().set(false);
      if (bypassRef.current) void backend.headphoneCorrectionCompare(false).catch(() => {});
    },
    [],
  );
  // Another device has its own profile (or none): start from it.
  const deviceId = device?.id;
  useEffect(() => {
    usePicking.getState().set(false);
    setQuery("");
  }, [deviceId]);

  const compare = (off: boolean) => {
    setBypass(off);
    bypassRef.current = off;
    void backend.headphoneCorrectionCompare(off).catch(() => {});
  };

  if (!s || device === undefined) return null;
  if (!binding && !picking) return null;
  if (!device) return <p className="hifi-note">Pick an output on this computer to use headphone correction.</p>;
  const pick = (h: HeadphoneEntry) => {
    setBusy(h.path);
    apply(backend.headphoneCorrectionSet(h.path))
      .then(() => {
        usePicking.getState().set(false);
        setQuery("");
      })
      .catch((e) => toast.error(`Couldn't get that profile${errorText(e)}`))
      .finally(() => setBusy(null));
  };
  const searching = picking || !binding;

  return (
    <div className="hifi">
      {binding && (
        <div className="hifi-profile">
          <span className="hifi-profile__icon" aria-hidden="true">
            <Headphones size={20} />
          </span>
          <div className="hifi-profile__text">
            <div className="hifi-profile__name truncate">{binding.name}</div>
            <div className="hifi-profile__sub truncate">
              {`${binding.source} • ${binding.form} • for ${deviceLabel(device)} • preamp ${binding.preampDb.toFixed(1)} dB`}
            </div>
          </div>
          <div className="hifi-ab" role="group" aria-label="Compare">
            <button type="button" className={`hifi-ab__btn ${!bypass ? "is-on" : ""}`} aria-pressed={!bypass} onClick={() => compare(false)}>
              Corrected
            </button>
            <button type="button" className={`hifi-ab__btn ${bypass ? "is-on" : ""}`} aria-pressed={bypass} onClick={() => compare(true)}>
              Original
            </button>
          </div>
          {!picking && (
            <Button size="sm" onClick={() => usePicking.getState().set(true)}>
              Change
            </Button>
          )}
        </div>
      )}
      {searching && (
        <>
          <SearchField
            className="hifi-search"
            value={query}
            onChange={setQuery}
            placeholder="Search for your headphones"
            label="Search for your headphones"
            autoFocus={picking}
          />
          {results && results.length > 0 && (
            <ul className="hifi-results" aria-label="Headphones">
              {results.slice(0, 8).map((h) => (
                <ResultRow key={h.path} h={h} chosen={binding?.path === h.path} busy={busy === h.path} onPick={() => pick(h)} />
              ))}
            </ul>
          )}
          {results && results.length === 0 && <p className="hifi-note">No headphones match "{query.trim()}".</p>}
          {binding && picking && (
            <Button size="sm" variant="ghost" onClick={() => usePicking.getState().set(false)}>
              Cancel
            </Button>
          )}
        </>
      )}
      <p className="hifi-credit">Profiles from the AutoEq project (MIT licence), fetched from GitHub when you pick one.</p>
    </div>
  );
}

function CorrectionNote() {
  const s = useSettings((x) => x.settings);
  const device = useDevice();
  const exclusive = useOutput((x) => x.state?.exclusive);
  const binding = bindingFor(s, device);
  if (exclusive?.state === "on" && binding) return <>Bypassed while bit-perfect output is on.</>;
  if (device && binding) return <>On for {deviceLabel(device)}.</>;
  return null;
}

function BitPerfectSwitch({ ids }: { ids: Ids }) {
  const s = useSettings((x) => x.settings);
  const device = useDevice();
  const [busy, setBusy] = useState(false);
  if (!s || device === undefined) return <Skeleton width={40} height={24} radius="pill" />;
  const on = !!device && (s.bitPerfectOutputs ?? []).some((b) => b.deviceId === device.id);
  return (
    <Switch
      checked={on}
      disabled={!device || busy}
      labelledBy={ids.title}
      describedBy={ids.desc}
      onChange={(v) => {
        setBusy(true);
        apply(backend.bitPerfectSet(v))
          .then(() => loadOutput())
          .catch((e) => toast.error(`Couldn't change bit-perfect output${errorText(e)}`))
          .finally(() => setBusy(false));
      }}
    />
  );
}

function BitPerfectNote() {
  const s = useSettings((x) => x.settings);
  const device = useDevice();
  const exclusive = useOutput((x) => x.state?.exclusive);
  if (!s || !device || !(s.bitPerfectOutputs ?? []).some((b) => b.deviceId === device.id)) return null;
  if (exclusive?.state === "fallback")
    return (
      <span className="hifi-warning" role="status">
        {exclusiveText(exclusive)}
      </span>
    );
  if (exclusive?.state === "on") return <>{exclusiveText(exclusive)}</>;
  return <>On for {deviceLabel(device)}. It takes the device when music plays.</>;
}

/** The two rows, for the Settings page's list. */
export function hifiRows(): HifiRow[] {
  return [
    {
      key: "headphoneCorrection",
      section: "Sound",
      title: "Headphone correction",
      description: "Evens out your headphones' sound with a measured profile. This device only.",
      more:
        "Evens out your headphones' sound with a measured profile, before your own EQ. Search for your model, then compare with the original. Each output device keeps its own. This device only.",
      note: <CorrectionNote />,
      control: (ids) => <CorrectionSwitch ids={ids} />,
      block: () => <CorrectionBlock />,
    },
    {
      key: "bitPerfectOutputs",
      section: "Sound",
      title: "Bit-perfect output",
      description: "Sends music to this output exactly as it is in the file. This device only.",
      more:
        "Sends music to this output exactly as it is in the file, at its own sample rate, with no other app's sound mixed in. While it's on, volume, the equalizer, headphone correction, loudness leveling and crossfades are bypassed, so set the level on your DAC or amplifier. If the device is busy or can't play a song's format, Platter plays shared and says why. Each output device has its own switch. This device only.",
      note: <BitPerfectNote />,
      control: (ids) => <BitPerfectSwitch ids={ids} />,
    },
  ];
}
