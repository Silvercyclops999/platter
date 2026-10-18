/**
 * "First heard: March 3, 2021" under an album's buttons: the earliest play
 * of any of its songs, imported history included. Shown while the
 * listening diary is kept and the album was ever played.
 */
import { useEffect, useState } from "react";
import { backend } from "../lib/backend";
import { longDate, useDiaryOn, useDiaryVersion } from "../lib/diary";
import "./diary.css";

export function FirstHeard({ albumId }: { albumId: string }) {
  const on = useDiaryOn();
  const version = useDiaryVersion();
  const [at, setAt] = useState<number | null>(null);
  // Another album clears the last one's date before its own arrives.
  const [shownFor, setShownFor] = useState(albumId);
  if (shownFor !== albumId) {
    setShownFor(albumId);
    setAt(null);
  }
  useEffect(() => {
    if (!on) return;
    let alive = true;
    backend.albumFirstHeard(albumId).then(
      (t) => alive && setAt(t),
      () => alive && setAt(null),
    );
    return () => {
      alive = false;
    };
  }, [albumId, on, version]);
  if (!on || at == null) return null;
  return (
    <p className="first-heard">
      <span className="first-heard__label">First heard:</span> {longDate(at)}
    </p>
  );
}
