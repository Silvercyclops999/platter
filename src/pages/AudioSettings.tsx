/**
 * Settings controls for Transitions, Loudness and the Equalizer: the
 * crossfade length, the preamp, and the EQ editor (a 10-point curve with
 * presets, saved profiles and per-output profiles). Changes are sent while
 * dragging so they're heard at once; the core ramps them.
 */
import { useEffect, useMemo, useRef, useState, type KeyboardEvent, type PointerEvent } from "react";
import { Button } from "../components/Button";
import { Dialog } from "../components/Dialog";
import { Select, type SelectOption } from "../components/Select";
import { Skeleton } from "../components/Skeleton";
import { Slider } from "../components/Slider";
import { Switch } from "../components/Switch";
import { backend } from "../lib/backend";
import { EQ_FREQS, EQ_PRESETS, type AppError, type Settings } from "../lib/types";
import { loadOutput, useOutput } from "../stores/output";
import { updateSettings, useSettings } from "../stores/settings";
import { toast } from "../stores/toasts";
import { usePhoneForm } from "../phone/form";
import { phoneText } from "./settingsMeta";
import "./audio-settings.css";
import { failText } from "../lib/errorText";

type Ids = { title: string; desc: string };

const EQ_MIN = -12;
const EQ_MAX = 12;

/** Calls `send` at most every `ms` while values keep coming, and once more with the last one. */
function useThrottle<T>(send: (v: T) => void, ms = 80) {
  const last = useRef(0);
  const pending = useRef<{ v: T } | null>(null);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const sendRef = useRef(send);
  sendRef.current = send;
  useEffect(() => () => void (timer.current && clearTimeout(timer.current)), []);
  return (v: T) => {
    pending.current = { v };
    const now = Date.now();
    const fire = () => {
      timer.current = null;
      last.current = Date.now();
      const p = pending.current;
      pending.current = null;
      if (p) sendRef.current(p.v);
    };
    if (now - last.current >= ms) fire();
    else if (!timer.current) timer.current = setTimeout(fire, ms - (now - last.current));
  };
}

const fmtDb = (v: number) => `${v > 0 ? "+" : v < 0 ? "−" : ""}${Math.abs(v).toFixed(Math.abs(v) % 1 ? 1 : 0)} dB`;
const fmtFreq = (f: number) => (f >= 1000 ? `${f / 1000}k` : `${f}`);
const freqName = (f: number) => (f >= 1000 ? `${f / 1000} kHz` : `${f} Hz`);

// --------------------------------------------------------------- transitions

const CROSSFADE: SelectOption<Settings["crossfade"]>[] = [
  { value: "smart", label: "Smart" },
  { value: "fixed", label: "Fixed length" },
  { value: "off", label: "Off" },
];

/** Crossfade mode, plus the length when it's fixed. */
export function CrossfadeControl({ ids }: { ids: Ids }) {
  const s = useSettings((x) => x.settings);
  const [secs, setSecs] = useState<number | null>(null);
  if (!s) return <Skeleton width={120} height={32} radius="pill" />;
  const shown = secs ?? s.crossfadeSeconds;
  return (
    <>
      {s.crossfade === "fixed" && (
        <div className="audio-slider">
          <Slider
            label="Crossfade length"
            className="audio-slider__track"
            value={shown}
            min={1}
            max={12}
            step={1}
            pageStep={3}
            valueText={(v) => `${v} seconds`}
            onChange={(v) => setSecs(Math.round(v))}
            onCommit={(v) => {
              setSecs(null);
              void updateSettings({ crossfadeSeconds: Math.round(v) });
            }}
          />
          <span className="audio-slider__value">{shown} s</span>
        </div>
      )}
      <Select
        value={s.crossfade}
        options={CROSSFADE}
        onChange={(v) => void updateSettings({ crossfade: v })}
        labelledBy={ids.title}
        describedBy={ids.desc}
      />
    </>
  );
}

// ------------------------------------------------------------------ loudness

