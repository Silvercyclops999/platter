import { useEffect, useState } from "react";
import { Button } from "../components/Button";
import { Dialog } from "../components/Dialog";
import { useAutoRequest } from "../stores/radar";
import { useSettings } from "../stores/settings";
import "./radar.css";

/** Daily limit choices (same as the core's). */
export const AUTO_CAP_OPTIONS = [
  { value: "1", label: "1 a day" },
  { value: "2", label: "2 a day" },
  { value: "3", label: "3 a day" },
  { value: "5", label: "5 a day" },
  { value: "10", label: "10 a day" },
] as const;

/** Settings: the artists auto-request is on for, each with Turn off. */
export function AutoArtistsList() {
  const artists = useAutoRequest((s) => s.artists);
  useEffect(() => {
    void useAutoRequest.getState().load();
  }, []);
  if (!artists.length) {
    return <p className="auto-artists__empty">None yet. Turn it on for an artist from the artist page's menu (the … button).</p>;
  }
  return (
    <ul className="auto-artists" aria-label="Artists with auto-request on">
      {artists.map((a) => (
        <li key={a.id} className="auto-artists__item">
          <span className="truncate">{a.name}</span>
          <Button size="sm" variant="ghost" aria-label={`Turn off auto-request for ${a.name}`} onClick={() => void useAutoRequest.getState().set(a.id, a.name, false)}>
            Turn off
          </Button>
        </li>
      ))}
    </ul>
  );
}

/**
 * Turning on auto-request for an artist: what it will do, asked once per
 * artist. Turning it off needs no question.
 */
export function AutoRequestDialog({ id, name, open, onClose }: { id: string; name: string; open: boolean; onClose: () => void }) {
  const cap = useSettings((s) => s.settings?.autoRequestCap ?? 3);
  const allOn = useSettings((s) => s.settings?.autoRequest === true);
  const [busy, setBusy] = useState(false);
  const turnOn = async () => {
    setBusy(true);
    const ok = await useAutoRequest.getState().set(id, name, true);
    setBusy(false);
    if (ok) onClose();
  };
  return (
    <Dialog open={open} onClose={onClose} label={`Auto-request new albums by ${name}`} className="confirm">
      <h2 className="confirm__title">Auto-request new albums by {name}?</h2>
      <p className="confirm__body">
        When MusicBrainz lists a new official album by {name}, Platter asks Lidarr for it, after the same setup check as any request. Bootlegs,
        promos and anything sharing a title with an album in your library, unreleased ones included, are never requested.
      </p>
      <p className="confirm__body">
        Up to {cap} a day across all artists, with a notification each time. Anything Platter won't request shows why in Settings under Requests.
        {allOn ? "" : " This also turns on auto-request in Settings."}
      </p>
      <div className="confirm__actions">
        <Button variant="ghost" onClick={onClose}>
          Cancel
        </Button>
        <Button variant="primary" disabled={busy} onClick={() => void turnOn()}>
          Turn on
        </Button>
      </div>
    </Dialog>
  );
}
