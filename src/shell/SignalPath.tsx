/**
 * Signal path: what happens to the sound between the file and the output
 * device, drawn as a column of engraved plates. Opened from the badge in
 * the player bar, the More player options menu or the expanded view, and
 * live while it's open (the core sends `audio://signal-path` on changes).
 */
import { ArrowDown, X } from "lucide-react";
import { useEffect, useLayoutEffect, useRef, type KeyboardEvent } from "react";
import { createPortal } from "react-dom";
import { PlateButton } from "../components/Plate";
import { qualityLabel } from "../lib/format";
import { bindingFor, formatRate } from "../lib/hifi";
import type { SignalPath } from "../lib/types";
import { useApp } from "../stores/app";
import { useOutput } from "../stores/output";
import { usePlayer } from "../stores/player";
import { nav } from "../stores/router";
import { useSettings } from "../stores/settings";
import { closeSignal, loadSignal, openSignal, useSignal } from "../stores/signalPath";
import { streamQuality } from "./streamQuality";
import "./signal-path.css";

/** Marks elements that open the panel, so a click on one doesn't count as a click away. */
const OPENER = "data-signal-opener";

function channels(n: number | null | undefined): string | null {
  if (!n) return null;
  return n === 1 ? "mono" : n === 2 ? "stereo" : `${n} channels`;
}

function db(v: number): string {
  const r = Math.round(v * 10) / 10;
  return `${r > 0 ? "+" : r < 0 ? "−" : ""}${Math.abs(r)} dB`;
}

/**
 * The verdict line. A transcode's bitrate isn't known to the core, so the
 * stream quality the window works out (like "MP3 320") fills it in.
 */
export function verdictText(p: SignalPath, stream: string | null): string {
  if (p.verdict.kind === "lossy" && p.source?.transcoded && stream && /^MP3 \d+$/.test(stream)) return `Lossy stream (${stream})`;
  return p.verdict.text;
}

/** The short label on the badge. */
export function badgeText(p: SignalPath | null, fallback: string | null): string | null {
  if (!p || p.verdict.kind === "idle") return fallback;
  if (p.verdict.kind === "bitPerfect") return "Bit-perfect";
  return fallback;
}

interface Stage {
  name: string;
  value: string;
  detail?: string | null;
  /** Bypassed or doing nothing. */
  off?: boolean;
}

function sourceFrom(p: SignalPath): string {
  const s = p.source;
  if (!s) return "";
  if (s.transcoded) return "Converted by your server";
  if (s.from === "download") return "Downloaded to this computer";
  if (s.from === "cache") return "From the audio cache on this computer";
  return "The original file, from your server";
}

