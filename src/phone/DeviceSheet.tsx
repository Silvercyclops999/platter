/**
 * Where music plays: the output in use as a large card (name, quality, the
 * song, a volume slider when it's another device), then the other places
 * as rows with a type icon. A tapped row shows its progress at once until
 * the move is confirmed, and the player stays open through a switch.
 */
import { Plate } from "../components/Plate";
import { Cast, ChevronDown, Laptop, Loader2, MonitorSpeaker, Smartphone, Speaker, Tv, Users } from "lucide-react";
import { useEffect, useState } from "react";
import { Slider } from "../components/Slider";
import type { PlayerKind, RemotePlayer } from "../lib/types";
import { player, usePlayer } from "../stores/player";
import { moveTo, returnHere, useRemote, watchPlayers } from "../stores/remote";
import { toast } from "../stores/toasts";
import { SheetFrame, plateText, tap } from "./bits";
import { phone } from "./phoneUi";
import { useIsDownloaded } from "../stores/downloads";

const KIND_ICON: Record<PlayerKind, typeof Speaker> = {
  computer: Laptop,
  phone: Smartphone,
  speaker: Speaker,
  browser: MonitorSpeaker,
  tv: Tv,
  group: Users,
};

const VIA_LINE: Record<RemotePlayer["via"], string> = {
  platter: "Platter",
  plex: "Plex player",
  cast: "Cast",
  airplay: "AirPlay",
  dlna: "Network player",
};

/** The output button in the mini player and on now playing; "Playing on <device>" in the accent while remote. */
export function OutputButton({ compact = false }: { compact?: boolean }) {
  const remote = usePlayer((s) => s.remote);
  const Icon = remote ? KIND_ICON[remote.kind] : Smartphone;
  return (
    <button
      type="button"
      className={`ph-output ${remote ? "is-remote" : ""} ${compact ? "ph-output--compact" : ""}`}
      aria-label={remote ? `Playing on ${remote.playerName}. Choose where music plays` : "Choose where music plays"}
      onClick={() => {
        tap();
        phone.push({ kind: "device" });
      }}
    >
      <Icon size={compact ? 20 : 18} aria-hidden="true" />
      {!compact && remote && <span className="ph-output__name">Playing on {remote.playerName}</span>}
    </button>
  );
}

export function DeviceSheet() {
  const remote = usePlayer((s) => s.remote);
  const track = usePlayer((s) => s.state?.track ?? null);
  const players = useRemote((s) => s.players);
  const busy = useRemote((s) => s.busy);
  const [help, setHelp] = useState(false);
  useEffect(() => watchPlayers(), []);
  const downloaded = useIsDownloaded(track?.id ?? "");
  const quality = track ? plateText(track, downloaded) : null;
  const others = (players ?? []).filter((p) => p.id !== remote?.playerId && p.online);
  const CardIcon = remote ? KIND_ICON[remote.kind] : Smartphone;
  return (
    <SheetFrame title="Where music plays" onClose={phone.pop}>
      <div className={`ph-device-card ${remote ? "is-remote" : ""}`}>
        <div className="ph-device-card__text">
          <span className="ph-device-card__name">{remote ? remote.playerName : "This phone"}</span>
          {quality && <Plate size="sm">{quality}</Plate>}
          {track && (
            <span className="ph-device-card__now">
              {track.title} · {track.artistTitle}
            </span>
          )}
        </div>
        <CardIcon className="ph-device-card__icon" size={36} aria-hidden="true" />
        {remote && remote.volume !== null && (
          <div className="ph-device-card__volume">
            <Slider label={`Volume on ${remote.playerName}`} value={remote.volume} max={1} step={0.05} onCommit={(v) => void player.setVolume(v)} />
          </div>
        )}
      </div>
      <ul className="ph-rows" aria-label="Other places to play">
        {remote && (
          <DeviceRow name="This phone" line="Play here" Icon={Smartphone} busy={busy === null} onPick={() => void returnHere()} />
        )}
        {others.map((p) => (
          <DeviceRow
            key={p.id}
            name={p.name}
            line={p.unsupported ?? (p.playing ? `Playing ${p.playing}` : VIA_LINE[p.via])}
            Icon={p.via === "cast" ? Cast : KIND_ICON[p.kind]}
            busy={busy === p.id}
            disabled={!!p.unsupported || p.unreachable}
            onPick={() => void moveTo(p)}
          />
        ))}
        {players === null && <li className="ph-rows__note">Looking for devices…</li>}
        {players !== null && others.length === 0 && !remote && <li className="ph-rows__note">No other devices found on your network.</li>}
      </ul>
      <div className="ph-help">
        <button type="button" className="ph-help__toggle" aria-expanded={help} onClick={() => setHelp(!help)}>
          Can't find a device?
          <ChevronDown size={18} aria-hidden="true" className={help ? "is-open" : ""} />
        </button>
        {help && (
          <p className="ph-help__text">
            Make sure the device is on and on the same network. Plex players show while Plex is open on them; Jellyfin cast targets and Navidrome jukebox
            players show once their server is added in Settings.
          </p>
        )}
      </div>
      <button
        type="button"
        className="ph-pill ph-pill--wide"
        onClick={() => toast.info("AirPlay and Bluetooth open from the system's own picker in the phone app.")}
      >
        <Speaker size={18} aria-hidden="true" />
        AirPlay and Bluetooth
      </button>
    </SheetFrame>
  );
}

function DeviceRow({
  name,
  line,
  Icon,
  busy,
  disabled,
  onPick,
}: {
  name: string;
  line: string;
  Icon: typeof Speaker;
  busy: boolean;
  disabled?: boolean;
  onPick: () => void;
}) {
  return (
    <li>
      <button
        type="button"
        className={`ph-row ph-device-row ${busy ? "is-busy" : ""}`}
        disabled={disabled}
        aria-busy={busy}
        onClick={() => {
          tap();
          onPick();
        }}
      >
        <span className="ph-device-row__icon">{busy ? <Loader2 className="ph-spin" size={22} aria-label="Switching" /> : <Icon size={22} aria-hidden="true" />}</span>
        <span className="ph-row__text">
          <span className="ph-row__title">{name}</span>
          <span className="ph-row__sub">{busy ? "Switching…" : line}</span>
        </span>
      </button>
    </li>
  );
}
