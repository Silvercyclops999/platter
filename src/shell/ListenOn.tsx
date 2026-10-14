import { Globe, Laptop, Smartphone, Speaker, Tv } from "lucide-react";
import { useEffect } from "react";
import { Skeleton } from "../components/Skeleton";
import type { PlayerKind, RemotePlayer, RemoteSession } from "../lib/types";
import { usePlayer } from "../stores/player";
import { moveTo, returnHere, useRemote, watchPlayers } from "../stores/remote";
import { useSettings } from "../stores/settings";
import "./listen-on.css";

/** The icon for a kind of Plex player. */
export function PlayerIcon({ kind, size = 20 }: { kind: PlayerKind; size?: number }) {
  switch (kind) {
    case "phone":
      return <Smartphone size={size} />;
    case "speaker":
      return <Speaker size={size} />;
    case "browser":
      return <Globe size={size} />;
    case "tv":
      return <Tv size={size} />;
    default:
      return <Laptop size={size} />;
  }
}

/** The grey line under a player's name. */
export function playerSub(p: RemotePlayer): string {
  if (p.playing) return p.playing;
  return p.online ? p.product : `${p.product} • Not active`;
}

/** The current-device card while playback is on another device. */
export function RemoteCurrent({ session }: { session: RemoteSession }) {
  const status = session.status === "connecting" ? "Connecting…" : session.status === "paused" ? "Paused" : "Playing";
  return (
    <div className="output-current" aria-labelledby="output-current-label" role="group">
      <span className="output-current__icon" aria-hidden="true">
        <PlayerIcon kind={session.kind} size={28} />
      </span>
      <div className="output-current__text">
        <div id="output-current-label" className="output-current__label">
          Listening on
        </div>
        <div className="output-current__name" title={session.playerName}>
          {session.playerName}
        </div>
        <div className="output-current__sub">{status}</div>
      </div>
    </div>
  );
}

function Row({
  icon,
  name,
  sub,
  busy,
  label,
  muted,
  onPick,
}: {
  icon: React.ReactNode;
  name: string;
  sub: string;
  busy: boolean;
  label: string;
  muted?: boolean;
  onPick: () => void;
}) {
  return (
    <li>
      <button
        type="button"
        className={`output-row${muted ? " listen-row--idle" : ""}`}
        aria-label={label}
        aria-busy={busy || undefined}
        onClick={onPick}
      >
        <span className="output-row__icon" aria-hidden="true">
          {icon}
        </span>
        <span className="output-row__text">
          <span className="output-row__name truncate">{name}</span>
          <span className="output-row__sub truncate">{busy ? "Connecting…" : sub}</span>
        </span>
      </button>
    </li>
  );
}

/** "Listen on": Plex players to move playback to, and this computer while remote. */
export function ListenOn() {
  const show = useSettings((s) => s.settings?.showPlexPlayers ?? true);
  const players = useRemote((s) => s.players);
  const busy = useRemote((s) => s.busy);
  const session = usePlayer((s) => s.remote);

  useEffect(() => (show ? watchPlayers() : undefined), [show]);

  if (!show && !session) return null;
  const others = (players ?? []).filter((p) => p.id !== session?.playerId);

  return (
    <section className="output-section" aria-labelledby="output-listen-on">
      <h3 id="output-listen-on" className="output-section__title">
        Listen on
      </h3>
      <ul className="output-list">
        {session && (
          <Row
            icon={<Laptop size={20} />}
            name="This computer"
            sub="Bring playback back here"
            label="Play on this computer"
            busy={busy === null}
            onPick={() => void returnHere()}
          />
        )}
        {show &&
          others.map((p) => (
            <Row
              key={p.id}
              icon={<PlayerIcon kind={p.kind} />}
              name={p.name}
              sub={playerSub(p)}
              label={`Play on ${p.name}`}
              muted={!p.online}
              busy={busy === p.id}
              onPick={() => void moveTo(p)}
            />
          ))}
      </ul>
      {show && players === null && <Skeleton height={48} radius="card" style={{ marginTop: 4 }} />}
      {show && players !== null && others.length === 0 && (
        <p className="output-empty">No Plex players found. Open Plexamp or another Plex app on a device signed in to your account.</p>
      )}
    </section>
  );
}
