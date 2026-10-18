/**
 * The signal path for the browser mock, following the core's rules
 * (src-tauri/src/audio/signal.rs): the track's own format, the sound
 * settings, and a shared device that runs at 48 kHz.
 */
import type { Exclusive, OutputDevice, Settings, SignalPath, Track } from "../types";

/** The mock's shared output rate. */
export const MOCK_DEVICE_RATE = 48_000;

const LOSSLESS = ["flac", "alac", "wav", "aiff", "aif", "pcm", "ape", "wv", "wavpack", "dsd"];

function khz(rate: number): string {
  return rate % 1000 === 0 ? `${rate / 1000} kHz` : `${(rate / 1000).toFixed(1)} kHz`;
}

export interface MockSignalInput {
  track: Track | null;
  settings: Settings;
  device: OutputDevice | null;
  exclusive: Exclusive;
  volume: number;
  muted: boolean;
  /** The stream is an MP3 transcode at this many kbps (a quality cap). */
  transcodeKbps: number | null;
}

export function mockSignalPath(i: MockSignalInput): SignalPath {
  const s = i.settings;
  const bitPerfect = i.exclusive.state === "on";
  const leveling = bitPerfect ? "off" : s.leveling;
  const eqOn = !bitPerfect && s.eqEnabled;
  const corr = bitPerfect ? undefined : s.headphoneCorrection.find((c) => c.deviceId === i.device?.id);
  const preampDb = bitPerfect ? 0 : s.preampDb;
  const outRate = i.exclusive.state === "on" ? i.exclusive.rate : MOCK_DEVICE_RATE;
  const volume = i.muted ? 0 : i.volume;
  const output: SignalPath["output"] = {
    device: i.device?.name ?? null,
    mode: i.exclusive.state === "on" ? "exclusive" : i.exclusive.state === "fallback" ? "fallback" : "shared",
    reason: i.exclusive.state === "fallback" ? i.exclusive.reason : null,
    rate: i.track ? outRate : null,
    channels: i.track ? 2 : null,
    bits: i.exclusive.state === "on" ? i.exclusive.bits : null,
    network: !!i.device?.id.startsWith("net:"),
    volume,
    volumeBypassed: bitPerfect,
  };
  const crossfade = bitPerfect ? "off" : s.crossfade === "off" ? "off" : s.crossfade === "fixed" ? "fixed" : "smart";
  const base = {
    loudness: { on: leveling !== "off", mode: leveling, gainDb: i.track && leveling !== "off" ? -6.2 : null },
    eq: { eq: eqOn, correction: !!corr, correctionPreampDb: corr?.preampDb ?? null, preampDb, limiter: !bitPerfect && s.limiter },
    // Where Automix plays isn't modelled here; the core decides per song.
    blend: { automix: false, crossfade, crossfadeMs: crossfade === "fixed" ? s.crossfadeSeconds * 1000 : null },
    output,
  } as const;
  const t = i.track;
  if (!t) {
    return { trackId: null, source: null, decode: null, resample: null, ...base, verdict: { kind: "idle", text: "Nothing playing" }, why: null };
  }
  const transcoded = i.transcodeKbps !== null;
  const codec = transcoded ? "mp3" : (t.codec ?? "").toLowerCase();
  const lossless = LOSSLESS.includes(codec);
  const rate = transcoded ? 44_100 : (t.sampleRate ?? 44_100);
  const source: NonNullable<SignalPath["source"]> = {
    codec: codec ? codec.toUpperCase() : null,
    lossless,
    bitDepth: !transcoded && lossless ? (t.bitDepth ?? 16) : null,
    sampleRate: rate,
    channels: 2,
    bitrate: transcoded ? null : (t.bitrate ?? null),
    transcoded,
    from: "server",
  };
  const resample = rate !== outRate ? { fromRate: rate, toRate: outRate, fromChannels: 2, toChannels: 2, quality: "High quality (FFT)" } : null;
  let verdict: SignalPath["verdict"];
  let why: SignalPath["why"];
  if (!lossless) {
    const label = transcoded ? "MP3" : `${source.codec}${t.bitrate ? ` ${t.bitrate}` : ""}`;
    verdict = { kind: "lossy", text: transcoded ? `Lossy stream (${label})` : `Lossy source (${label})` };
    why = transcoded
      ? { text: "Your server is sending a smaller MP3 instead of the file. Set streaming quality to Original.", setting: "qualityHome" }
      : { text: "The file itself is lossy, so no setting can make it bit-perfect.", setting: null };
  } else if (bitPerfect && !resample) {
    verdict = { kind: "bitPerfect", text: "Bit-perfect" };
    why = null;
  } else {
    const parts: string[] = [];
    if (resample) parts.push(`resampled to ${khz(outRate)}`);
    const applied: string[] = [];
    if (volume < 1 || base.loudness.gainDb || preampDb) applied.push("volume");
    if (eqOn) applied.push("EQ");
    if (corr) applied.push("headphone correction");
    if (applied.length) parts.push(`${applied.length > 1 ? `${applied.slice(0, -1).join(", ")} and ${applied[applied.length - 1]}` : applied[0]} applied`);
    if (!parts.length) parts.push("shared with other apps");
    verdict = { kind: "lossless", text: `Lossless, ${parts.join(", ")}` };
    why =
      i.exclusive.state === "fallback"
        ? { text: `Bit-perfect output is playing shared for now: ${i.exclusive.reason}`, setting: "bitPerfectOutputs" }
        : {
            text: "The system mixer, volume and sound settings all touch the sound. Turn on Bit-perfect output for this device.",
            setting: "bitPerfectOutputs",
          };
  }
  return { trackId: t.id, source, decode: "32-bit float", resample, ...base, verdict, why };
}
