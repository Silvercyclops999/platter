import { useEffect } from "react";
import { usePlayer } from "../stores/player";
import { useUi } from "../stores/ui";
import { PlayerIcon } from "./ListenOn";
import "./listen-on.css";

/** How long "Stopped on <device>" stays up. */
export const STOPPED_NOTE_MS = 8000;

/** The accent strip above the player bar: "Playing on <device>" while remote. */
export function RemoteStrip() {
  const session = usePlayer((s) => s.remote);
  const stoppedOn = usePlayer((s) => s.stoppedOn);

  useEffect(() => {
    if (!stoppedOn || session) return;
    const t = setTimeout(() => usePlayer.setState({ stoppedOn: null }), STOPPED_NOTE_MS);
    return () => clearTimeout(t);
  }, [stoppedOn, session]);

  if (session) {
    return (
      <button
        type="button"
        className="remote-strip"
        aria-label={`Playing on ${session.playerName}. Open the device picker`}
        onClick={() => useUi.getState().setOutputOpen(true)}
      >
        <PlayerIcon kind={session.kind} size={14} />
        <span className="truncate">Playing on {session.playerName}</span>
      </button>
    );
  }
  if (stoppedOn) {
    return (
      <div className="remote-strip remote-strip--stopped" role="status">
        <span className="truncate">Stopped on {stoppedOn}</span>
      </div>
    );
  }
  return null;
}
