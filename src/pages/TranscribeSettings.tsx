/** Settings bits for lyrics transcription: the status line, the model picker with download and delete, processor use, and the batch button. */
import { Button } from "../components/Button";
import { Select } from "../components/Select";
import type { Settings, TranscribeModel, TranscribeProcessor, TranscribeStatus } from "../lib/types";
import { updateSettings } from "../stores/settings";
import { modelSize, transcribe, useTranscribe } from "../stores/transcribe";

const MODEL_LABEL: Record<TranscribeModel, string> = { small: "Small (faster)", medium: "Medium (slower, much harder on the processor)" };

export const PROCESSOR_OPTIONS: { value: TranscribeProcessor; label: string }[] = [
  { value: "light", label: "Light" },
  { value: "balanced", label: "Balanced" },
  { value: "fast", label: "Fast" },
];

const PAUSED: Record<NonNullable<TranscribeStatus["paused"]>, string> = {
  battery: "Paused while on battery power.",
  offline: "Paused: the server can't be reached. Trying again soon.",
  busy: "Paused while your computer is busy.",
};

/** "about 3 minutes per song" for a long run, once a song is done. */
export function paceNote(secs: number | undefined): string | undefined {
  if (secs === undefined) return undefined;
  const min = Math.max(1, Math.round(secs / 60));
  return `about ${min} ${min === 1 ? "minute" : "minutes"} per song`;
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
  const options = status.models.map((m) => ({ value: m.kind, label: `${MODEL_LABEL[m.kind]}, ${modelSize(m.sizeBytes)}` }));
  const m = status.models.find((x) => x.kind === s.transcribeModel);
  const busy = status.current !== undefined && m?.installed;
  return (
    <div className="transcribe-model">
      <Select
        value={s.transcribeModel}
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
        <Button size="sm" disabled={!status.available || status.models.some((x) => x.downloading)} onClick={() => void transcribe.download(s.transcribeModel)}>
          Download {m ? modelSize(m.sizeBytes) : ""}
        </Button>
      )}
    </div>
  );
}

/** The batch's progress line. */
export function batchNote(s: TranscribeStatus | null): string | undefined {
  if (!s || !s.enabled) return undefined;
  if (s.error?.kind === "transcribe") return s.error.message;
  if (s.batch.total === 0) return undefined;
  const of = `${s.batch.done} of ${s.batch.total} songs`;
  if (s.paused) return `${of}. ${PAUSED[s.paused]}`;
  const pace = paceNote(s.secsPerSong);
  return pace ? `Transcribing: ${of}, ${pace}.` : `Transcribing: ${of}.`;
}

export function BatchNote() {
  const text = batchNote(useTranscribe((s) => s.status));
  return text ? (
    <span role="status" aria-live="polite">
      {text}
    </span>
  ) : null;
}

/** "Transcribe now" for every unreleased song without lyrics, or Cancel while it runs. */
export function BatchControl() {
  const status = useTranscribe((s) => s.status);
  if (!status) return null;
  const running = status.batch.total > 0;
  return running ? (
    <Button size="sm" onClick={() => void transcribe.cancel()}>
      Cancel
    </Button>
  ) : (
    <Button size="sm" disabled={!status.enabled || !status.available} onClick={() => void transcribe.unreleased()}>
      Transcribe now
    </Button>
  );
}
