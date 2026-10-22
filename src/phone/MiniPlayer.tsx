/**
 * The floating mini player above the tabs: tinted from the cover, the song
 * over its artist, the output button and play or
 * pause, and a thin progress line along the bottom edge (paused too).
 * Tapping it opens now playing.
 */
import { Pause, Play } from "lucide-react";
import type { CSSProperties } from "react";
import { Art } from "../components/Art";
import type { Tap } from "../lib/types";
import { trackArtistLine } from "../lib/format";
import { useArtColours } from "../lib/useTint";
import { player, usePlayer } from "../stores/player";
import { ProgressLine, tap } from "./bits";
import { OutputButton } from "./DeviceSheet";
import { phone } from "./phoneUi";
import { GettingLine } from "../components/GettingLine";
import { followTaps, getting, useTaps } from "../stores/taps";
import { useEffect } from "react";

export function MiniPlayer() {
  const track = usePlayer((s) => s.state?.track ?? null);
  const status = usePlayer((s) => s.state?.status ?? "idle");
  const tapped = useTaps((s) => s.current);
  useEffect(() => followTaps(), []);
  // A tapped song on its way shows here, over what's still playing.
  const coming = getting(tapped) ? tapped : null;
  const colours = useArtColours(null, coming ? coming.thumb : track?.thumb);
  if (coming) return <MiniGetting tap={coming} playing={track?.title ?? null} />;
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

function MiniGetting({ tap: t, playing }: { tap: Tap; playing: string | null }) {
  return (
    <div className="ph-mini" data-testid="mini-player">
      <button type="button" className="ph-mini__open" aria-label={`Getting ${t.title} by ${t.artist}. Open the player`} onClick={phone.openPlayer}>
        <span className="ph-mini__art">
          <Art path={t.thumb} size={44} sleeve={{ title: t.album, artist: t.artist }} />
        </span>
        <span className="ph-mini__text">
          <span className="ph-mini__title">{t.title}</span>
          <span className="ph-mini__artist">
            <span className="ph-mini__artist-name">{playing ? `${t.artist} · still playing ${playing}` : t.artist}</span>
          </span>
        </span>
      </button>
      <GettingLine tap={t} />
    </div>
  );
}