/** Preamp, -12 to +12 dB in half steps, heard while dragging. */
export function PreampControl({ ids }: { ids: Ids }) {
  const saved = useSettings((x) => x.settings?.preampDb);
  const [local, setLocal] = useState<number | null>(null);
  const send = useThrottle((v: number) => void updateSettings({ preampDb: v }));
  if (saved === undefined) return <Skeleton width={180} height={24} radius="pill" />;
  const v = local ?? saved;
  return (
    <div className="audio-slider" aria-describedby={ids.desc}>
      <Slider
        label="Volume before the equalizer"
        className="audio-slider__track audio-slider__track--wide"
        value={v}
        min={-12}
        max={12}
        step={0.5}
        pageStep={2}
        valueText={fmtDb}
        onChange={(x) => {
          const r = Math.round(x * 2) / 2;
          setLocal(r);
          send(r);
        }}
        onCommit={(x) => {
          const r = Math.round(x * 2) / 2;
          setLocal(null);
          void updateSettings({ preampDb: r });
        }}
      />
      <span className="audio-slider__value">{fmtDb(v)}</span>
    </div>
  );
}

// ---------------------------------------------------------------------- EQ

const W = 640;
const H = 220;
const PAD = { l: 44, r: 20, t: 18, b: 30 };
const x = (i: number) => PAD.l + (i * (W - PAD.l - PAD.r)) / (EQ_FREQS.length - 1);
const y = (db: number) => PAD.t + ((EQ_MAX - db) * (H - PAD.t - PAD.b)) / (EQ_MAX - EQ_MIN);
const clampDb = (v: number) => Math.min(EQ_MAX, Math.max(EQ_MIN, Math.round(v * 2) / 2));

/** A smooth line through the points (Catmull-Rom as cubic Béziers). */
function curve(pts: [number, number][]): string {
  let d = `M${pts[0][0]},${pts[0][1]}`;
  for (let i = 0; i < pts.length - 1; i++) {
    const p0 = pts[i - 1] ?? pts[i];
    const [p1, p2] = [pts[i], pts[i + 1]];
    const p3 = pts[i + 2] ?? p2;
    const c1 = [p1[0] + (p2[0] - p0[0]) / 6, p1[1] + (p2[1] - p0[1]) / 6];
    const c2 = [p2[0] - (p3[0] - p1[0]) / 6, p2[1] - (p3[1] - p1[1]) / 6];
    d += ` C${c1[0]},${c1[1]} ${c2[0]},${c2[1]} ${p2[0]},${p2[1]}`;
  }
  return d;
}

interface GraphProps {
  bands: number[];
  disabled: boolean;
  onChange: (bands: number[]) => void;
  onCommit: (bands: number[]) => void;
}

