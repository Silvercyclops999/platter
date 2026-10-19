/**
 * The floating mini player above the tabs: tinted from the cover, the song
 * over its artist with the format plate, the output button and play or
 * pause, and a thin progress line along the bottom edge (paused too).
 * Tapping it opens now playing.
 */
import { Pause, Play } from "lucide-react";
import type { CSSProperties } from "react";
import { Art } from "../components/Art";
import { trackArtistLine } from "../lib/format";
import { useArtColours } from "../lib/useTint";
import { player, usePlayer } from "../stores/player";
import { FormatPlate, ProgressLine, tap } from "./bits";
import { OutputButton } from "./DeviceSheet";
import { phone } from "./phoneUi";

export function MiniPlayer() {
  const track = usePlayer((s) => s.state?.track ?? null);
  const status = usePlayer((s) => s.state?.status ?? "idle");
  const colours = useArtColours(null, track?.thumb);
  if (!track) return null;
  const playing = status === "playing" || status === "loading" || status === "buffering";
  const style = { "--ph-tint": colours?.vivid ?? "var(--color-bg-overlay)" } as CSSProperties;
  return (
    <div className="ph-mini" style={style} data-testid="mini-player">
      <button type="button" className="ph-mini__open" aria-label={`Now playing: ${track.title} by ${trackArtistLine(track)}. Open the player`} onClick={phone.openPlayer}>
        <span className="ph-mini__art">
          <Art path={track.thumb} size={44} sleeve={{ title: track.albumTitle, artist: track.artistTitle }} />
        </span>
        <span className="ph-mini__text">
          <span className="ph-mini__title">{track.title}</span>
          <span className="ph-mini__artist">
            <span className="ph-mini__artist-name">{trackArtistLine(track)}</span>
            <FormatPlate track={track} />
          </span>
        </span>
      </button>
      <OutputButton compact />
      <button
        type="button"
        className="ph-icon-btn ph-mini__play"
        aria-label={playing ? "Pause" : "Play"}
        onClick={() => {
          tap();
          void player.toggle();
        }}
      >
        {playing ? <Pause size={24} fill="currentColor" aria-hidden="true" /> : <Play size={24} fill="currentColor" aria-hidden="true" />}
      </button>
      <ProgressLine className="ph-mini__progress" />
    </div>
  );
}
