import type { CSSProperties } from "react";
import { Skeleton } from "../components/Skeleton";
import { NEUTRAL_HEADER } from "../lib/color";
import { usePlayer } from "../stores/player";
import { useSettings } from "../stores/settings";
import { useLyrics } from "../shell/lyrics";
import { LyricsView } from "../shell/LyricsView";
import { useCoverColors } from "../shell/nowPlaying";
import "../shell/lyrics.css";

/** Colour behind the lyrics: from the cover, or neutral when Artwork colours is off. */
export function useLyricsBackground(thumb: string | null | undefined): string {
  const colours = useSettings((s) => s.settings?.artworkColours !== false);
  const c = useCoverColors(colours ? thumb : null);
  return (colours && c?.vivid) || NEUTRAL_HEADER.vivid;
}

/** The lyrics view in the main column, for whatever is playing. */
export function LyricsPage() {
  const track = usePlayer((s) => s.state?.track);
  const { lyrics, loading } = useLyrics(track?.id);
  const bg = useLyricsBackground(track?.thumb);

  let body;
  if (!track) body = <p className="lyrics-page__empty">Nothing playing.</p>;
  else if (loading)
    body = (
      <div className="lyrics-page__loading" aria-label="Loading lyrics">
        {[70, 55, 80, 45].map((w, i) => (
          <Skeleton key={i} width={`${w}%`} height={32} />
        ))}
      </div>
    );
  else if (!lyrics) body = <p className="lyrics-page__empty">No lyrics for this song</p>;
  else body = <LyricsView lyrics={lyrics} trackId={track.id} />;

  return (
    <section className="lyrics-page" aria-label={track ? `Lyrics: ${track.title}` : "Lyrics"} style={{ "--lyrics-bg": bg } as CSSProperties}>
      {body}
    </section>
  );
}