/** The 10-band curve. Drag a point (or anywhere in its column); arrows move the focused band 0.5 dB, Shift 2 dB. */
export function EqGraph({ bands, disabled, onChange, onCommit }: GraphProps) {
  const svg = useRef<SVGSVGElement>(null);
  const points = useRef<(SVGGElement | null)[]>([]);
  const [drag, setDrag] = useState<number | null>(null);
  const [focus, setFocus] = useState<number | null>(null);
  const latest = useRef(bands);
  latest.current = bands;

  const dbAt = (clientY: number) => {
    const r = svg.current!.getBoundingClientRect();
    const vy = ((clientY - r.top) / r.height) * H;
    return clampDb(EQ_MAX - ((vy - PAD.t) * (EQ_MAX - EQ_MIN)) / (H - PAD.t - PAD.b));
  };
  const bandAt = (clientX: number) => {
    const r = svg.current!.getBoundingClientRect();
    const vx = ((clientX - r.left) / r.width) * W;
    let best = 0;
    for (let i = 1; i < EQ_FREQS.length; i++) if (Math.abs(x(i) - vx) < Math.abs(x(best) - vx)) best = i;
    return best;
  };
  const setBand = (i: number, db: number, commit: boolean) => {
    if (latest.current[i] === db && !commit) return;
    const next = latest.current.map((v, j) => (j === i ? db : v));
    latest.current = next;
    (commit ? onCommit : onChange)(next);
  };

  const down = (e: PointerEvent<SVGSVGElement>) => {
    if (disabled || e.button !== 0) return;
    const i = bandAt(e.clientX);
    e.currentTarget.setPointerCapture(e.pointerId);
    setDrag(i);
    points.current[i]?.focus();
    setBand(i, dbAt(e.clientY), false);
    e.preventDefault();
  };
  const move = (e: PointerEvent<SVGSVGElement>) => {
    if (drag !== null) setBand(drag, dbAt(e.clientY), false);
  };
  const up = () => {
    if (drag === null) return;
    setBand(drag, latest.current[drag], true);
    setDrag(null);
  };
  const key = (i: number) => (e: KeyboardEvent<SVGGElement>) => {
    const step = e.shiftKey ? 2 : 0.5;
    const v = latest.current[i];
    let next: number | null = null;
    if (e.key === "ArrowUp") next = v + step;
    else if (e.key === "ArrowDown") next = v - step;
    else if (e.key === "PageUp") next = v + 2;
    else if (e.key === "PageDown") next = v - 2;
    else if (e.key === "Home") next = 0;
    else if (e.key === "ArrowLeft" || e.key === "ArrowRight") {
      const j = i + (e.key === "ArrowLeft" ? -1 : 1);
      if (j >= 0 && j < EQ_FREQS.length) points.current[j]?.focus();
      e.preventDefault();
      return;
    }
    if (next === null) return;
    e.preventDefault();
    setBand(i, clampDb(next), true);
  };

  const pts = bands.map((b, i) => [x(i), y(b)] as [number, number]);
  const line = curve(pts);
  const zero = y(0);
  const active = drag ?? focus;
  return (
    <svg
      ref={svg}
      className={`eq-graph${disabled ? " is-disabled" : ""}${drag !== null ? " is-dragging" : ""}`}
      viewBox={`0 0 ${W} ${H}`}
      role="group"
      aria-label="Equalizer bands"
      onPointerDown={down}
      onPointerMove={move}
      onPointerUp={up}
      onPointerCancel={up}
    >
      {[12, 6, 0, -6, -12].map((db) => (
        <g key={db}>
          <line className={`eq-graph__grid${db === 0 ? " is-zero" : ""}`} x1={PAD.l} x2={W - PAD.r} y1={y(db)} y2={y(db)} />
          <text className="eq-graph__axis" x={PAD.l - 10} y={y(db)} textAnchor="end" dominantBaseline="middle">
            {db > 0 ? `+${db}` : db}
          </text>
        </g>
      ))}
      {EQ_FREQS.map((f, i) => (
        <text key={f} className="eq-graph__axis" x={x(i)} y={H - 8} textAnchor="middle">
          {fmtFreq(f)}
        </text>
      ))}
      <path className="eq-graph__fill" d={`${line} L${x(EQ_FREQS.length - 1)},${zero} L${x(0)},${zero} Z`} />
      <path className="eq-graph__line" d={line} />
      {bands.map((b, i) => (
        <g
          key={EQ_FREQS[i]}
          ref={(el) => {
            points.current[i] = el;
          }}
          className={`eq-graph__point${active === i ? " is-active" : ""}`}
          role="slider"
          tabIndex={disabled ? -1 : 0}
          aria-label={freqName(EQ_FREQS[i])}
          aria-valuemin={EQ_MIN}
          aria-valuemax={EQ_MAX}
          aria-valuenow={b}
          aria-valuetext={fmtDb(b)}
          aria-orientation="vertical"
          aria-disabled={disabled || undefined}
          onKeyDown={key(i)}
          onFocus={() => setFocus(i)}
          onBlur={() => setFocus((f) => (f === i ? null : f))}
        >
          <circle className="eq-graph__ring" cx={x(i)} cy={y(b)} r={11} />
          <circle className="eq-graph__dot" cx={x(i)} cy={y(b)} r={6} />
          {active === i && (
            <text className="eq-graph__value" x={x(i)} y={y(b) > 40 ? y(b) - 20 : y(b) + 28} textAnchor="middle">
              {fmtDb(b)}
            </text>
          )}
        </g>
      ))}
    </svg>
  );
}

type NameDialog = { kind: "save" | "rename"; from?: string } | null;

