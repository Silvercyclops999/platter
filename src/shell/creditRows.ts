/**
 * A song's credits as rows (name, roles), from the same sources the now
 * playing panel reads: the song's own artists, the server's details and the
 * album's sleeve notes. Gatefold's credits page and Console's details pane
 * use it.
 */
import { useTrackCredits } from "../components/ArtistLinks";
import { isOffline } from "../lib/offline";
import type { Track } from "../lib/types";
import { songCredits, useAlbumCredits } from "../lib/useAlbumCredits";
import { useApp } from "../stores/app";
import { buildCredits, useNowPlayingDetails, withAlbumCredits, type CreditRow } from "./nowPlaying";

export function useCreditRows(track: Track): { rows: CreditRow[]; loading: boolean } {
  const lookup = useApp((s) => s.artistLookup);
  const offline = useApp((s) => isOffline(s.health));
  const own = useTrackCredits(track);
  const { details, loading } = useNowPlayingDetails(offline ? null : track.id);
  const album = useAlbumCredits(track.albumId);
  const rows = withAlbumCredits(buildCredits(own, details?.credits ?? [], lookup), songCredits(album, track.id));
  return { rows, loading };
}
