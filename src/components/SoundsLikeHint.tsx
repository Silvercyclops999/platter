/**
 * "Sounds like" on an unreleased song that identify by sound matched to a
 * released recording. Only a hint: the song keeps its own name and is
 * never treated as that recording.
 */
import { AudioLines } from "lucide-react";
import { useEffect, useState } from "react";
import { backend } from "../lib/backend";
import type { SoundsLike } from "../lib/types";
import { nav } from "../stores/router";
import { useSettings } from "../stores/settings";

export function SoundsLikeHint({ trackId, onOpen }: { trackId: string; onOpen?: () => void }) {
  const on = useSettings((s) => s.settings?.identifyBySound === true);
  const [hint, setHint] = useState<SoundsLike | null>(null);
  useEffect(() => {
    if (!on) {
      setHint(null);
      return;
    }
    let alive = true;
    backend.trackSoundsLike(trackId).then(
      (h) => alive && setHint(h),
      () => {},
    );
    return () => {
      alive = false;
    };
  }, [trackId, on]);
  if (!hint || !hint.unreleased) return null;
  const name = hint.artist ? `${hint.title} by ${hint.artist}` : hint.title;
  const copy = hint.inLibrary;
  return (
    <p className="sounds-like" title="A hint from its sound. It stays its own song and is never renamed or matched to the release.">
      <AudioLines size={14} aria-hidden="true" />
      <span>
        Sounds like{" "}
        {copy ? (
          <button
            type="button"
            className="link"
            onClick={() => {
              onOpen?.();
              nav.go({ name: "album", id: copy.albumId });
            }}
          >
            {name}
          </button>
        ) : (
          name
        )}
        <span className="sounds-like__note"> (a hint from its sound, {hint.score}% match)</span>
      </span>
    </p>
  );
}