function bandsFor(s: Settings, id: string): number[] | null {
  if (id.startsWith("profile:")) return s.eqProfiles.find((p) => p.name === id.slice("profile:".length))?.bands ?? null;
  return EQ_PRESETS.find((p) => p.id === id)?.bands ?? null;
}

/**
 * Picks a preset, or sets the curve. Sends the preset and the curve
 * together, the way the core will have them (moving a band makes it custom),
 * so the page shows the result at once.
 */
function setEq(change: { preset?: string; bands?: number[] }): Promise<void> {
  const s = useSettings.getState().settings;
  if (!s) return Promise.resolve();
  if (change.preset !== undefined) {
    const b = bandsFor(s, change.preset);
    return updateSettings(b ? { eqPreset: change.preset, eqBands: [...b] } : { eqPreset: change.preset });
  }
  const bands = change.bands ?? s.eqBands;
  const cur = bandsFor(s, s.eqPreset);
  const kept = cur && cur.every((v, i) => v === bands[i]);
  return updateSettings({ eqBands: bands, eqPreset: kept ? s.eqPreset : "custom" });
}

/** The EQ editor: presets and profiles, the curve, and per-output profiles. */
export function EqEditor() {
  const s = useSettings((x) => x.settings);
  const out = useOutput((o) => o.state);
  const [local, setLocal] = useState<number[] | null>(null);
  const [dialog, setDialog] = useState<NameDialog>(null);
  const [confirmDelete, setConfirmDelete] = useState<string | null>(null);
  const send = useThrottle((bands: number[]) => void setEq({ bands }));
  useEffect(() => {
    if (!out) void loadOutput();
  }, [out]);

  const options = useMemo<SelectOption<string>[]>(() => {
    const list: SelectOption<string>[] = EQ_PRESETS.map((p) => ({ value: p.id, label: p.name }));
    for (const p of s?.eqProfiles ?? []) list.push({ value: `profile:${p.name}`, label: p.name });
    if (s?.eqPreset === "custom") list.push({ value: "custom", label: "Custom" });
    return list;
  }, [s?.eqProfiles, s?.eqPreset]);

  const phone = usePhoneForm((x) => x.phone);
  if (!s) return <Skeleton height={260} radius="card" />;
  const bands = local ?? s.eqBands;
  const profile = s.eqPreset.startsWith("profile:") ? s.eqPreset.slice("profile:".length) : null;
  const current = out?.current ?? null;
  // A phone names its own output in its own words ("Default Device" is the computer's).
  const device = current && phone ? { ...current, name: phoneText(current.name) } : current;
  const binding = device ? s.eqOutputs.find((o) => o.deviceId === device.id) : undefined;
  const label = (id: string) => options.find((o) => o.value === id)?.label ?? EQ_PRESETS.find((p) => p.id === id)?.name ?? id;
  const others = s.eqOutputs.filter((o) => o.deviceId !== device?.id);
  const off = !s.eqEnabled;

  const fail = (what: string) => (e: unknown) => toast.error(failText(what, e));
  const apply = (p: Promise<Settings>) => p.then((next) => useSettings.getState().setLocal(next));
  const bind = (on: boolean) => {
    void apply(backend.eqUseForOutput(on)).catch(fail(on ? "Couldn't keep this EQ for this output" : "Couldn't stop using this EQ for this output"));
  };

  return (
    <div className={`eq-editor${off ? " eq-editor--off" : ""}`}>
      <div className="eq-editor__bar">
        <Select
          value={s.eqPreset}
          options={options}
          label="Preset"
          disabled={off}
          onChange={(v) => {
            setLocal(null);
            void setEq({ preset: v });
          }}
        />
        <Button size="sm" disabled={off} onClick={() => setDialog({ kind: "save" })}>
          Save as
        </Button>
        {profile && (
          <>
            <Button size="sm" disabled={off} onClick={() => setDialog({ kind: "rename", from: profile })}>
              Rename
            </Button>
            <Button size="sm" disabled={off} onClick={() => setConfirmDelete(profile)}>
              Delete
            </Button>
          </>
        )}
        <Button size="sm" variant="ghost" disabled={off || s.eqPreset === "flat"} onClick={() => void setEq({ preset: "flat" })}>
          Reset
        </Button>
      </div>
      <EqGraph
        bands={bands}
        disabled={off}
        onChange={(b) => {
          setLocal(b);
          send(b);
        }}
        onCommit={(b) => {
          setLocal(null);
          void setEq({ bands: b });
        }}
      />
      {off && <p className="eq-editor__note">Turn on the equalizer to change the curve.</p>}
      <div className="eq-editor__output">
        <div className="eq-editor__output-text">
          <div className="eq-editor__output-title" id="eq-output-title">
            Use for this output
          </div>
          <p className="eq-editor__output-desc" id="eq-output-desc">
            {device
              ? binding
                ? `${device.name} always plays with ${label(binding.preset)}.`
                : s.eqPreset === "custom"
                  ? `Save this curve as a profile to give it to ${device.name}.`
                  : `Always play ${device.name} with ${label(s.eqPreset)}. Other outputs keep their own choice.`
              : "No output device is in use."}
            {others.length > 0 && ` ${others.map((o) => `${o.deviceName} uses ${label(o.preset)}.`).join(" ")}`}
          </p>
        </div>
        <Switch
          checked={!!binding}
          disabled={off || !device || (!binding && s.eqPreset === "custom")}
          onChange={bind}
          labelledBy="eq-output-title"
          describedBy="eq-output-desc"
        />
      </div>
      <ProfileNameDialog
        state={dialog}
        onClose={() => setDialog(null)}
        onSave={(name) =>
          apply(dialog?.kind === "rename" && dialog.from ? backend.eqProfileRename(dialog.from, name) : backend.eqProfileSave(name)).then(() => {
            setLocal(null);
            setDialog(null);
          })
        }
        initial={dialog?.kind === "rename" ? (dialog.from ?? "") : (profile ?? "")}
      />
      <Dialog open={confirmDelete !== null} onClose={() => setConfirmDelete(null)} label="Delete profile" className="confirm">
        <h2 className="confirm__title">Delete {confirmDelete}?</h2>
        <p className="confirm__body">The curve stays as it is until you change it. Outputs that used this profile go back to your usual choice.</p>
        <div className="confirm__actions">
          <Button variant="ghost" onClick={() => setConfirmDelete(null)}>
            Cancel
          </Button>
          <Button
            variant="primary"
            onClick={() => {
              const name = confirmDelete!;
              setConfirmDelete(null);
              void apply(backend.eqProfileDelete(name)).catch(fail("Couldn't delete the profile"));
            }}
          >
            Delete
          </Button>
        </div>
      </Dialog>
    </div>
  );
}

