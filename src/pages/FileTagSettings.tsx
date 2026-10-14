/** Settings bits for tags read from the files: the scan's progress line and the explicit choice. */
import type { SelectOption } from "../components/Select";
import type { ExplicitContent, TagScanStatus } from "../lib/types";
import { useFileTags } from "../stores/fileTags";

export const EXPLICIT_OPTIONS = [
  { value: "allowed", label: "Allowed" },
  { value: "filtered", label: "Filtered" },
] as const satisfies SelectOption<ExplicitContent>[];

const n = (x: number) => x.toLocaleString("en-US");

/** The progress line under "Read extra tags from files". */
export function scanNote(s: TagScanStatus | null): string | undefined {
  if (!s || !s.enabled || s.total === 0) return undefined;
  const failed = s.failed > 0 ? ` ${n(s.failed)} couldn't be read.` : "";
  if (s.paused) return `Paused at ${n(s.done)} of ${n(s.total)}: the server can't be reached directly. Trying again soon.`;
  if (s.done < s.total) return `Reading tags: ${n(s.done)} of ${n(s.total)}.${failed}`;
  return `Tags read from all ${n(s.total)} songs.${failed}`;
}

/** Live progress text (updates as the scan moves). */
export function TagScanNote() {
  const scan = useFileTags((s) => s.scan);
  const text = scanNote(scan);
  return text ? (
    <span role="status" aria-live="polite">
      {text}
    </span>
  ) : null;
}
