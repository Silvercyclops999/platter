import { PenLine } from "lucide-react";
import type { CSSProperties } from "react";
import { Button } from "../components/Button";
import { Skeleton } from "../components/Skeleton";
import { NEUTRAL_HEADER } from "../lib/color";
import { openLyricsEditor } from "../stores/lyricsEditor";
import { usePlayer } from "../stores/player";
import { useSettings } from "../stores/settings";
import { useLyrics } from "../shell/lyrics";
import { LyricsView } from "../shell/LyricsView";
import { TranscribeOffer } from "../shell/TranscribeOffer";
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
  const showAdd = useSettings((s) => s.settings?.lyricsShowAdd ?? true);

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
  else if (!lyrics)
    body = (
      <div className="lyrics-page__empty-wrap">
        <p className="lyrics-page__empty">No lyrics for this song</p>
        {showAdd && (
          <Button variant="primary" icon={<PenLine size={16} />} onClick={() => openLyricsEditor(track)}>
            Add lyrics
          </Button>
        )}
        <TranscribeOffer trackId={track.id} />
      </div>
    );
  else body = <LyricsView lyrics={lyrics} track={track} />;

  return (
    <section className="lyrics-page" aria-label={track ? `Lyrics: ${track.title}` : "Lyrics"} style={{ "--lyrics-bg": bg } as CSSProperties}>
      {body}
    </section>
  );
}
