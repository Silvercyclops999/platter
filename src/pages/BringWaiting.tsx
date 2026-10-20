/**
 * A brought-over playlist's songs that aren't in the library yet, greyed
 * under its songs. Each fills in (and leaves this list) when its album
 * arrives. Nothing shows for any other playlist.
 */
import { useEffect, useState } from "react";
import { backend } from "../lib/backend";
import { formatDuration } from "../lib/format";
import type { ImportedRow } from "../lib/importTypes";
import "./bring.css";

export function BringWaiting({ playlistId }: { playlistId: string }) {
  const [rows, setRows] = useState<ImportedRow[]>([]);
  useEffect(() => {
    let alive = true;
    const load = () =>
      backend.bringWaitingRows(playlistId).then(
        (r) => alive && setRows(r),
        () => {},
      );
    void load();
    const un = backend.listen("library://changed", () => void load());
    return () => {
      alive = false;
      void un.then((f) => f());
    };
  }, [playlistId]);
  if (!rows.length) return null;
  return (
    <section className="bring-waiting" aria-labelledby={`bring-waiting-${playlistId}`} data-testid="bring-waiting">
      <h2 id={`bring-waiting-${playlistId}`} className="bring-waiting__title">
        Not in your library yet
      </h2>
      <p className="bring-waiting__sub">These fill in as their albums arrive.</p>
      <ul className="bring-waiting__rows">
        {rows.map((r) => (
          <li key={r.songId} className="bring-waiting__row" aria-label={`${r.title} by ${r.artist}, not in your library`}>
            <span className="bring-waiting__main">
              <span className="bring-waiting__song truncate">{r.title}</span>
              <span className="bring-waiting__artist truncate">{r.artist}</span>
            </span>
            <span className="bring-waiting__album truncate">{r.album ?? ""}</span>
            <span className="bring-waiting__len">{r.durationMs ? formatDuration(r.durationMs) : ""}</span>
          </li>
        ))}
      </ul>
    </section>
  );
}