/** `tunedFor`: the headphones whose correction plays, by name. */
export function stages(p: SignalPath, stream: string | null, tunedFor?: string | null): Stage[] {
  const s = p.source;
  const out: Stage[] = [];
  if (s) {
    const fmt = [
      s.transcoded && stream && /^MP3 \d+$/.test(stream) ? stream : s.codec,
      s.bitDepth ? `${s.bitDepth}-bit` : null,
      s.sampleRate ? formatRate(s.sampleRate) : null,
      channels(s.channels),
      !s.lossless && !s.transcoded && s.bitrate ? `${s.bitrate} kbps` : null,
    ].filter(Boolean);
    out.push({ name: "Source", value: fmt.join(" · ") || "Unknown format", detail: sourceFrom(p) });
  }
  if (p.decode) out.push({ name: "Decode", value: p.decode, detail: s?.lossless ? "Every bit of the file, unchanged" : null });
  const bypassed = p.output.volumeBypassed;
  const l = p.loudness;
  out.push({
    name: "Loudness",
    value: l.on ? (l.gainDb !== null ? db(l.gainDb) : "No loudness data for this song") : bypassed ? "Bypassed" : "Off",
    detail: l.on ? { smart: "Smart leveling", track: "Track gain", album: "Album gain", off: null }[l.mode] : null,
    off: !l.on || l.gainDb === 0,
  });
  const e = p.eq;
  const eqParts = [e.correction ? (tunedFor ? `Tuned for ${tunedFor}` : "Headphone correction") : null, e.eq ? "Equalizer" : null].filter(Boolean);
  const eqDetail = [
    e.correctionPreampDb !== null && e.correction ? `correction preamp ${db(e.correctionPreampDb)}` : null,
    e.preampDb ? `preamp ${db(e.preampDb)}` : null,
    e.limiter && (e.eq || e.correction || e.preampDb > 0) ? "limiter on" : null,
  ].filter(Boolean);
  out.push({
    name: "EQ",
    value: eqParts.length ? eqParts.join(" + ") : e.preampDb ? "Preamp only" : bypassed ? "Bypassed" : "Off",
    detail: eqDetail.length ? eqDetail.join(", ") : null,
    off: !eqParts.length && !e.preampDb,
  });
  const r = p.resample;
  out.push(
    r
      ? {
          name: "Resample",
          value: r.fromRate !== r.toRate ? `${formatRate(r.fromRate)} → ${formatRate(r.toRate)}` : `${channels(r.fromChannels)} → ${channels(r.toChannels)}`,
          detail: r.quality,
        }
      : { name: "Resample", value: "None", detail: s ? "Plays at its own rate" : null, off: true },
  );
  const b = p.blend;
  const blend = b.automix ? "Automix into the next song" : b.crossfade === "fixed" && b.crossfadeMs ? `Crossfade ${Math.round(b.crossfadeMs / 1000)} s` : b.crossfade === "smart" ? "Smart crossfade" : bypassed ? "Bypassed" : "Off";
  out.push({ name: "Blend", value: blend, off: !b.automix && b.crossfade === "off" });
  const o = p.output;
  const how =
    o.mode === "exclusive"
      ? `Exclusive${o.bits ? `, ${o.bits}-bit` : ""}${o.rate ? ` at ${formatRate(o.rate)}` : ""}`
      : o.mode === "fallback"
        ? `Shared for now${o.rate ? ` at ${formatRate(o.rate)}` : ""}`
        : o.network
          ? "Over the network"
          : `Shared${o.rate ? ` at ${formatRate(o.rate)}` : ""}, through the system mixer`;
  out.push({
    name: "Output",
    value: o.device ?? "No output device",
    detail: [how, o.volumeBypassed ? "volume bypassed" : `volume ${Math.round(o.volume * 100)}%`].join(", "),
  });
  return out;
}

/** The stream quality label for the playing song, as the window works it out. */
export function useStreamLabel(): string | null {
  const track = usePlayer((s) => s.state?.track ?? null);
  const settings = useSettings((s) => s.settings);
  const connection = useApp((s) => s.health?.connection ?? s.auth?.server?.connection ?? null);
  return track ? streamQuality(track, connection, settings) : null;
}

/** Player bar: the playing song's format; opens the signal path. */
export function SignalBadge() {
  const on = useSettings((s) => s.settings?.signalPathBadge !== false);
  const track = usePlayer((s) => s.state?.track ?? null);
  const remote = usePlayer((s) => !!s.remote);
  const path = useSignal((s) => s.path);
  const open = useSignal((s) => s.open);
  const stream = useStreamLabel();
  const trackId = track?.id;
  // A fresh snapshot for each song (events keep it current after that).
  useEffect(() => {
    if (on && trackId && !remote) void loadSignal();
  }, [on, trackId, remote]);
  if (!on || !track || remote) return null;
  const current = path?.trackId === track.id ? path : null;
  const text = badgeText(current, stream ?? qualityLabel(track));
  if (!text) return null;
  const perfect = current?.verdict.kind === "bitPerfect";
  return (
    <PlateButton
      tone={perfect ? "lit" : "plain"}
      className={`signal-badge ${perfect ? "is-perfect" : ""} ${open ? "is-open" : ""}`}
      aria-haspopup="dialog"
      aria-expanded={open}
      aria-label={`Signal path: ${current ? verdictText(current, stream) : text}`}
      title="Signal path"
      {...{ [OPENER]: "" }}
      onClick={() => (open ? closeSignal() : openSignal())}
    >
      {text}
    </PlateButton>
  );
}

function Plate({ stage }: { stage: Stage }) {
  return (
    <li className={`signal-plate ${stage.off ? "is-off" : ""}`}>
      <span className="signal-plate__name">{stage.name}</span>
      <span className="signal-plate__value">{stage.value}</span>
      {stage.detail && <span className="signal-plate__detail">{stage.detail}</span>}
    </li>
  );
}

