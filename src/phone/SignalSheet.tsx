/**
 * The signal path on the phone: now playing's format plate is a button, and
 * it opens the chain from the file to the output as a sheet, the same plates
 * as the computer's panel.
 */
import { useEffect } from "react";
import { Plate } from "../components/Plate";
import { isHiRes, qualityLabel } from "../lib/format";
import type { SignalPath, Track } from "../lib/types";
import { useSettings } from "../stores/settings";
import { loadSignal, useSignal } from "../stores/signalPath";
import { usePlayer } from "../stores/player";
import { SignalBody, useStreamLabel, verdictText } from "../shell/SignalPath";
import { SheetFrame, tap } from "./bits";
import { phone } from "./phoneUi";

type Format = Pick<Track, "codec" | "bitDepth" | "sampleRate" | "bitrate">;

/**
 * What the plate says: what is streamed when the window knows it, else the
 * file's own format, else what the core says the source is. Lossy formats
 * get a plate too ("MP3 320"); only a song with no format at all has none.
 */
export function signalPlateText(track: Format, stream: string | null, path: SignalPath | null): string | null {
  const own = stream ?? qualityLabel(track);
  if (own) return own;
  const s = path?.source;
  return s?.codec ? qualityLabel({ codec: s.codec, bitDepth: s.bitDepth, sampleRate: s.sampleRate, bitrate: s.bitrate }) : null;
}

/** The engraved format plate under now playing's transport; a tap opens the signal path. */
export function SignalPlate({ track }: { track: Track }) {
  const stream = useStreamLabel();
  const remote = usePlayer((s) => !!s.remote);
  const raw = useSignal((s) => s.path);
  const path = raw?.trackId === track.id ? raw : null;
  const badge = useSettings((s) => s.settings?.signalPathBadge !== false);
  // A fresh snapshot for each song, so the plate can fall back on the core's word for the format.
  useEffect(() => {
    if (!remote) void loadSignal();
  }, [track.id, remote]);
  const text = signalPlateText(track, stream, path);
  if (!text) return null;
  const perfect = path?.verdict.kind === "bitPerfect";
  const lit = perfect || isHiRes(path?.source ?? track);
  const plate = (
    <Plate size="sm" tone={lit ? "lit" : "plain"} className="ph-plate">
      {text}
    </Plate>
  );
  // With the badge turned off, or for another device, the plate only says the format.
  if (!badge || remote) return <span className="ph-np__plate">{plate}</span>;
  return (
    <button
      type="button"
      className="ph-np__plate"
      aria-haspopup="dialog"
      aria-label={`Signal path: ${path ? verdictText(path, stream) : text}`}
      onClick={() => {
        tap("selection");
        phone.push({ kind: "signal" });
      }}
    >
      {plate}
    </button>
  );
}

export function SignalSheet() {
  const path = useSignal((s) => s.path);
  const stream = useStreamLabel();
  const trackId = usePlayer((s) => s.state?.track?.id ?? null);
  // A fresh snapshot when it opens and for each song; events keep it live after that.
  useEffect(() => void loadSignal(), [trackId]);
  const kind = path?.verdict.kind ?? "idle";
  return (
    <SheetFrame
      title="Signal path"
      subtitle={path && kind !== "idle" ? <span className={`ph-signal__verdict ph-signal__verdict--${kind}`}>{verdictText(path, stream)}</span> : undefined}
      onClose={phone.pop}
      dimBack
      className="ph-signal"
    >
      <SignalBody path={path} stream={stream} onSetting={(row) => phone.go({ name: "settings", row })} />
    </SheetFrame>
  );
}
