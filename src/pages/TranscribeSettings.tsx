/** Settings bits for lyrics transcription: the status line, the model picker with download and delete, the graphics card, processor use, and the batch button. */
import { Button } from "../components/Button";
import { Select } from "../components/Select";
import type { Settings, TranscribeGpu, TranscribeModel, TranscribeModelInfo, TranscribeProcessor, TranscribeStatus } from "../lib/types";
import { updateSettings } from "../stores/settings";
import { modelSize, transcribe, useTranscribe } from "../stores/transcribe";

/** Short names for the picker; size and speed go in the line under it. */
export const MODEL_LABEL: Record<TranscribeModel, string> = {
  small: "Small",
  medium: "Medium",
  turbo: "Turbo (best for graphics cards)",
};

const MODEL_NAME: Record<TranscribeModel, string> = { small: "small", medium: "medium", turbo: "turbo" };

export const GPU_OPTIONS: { value: TranscribeGpu; label: string }[] = [
  { value: "auto", label: "Auto" },
  { value: "off", label: "Off" },
];

/** "about 8 seconds" or "about 3 minutes". */
export function aboutTime(secs: number): string {
  if (secs < 90) {
    const s = Math.max(1, Math.round(secs));
    return `about ${s} ${s === 1 ? "second" : "seconds"}`;
  }
  const min = Math.round(secs / 60);
  return `about ${min} minutes`;
}

/** How long a song takes with this model on the device in use: measured here when it has run, else a rough guide. */
export function speedNote(m: TranscribeModelInfo, s: TranscribeStatus): string {
  const where = s.gpu ? "on your graphics card" : "on the processor";
  if (m.secsPerSong !== undefined) return `${aboutTime(m.secsPerSong)} per song ${where}`;
  return s.gpu ? `a few seconds per song ${where}` : `a few minutes per song ${where}`;
}

/** The note under "Use the graphics card". */
export function gpuNote(s: TranscribeStatus | null, setting: TranscribeGpu | undefined): string | undefined {
  if (!s || !s.available) return undefined;
  if (s.gpu) return `Uses your graphics card (${s.gpu}).`;
  if (setting === "off" && s.gpuFound) return `Uses the processor. Your graphics card (${s.gpuFound}) isn't used.`;
  if (setting !== "off" && s.enabled && !s.gpuFound) return "Uses the processor: no graphics card that can help was found.";
  return undefined;
}

export function GpuNote({ setting }: { setting: TranscribeGpu | undefined }) {
  const text = gpuNote(useTranscribe((st) => st.status), setting);
  return text ? (
    <span role="status" aria-live="polite">
      {text}
    </span>
  ) : null;
}

export const PROCESSOR_OPTIONS: { value: TranscribeProcessor; label: string }[] = [
  { value: "light", label: "Light" },
  { value: "balanced", label: "Balanced" },
  { value: "fast", label: "Fast" },
];

const PAUSED: Record<NonNullable<TranscribeStatus["paused"]>, string> = {
  battery: "Paused while on battery power.",
  offline: "Paused: the server can't be reached. Trying again soon.",
  busy: "Paused while your computer is busy.",
  hours: "Waiting for the night hours.",
  bridge: "Paused: your Bridge transcribes these at night, so this computer doesn't have to.",
};

/** "about 20 seconds per song" for a long run, once a song is done (measured, breather included). */
export function paceNote(secs: number | undefined): string | undefined {
  if (secs === undefined) return undefined;
  return `${aboutTime(secs)} per song`;
}

/** The line under the model picker: its size and speed here, and a nudge toward Turbo on a graphics card. */
export function modelNote(s: TranscribeStatus): string | undefined {
  const m = s.models.find((x) => x.kind === s.model);
  if (!m) return undefined;
  const line = `${modelSize(m.sizeBytes)}, ${speedNote(m, s)}.`;
  return s.gpu && s.model !== "turbo" ? `${line} Turbo is faster and more accurate on your graphics card.` : line;
}

/** The line under the switch: why it can't run, the last failure, or the model's state. */
export function transcribeNote(s: TranscribeStatus | null): string | undefined {
  if (!s) return undefined;
  if (!s.available) return s.unavailableReason ?? "Transcription isn't available on this computer.";
  if (s.error && s.error.kind !== "transcribe") return s.error.message;
  if (!s.enabled) return undefined;
  const m = s.models.find((x) => x.kind === s.model);
  if (m?.downloading) return `Downloading the ${s.model} model: ${Math.floor((m.doneBytes / Math.max(1, m.sizeBytes)) * 100)}%.`;
  if (m && !m.installed) return `The ${s.model} model (${modelSize(m.sizeBytes)}) downloads the first time you transcribe, or now from below.`;
  return undefined;
}