function ProfileNameDialog({
  state,
  initial,
  onClose,
  onSave,
}: {
  state: NameDialog;
  initial: string;
  onClose: () => void;
  onSave: (name: string) => Promise<void>;
}) {
  const [name, setName] = useState("");
  const [err, setErr] = useState<string | null>(null);
  const input = useRef<HTMLInputElement>(null);
  useEffect(() => {
    if (state) {
      setName(initial);
      setErr(null);
    }
  }, [state, initial]);
  const title = state?.kind === "rename" ? "Rename profile" : "Save as profile";
  const submit = () => {
    if (!name.trim()) {
      setErr("Give the profile a name.");
      return;
    }
    onSave(name.trim()).catch((e: AppError) => setErr(e?.message || "Couldn't save the profile"));
  };
  return (
    <Dialog open={state !== null} onClose={onClose} label={title} className="confirm" initialFocus={input}>
      <h2 className="confirm__title">{title}</h2>
      <form
        onSubmit={(e) => {
          e.preventDefault();
          submit();
        }}
      >
        <input
          ref={input}
          className="setting-text eq-editor__name"
          value={name}
          maxLength={40}
          spellCheck={false}
          aria-label="Profile name"
          placeholder="Headphones"
          onChange={(e) => setName(e.target.value)}
        />
        {err && (
          <p className="confirm__error" role="alert">
            {err}
          </p>
        )}
        <div className="confirm__actions">
          <Button variant="ghost" onClick={onClose}>
            Cancel
          </Button>
          <Button variant="primary" type="submit">
            Save
          </Button>
        </div>
      </form>
    </Dialog>
  );
}
