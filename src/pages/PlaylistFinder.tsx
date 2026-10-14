import { Check, X } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { Art } from "../components/Art";
import { IconButton } from "../components/Button";
import { SearchField } from "../components/SearchField";
import { backend } from "../lib/backend";
import { formatDuration, trackArtistLine } from "../lib/format";
import type { Playlist, Track } from "../lib/types";
import { addNow } from "../stores/playlists";

const SEARCH_DELAY_MS = 150;

/**
 * "Let's find something for your playlist": search songs and add them with
 * one click. It stays open after adding, and a song the playlist already has
 * shows "Already added" instead of Add.
 */
export function PlaylistFinder({
  playlist,
  have,
  onClose,
  autoFocus = false,
}: {
  playlist: Playlist;
  /** Track ids already in the playlist. */
  have: ReadonlySet<string>;
  onClose?: () => void;
  autoFocus?: boolean;
}) {
  const [query, setQuery] = useState("");
  const [songs, setSongs] = useState<Track[] | null>(null);
  // Added during this visit, so the button changes before the page reloads.
  const [added, setAdded] = useState<ReadonlySet<string>>(new Set());
  const seq = useRef(0);

  useEffect(() => {
    const q = query.trim();
    const n = ++seq.current;
    if (!q) {
      setSongs(null);
      return;
    }
    const t = setTimeout(() => {
      backend.search({ query: q, filter: "songs", limit: 20 }).then(
        (r) => n === seq.current && setSongs(r.songs),
        () => n === seq.current && setSongs([]),
      );
    }, SEARCH_DELAY_MS);
    return () => clearTimeout(t);
  }, [query]);

  const add = async (t: Track) => {
    setAdded((s) => new Set(s).add(t.id));
    const ok = await addNow(playlist, [t.id]);
    if (!ok) setAdded((s) => new Set([...s].filter((x) => x !== t.id)));
  };

  return (
    <section className="pl-finder" aria-labelledby="pl-finder-title">
      <div className="pl-finder__head">
        <div>
          <h2 id="pl-finder-title" className="pl-finder__title">
            Let's find something for your playlist
          </h2>
          <SearchField value={query} onChange={setQuery} label="Search for songs" placeholder="Search for songs" autoFocus={autoFocus} className="pl-finder__search" />
        </div>
        {onClose && <IconButton label="Close the song finder" icon={<X size={20} />} onClick={onClose} />}
      </div>
      {songs && songs.length === 0 && <p className="pl-finder__none">No songs match "{query.trim()}".</p>}
      {songs && songs.length > 0 && (
        <ul className="pl-finder__list" aria-label="Songs found">
          {songs.map((t) => {
            const already = have.has(t.id) || added.has(t.id);
            return (
              <li key={t.id} className="pl-finder__row">
                <Art path={t.thumb} size={40} />
                <div className="pl-finder__text">
                  <div className="pl-finder__song truncate">{t.title}</div>
                  <div className="pl-finder__artist truncate">{trackArtistLine(t)}</div>
                </div>
                <div className="pl-finder__album truncate">{t.albumTitle}</div>
                <span className="pl-finder__dur tabular">{formatDuration(t.durationMs)}</span>
                {already ? (
                  <button type="button" className="pl-finder__add is-added" aria-disabled="true" aria-label={`${t.title} is already in ${playlist.title}`}>
                    <Check size={14} aria-hidden="true" />
                    Already added
                  </button>
                ) : (
                  <button type="button" className="pl-finder__add" aria-label={`Add ${t.title} to ${playlist.title}`} onClick={() => void add(t)}>
                    Add
                  </button>
                )}
              </li>
            );
          })}
        </ul>
      )}
    </section>
  );
}