export function TranscribeNote() {
  const status = useTranscribe((s) => s.status);
  const text = transcribeNote(status);
  if (!text) return null;
  const isError = !!status?.error && status.error.kind !== "transcribe";
  return isError ? (
    <span className="info-key__error" role="alert">
      {text}
    </span>
  ) : (
    <span role="status" aria-live="polite">
      {text}
    </span>
  );
}

/** Model choice, then Download (with its size), Cancel while downloading, or Delete. */
export function ModelControl({ s, ids }: { s: Settings | null; ids: { title: string; desc: string } }) {
  const status = useTranscribe((st) => st.status);
  if (!s || !status) return null;
  const options = status.models.map((m) => ({ value: m.kind, label: MODEL_LABEL[m.kind] }));
  const note = modelNote(status);
  // The model in use: the one picked, else this computer's default.
  const m = status.models.find((x) => x.kind === status.model);
  const busy = status.current !== undefined && m?.installed;
  return (
    <div className="transcribe-model">
      {note && <span className="transcribe-model__note">{note}</span>}
      <Select
        value={status.model}
        options={options}
        onChange={(v) => void updateSettings({ transcribeModel: v })}
        labelledBy={ids.title}
        describedBy={ids.desc}
      />
      {m?.downloading ? (
        <Button size="sm" onClick={() => void transcribe.cancelDownload()}>
          Cancel download
        </Button>
      ) : m?.installed ? (
        <Button size="sm" disabled={busy} onClick={() => void transcribe.deleteModel(m.kind)}>
          Delete model
        </Button>
      ) : (
        <Button size="sm" disabled={!status.available || status.models.some((x) => x.downloading)} onClick={() => void transcribe.download(status.model)}>
          Download {m ? modelSize(m.sizeBytes) : ""}
        </Button>
      )}
    </div>
  );
}

/** Songs done with another model that can be done again with the chosen one, once it's installed. */
export function redoCount(s: TranscribeStatus | null): number {
  if (!s || !s.models.find((x) => x.kind === s.model)?.installed) return 0;
  return (s.redo ?? []).reduce((n, r) => n + r.count, 0);
}

/** The batch's progress line: what it's doing now, the model it uses meanwhile, and songs to redo. */
export function batchNote(s: TranscribeStatus | null): string | undefined {
  if (!s || !s.enabled) return undefined;
  if (s.error?.kind === "transcribe") return s.error.message;
  const redo = redoCount(s);
  const redoText =
    redo > 0
      ? `${redo === 1 ? "1 song was" : `${redo} songs were`} done with the ${MODEL_NAME[s.redo![0].model]} model while the ${MODEL_NAME[s.model]} model downloaded. Redo does ${redo === 1 ? "it" : "them"} again with it.`
      : undefined;
  if (s.batch.total === 0) return redoText;
  const of = `${s.batch.done} of ${s.batch.total} songs`;
  const tail = redoText ? ` ${redoText}` : "";
  if (s.userPaused) return `${of}. Paused.${tail}`;
  if (s.paused) return `${of}. ${PAUSED[s.paused]}${tail}`;
  const c = s.current;
  const meanwhile =
    c && c.model && c.model !== s.model
      ? ` Using the ${MODEL_NAME[c.model]} model until the ${MODEL_NAME[s.model]} model finishes downloading, then the rest switch to it.`
      : "";
  if (c?.waitingForPlayback) return `Waiting: music is playing. ${of}.${meanwhile}${tail}`;
  const doing = c?.title ? `Transcribing “${c.title}”` : "Transcribing";
  const pace = paceNote(s.secsPerSong);
  return `${doing}: ${of}${pace ? `, ${pace}` : ""}.${meanwhile}${tail}`;
}

export function BatchNote() {
  const text = batchNote(useTranscribe((s) => s.status));
  return text ? (
    <span role="status" aria-live="polite">
      {text}
    </span>
  ) : null;
}

/** "Transcribe now" for every unreleased song without lyrics, or Pause (Resume) and Cancel while it runs; Redo for songs done with an older model. */
export function BatchControl() {
  const status = useTranscribe((s) => s.status);
  if (!status) return null;
  const running = status.batch.total > 0;
  const redo = redoCount(status);
  return (
    <div className="transcribe-batch">
      {redo > 0 && (
        <Button size="sm" onClick={() => void transcribe.redo()}>
          Redo {redo === 1 ? "1 song" : `${redo} songs`}
        </Button>
      )}
      {running ? (
        <>
          {status.userPaused ? (
            <Button size="sm" onClick={() => void transcribe.resume()}>
              Resume
            </Button>
          ) : (
            <Button size="sm" onClick={() => void transcribe.pause()}>
              Pause
            </Button>
          )}
          <Button size="sm" onClick={() => void transcribe.cancel()}>
            Cancel
          </Button>
        </>
      ) : (
        <Button size="sm" disabled={!status.enabled || !status.available} onClick={() => void transcribe.unreleased()}>
          Transcribe now
        </Button>
      )}
    </div>
  );
}