function Arrow() {
  return (
    <li className="signal-arrow" aria-hidden="true">
      <ArrowDown size={12} />
    </li>
  );
}

/**
 * The chain of plates and, when it isn't bit-perfect, why: the panel's body,
 * shared with the phone's sheet. `onSetting` opens the setting that would fix it.
 */
export function SignalBody({ path, stream, onSetting }: { path: SignalPath | null; stream: string | null; onSetting: (row: string) => void }) {
  const tunedFor = useTunedFor();
  if (!path) return <p className="signal-panel__empty">Looking at the signal path…</p>;
  if (path.verdict.kind === "idle") return <p className="signal-panel__empty">Nothing playing. Play something to see its signal path.</p>;
  const list = stages(path, stream, tunedFor);
  return (
    <>
      <ol className="signal-chain" aria-label="From the file to your speakers">
        {list.map((s, i) => (
          <PlateWithArrow key={s.name} stage={s} last={i === list.length - 1} />
        ))}
      </ol>
      {path.why && (
        <div className="signal-why">
          <h3 className="signal-why__title">Why isn't this bit-perfect?</h3>
          <p className="signal-why__text">{path.why.text}</p>
          {path.why.setting && (
            <button type="button" className="signal-why__go" onClick={() => onSetting(path.why!.setting!)}>
              Open the setting
            </button>
          )}
        </div>
      )}
    </>
  );
}

/** The headphones whose correction plays on the output in use, by name. */
export function useTunedFor(): string | null {
  const settings = useSettings((s) => s.settings);
  const device = useOutput((s) => s.state?.current);
  return bindingFor(settings, device)?.name ?? null;
}

/** The panel; mounted once, shown while the store says it's open. */
export function SignalPanel() {
  const open = useSignal((s) => s.open);
  const path = useSignal((s) => s.path);
  const stream = useStreamLabel();
  const panel = useRef<HTMLDivElement>(null);
  const returnTo = useRef<HTMLElement | null>(null);

  useLayoutEffect(() => {
    if (!open) return;
    returnTo.current = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    panel.current?.focus();
    return () => {
      const back = returnTo.current;
      if (back && document.contains(back)) back.focus();
    };
  }, [open]);

  // A click anywhere else closes it (an opener toggles it itself).
  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      const t = e.target as Element | null;
      if (!t || panel.current?.contains(t) || t.closest?.(`[${OPENER}]`)) return;
      closeSignal();
    };
    document.addEventListener("mousedown", onDown);
    return () => document.removeEventListener("mousedown", onDown);
  }, [open]);

  if (!open) return null;

  const onKeyDown = (e: KeyboardEvent) => {
    if (e.key === "Escape") {
      e.stopPropagation();
      e.preventDefault();
      closeSignal();
    }
  };
  // Focus leaving for somewhere outside closes it.
  const onBlur = (e: React.FocusEvent) => {
    const next = e.relatedTarget as Element | null;
    if (!next || panel.current?.contains(next) || next.closest?.(`[${OPENER}]`)) return;
    closeSignal();
  };

  const body = (
    <SignalBody
      path={path}
      stream={stream}
      onSetting={(row) => {
        closeSignal();
        nav.go({ name: "settings", row });
      }}
    />
  );
  const kind = path?.verdict.kind ?? "idle";
  return createPortal(
    <div
      ref={panel}
      className="signal-panel"
      role="dialog"
      aria-label="Signal path"
      tabIndex={-1}
      onKeyDown={onKeyDown}
      onBlur={onBlur}
      data-testid="signal-panel"
    >
      <header className="signal-panel__head">
        <h2 className="signal-panel__title">Signal path</h2>
        <button type="button" className="signal-panel__close" aria-label="Close signal path" onClick={closeSignal}>
          <X size={16} aria-hidden="true" />
        </button>
      </header>
      {path && kind !== "idle" && (
        <p className={`signal-verdict signal-verdict--${kind}`} aria-live="polite">
          {verdictText(path, stream)}
        </p>
      )}
      {body}
    </div>,
    document.body,
  );
}

function PlateWithArrow({ stage, last }: { stage: Stage; last: boolean }) {
  return (
    <>
      <Plate stage={stage} />
      {!last && <Arrow />}
    </>
  );
}
