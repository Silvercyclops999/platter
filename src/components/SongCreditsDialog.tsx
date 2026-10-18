/**
 * Credits for one song (from a song's ⋯ menu and the expanded view's):
 * who made it by role, when it was recorded, and where each fact came
 * from. Names open their credit page.
 */
import { formatSleeveDate } from "../lib/sleeve";
import type { Track } from "../lib/types";
import { useSongCredits } from "../lib/useSleeveNotes";
import { nav } from "../stores/router";
import { closeSongCredits, useSongCreditsDialog } from "../stores/songCredits";
import { RoleLines, SleeveHint, SourceMarks } from "../pages/SleeveNotes";
import { Button } from "./Button";
import { Dialog } from "./Dialog";
import { Skeleton } from "./Skeleton";
import "../pages/sleeve.css";

function SongCreditsDialog({ track }: { track: Track }) {
  const { data, loading, failed } = useSongCredits(track.id);
  const song = data?.song ?? null;
  const known = !!song && (song.groups.length > 0 || !!song.recorded);
  return (
    <Dialog open onClose={closeSongCredits} label={`Credits for ${track.title}`} className="song-credits">
      <span className="song-credits__kicker">Credits</span>
      <h2 className="song-credits__title">{track.title}</h2>
      <p className="song-credits__sub truncate">
        {track.artistTitle} · {track.albumTitle}
      </p>
      <div className="song-credits__body">
        {loading && !data ? (
          <div aria-busy="true">
            <Skeleton width="70%" height={14} style={{ margin: "10px 0" }} />
            <Skeleton width="50%" height={14} style={{ margin: "10px 0" }} />
          </div>
        ) : failed ? (
          <p className="sleeve-muted" role="alert">
            Couldn't read this song's credits.
          </p>
        ) : known ? (
          <>
            <RoleLines groups={song!.groups} onOpen={closeSongCredits} />
            {song!.recorded && (
              <p className="sleeve-muted">
                Recorded {formatSleeveDate(song!.recorded.value)} <SourceMarks sources={song!.recorded.sources} />
              </p>
            )}
          </>
        ) : data?.looking ? (
          <p className="sleeve-muted" role="status">
            Looking for credits…
          </p>
        ) : (
          <>
            <p className="sleeve-muted">No credits are known for this song yet.</p>
            {data && <SleeveHint lookups={data.lookups} empty />}
          </>
        )}
        {known && data?.looking && (
          <p className="sleeve-muted" role="status">
            Looking for more credits…
          </p>
        )}
      </div>
      <div className="song-credits__actions">
        <Button
          size="sm"
          variant="ghost"
          onClick={() => {
            closeSongCredits();
            nav.go({ name: "album", id: track.albumId, tab: "sleeve" });
          }}
        >
          Album sleeve notes
        </Button>
        <Button size="sm" variant="secondary" onClick={closeSongCredits}>
          Close
        </Button>
      </div>
    </Dialog>
  );
}

export function SongCreditsHost() {
  const track = useSongCreditsDialog((s) => s.track);
  if (!track) return null;
  return <SongCreditsDialog key={track.id} track={track} />;
}
