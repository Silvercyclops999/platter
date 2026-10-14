import { Check, ChevronDown, Clock3, Pause, Play } from "lucide-react";
import { useId, useState, type ReactNode } from "react";
import { Art } from "../components/Art";
import { TextLink } from "../components/ArtistLinks";
import { LikedCover } from "../components/LikedCover";
import { Skeleton } from "../components/Skeleton";
import { TrackList, TrackRow } from "../components/TrackRow";
import { backend } from "../lib/backend";
import type { Album, Artist, Playlist, RecentsEntry } from "../lib/types";
import { useLoad } from "../lib/useLoad";
import { player, usePlayer } from "../stores/player";
import { nav } from "../stores/router";
import { useAlbumActions, useArtistActions, useContextState, usePlaylistActions, type ItemActions } from "./cards";
import { entryKey, entryLines, groupByDay } from "./recents";
import "./recents.css";

/** What a row's play button and title do. */
interface RowActions {
  playing: boolean;
  play: () => void;
  open?: () => void;
  onContextMenu?: ItemActions["onContextMenu"];
}

function RowView({ entry, art, actions }: { entry: RecentsEntry; art: ReactNode; actions: RowActions }) {
  const [open, setOpen] = useState(false);
  const panel = useId();
  const { title, sub } = entryLines(entry);
  const nowId = usePlayer((s) => s.state?.track?.id);
  const status = usePlayer((s) => s.state?.status);
  const name = title ?? sub;
  // Played order, oldest first, for playing the songs from the list.
  const ids = [...entry.tracks].reverse().map((t) => t.id);
  return (
    <li className={`recents-row ${open ? "is-open" : ""}`}>
      <div className="recents-row__head" onClick={(e) => !(e.target as HTMLElement).closest("button, a, [role=link]") && setOpen(!open)} onContextMenu={actions.onContextMenu}>
        <span className="recents-row__art">
          {art}
          <button
            type="button"
            className={`recents-row__play ${actions.playing ? "is-playing" : ""}`}
            aria-label={`${actions.playing ? "Pause" : "Play"} ${title ?? sub}`}
            onClick={actions.play}
          >
            {actions.playing ? <Pause size={16} fill="currentColor" aria-hidden="true" /> : <Play size={16} fill="currentColor" aria-hidden="true" className="nudge-right" />}
          </button>
        </span>
        <span className="recents-row__text">
          {title &&
            (actions.open ? (
              <TextLink className="recents-row__title truncate" onActivate={actions.open}>
                {title}
              </TextLink>
            ) : (
              <span className="recents-row__title truncate">{title}</span>
            ))}
          <span className={`recents-row__sub truncate ${title ? "" : "recents-row__sub--lead"}`}>
            {entry.kind === "likedAdded" && <Check size={14} strokeWidth={3} className="recents-row__tick" aria-hidden="true" />}
            {sub}
          </span>
        </span>
        <button
          type="button"
          className="recents-row__expand"
          aria-expanded={open}
          aria-controls={panel}
          aria-label={`${open ? "Hide" : "Show"} songs for ${name}`}
          onClick={() => setOpen(!open)}
        >
          <ChevronDown size={20} aria-hidden="true" />
        </button>
      </div>
      <div id={panel} className="recents-row__songs" hidden={!open}>
        {open && (
          <TrackList label={`Songs from ${name}`} header={false}>
            {entry.tracks.map((t, i) => (
              <TrackRow
                key={`${i}:${t.id}`}
                rowKey={`${i}:${t.id}`}
                compact
                track={t}
                index={i + 1}
                active={t.id === nowId}
                playing={t.id === nowId && status === "playing"}
                showAlbum={false}
                onPlay={() => (t.id === nowId ? player.toggle() : player.playTracks(ids, t.id))}
              />
            ))}
          </TrackList>
        )}
      </div>
    </li>
  );
}

function AlbumRow({ entry, album }: { entry: RecentsEntry; album: Album }) {
  return <RowView entry={entry} art={<Art path={album.thumb} size={48} />} actions={useAlbumActions(album)} />;
}
function ArtistRow({ entry, artist }: { entry: RecentsEntry; artist: Artist }) {
  return <RowView entry={entry} art={<Art path={artist.thumb} size={48} round kind="artist" />} actions={useArtistActions(artist)} />;
}
function PlaylistRow({ entry, playlist }: { entry: RecentsEntry; playlist: Playlist }) {
  return <RowView entry={entry} art={<Art path={playlist.thumb} size={48} kind="playlist" />} actions={usePlaylistActions(playlist)} />;
}
function LikedRow({ entry }: { entry: RecentsEntry }) {
  const { isContext, playing } = useContextState("liked", "liked");
  const first = entry.kind === "likedAdded" ? entry.tracks[0]?.id : undefined;
  return (
    <RowView
      entry={entry}
      art={<LikedCover size={48} />}
      actions={{ playing, play: () => (isContext ? player.toggle() : player.playLiked(first)), open: () => nav.go({ name: "liked" }) }}
    />
  );
}
function SongsRow({ entry }: { entry: RecentsEntry }) {
  const ids = [...entry.tracks].reverse().map((t) => t.id);
  return <RowView entry={entry} art={<Art path={entry.tracks[0]?.thumb ?? null} size={48} />} actions={{ playing: false, play: () => player.playTracks(ids) }} />;
}

function EntryRow({ entry }: { entry: RecentsEntry }) {
  const i = entry.item;
  if (entry.kind === "liked" || entry.kind === "likedAdded") return <LikedRow entry={entry} />;
  if (i?.kind === "album") return <AlbumRow entry={entry} album={i.album} />;
  if (i?.kind === "artist") return <ArtistRow entry={entry} artist={i.artist} />;
  if (i?.kind === "playlist") return <PlaylistRow entry={entry} playlist={i.playlist} />;
  return <SongsRow entry={entry} />;
}

/**
 * Recents: what was played on this computer, one row per listening session
 * and where it came from, in day groups. Each row opens to its songs.
 */
export function RecentsPage() {
  const trackId = usePlayer((s) => s.state?.track?.id ?? null);
  // Reloads as songs change, so the page keeps up while it's open.
  const { data, error } = useLoad(() => backend.recents(), [trackId]);
  const groups = data ? groupByDay(data) : [];
  return (
    <div className="page-pad recents" data-testid="recents">
      <h1 className="recents__title">Recents</h1>
      {error && !data ? (
        <p className="page-error">Couldn't load your recents: {error.message}</p>
      ) : !data ? (
        <div aria-hidden="true">
          <Skeleton width={120} height={24} style={{ margin: "24px 0 12px" }} />
          {Array.from({ length: 6 }, (_, i) => (
            <div key={i} className="recents-row__head recents-row--skeleton">
              <Skeleton width={48} height={48} />
              <span className="recents-row__text">
                <Skeleton width="50%" height={14} />
                <Skeleton width="35%" height={12} style={{ marginTop: 6 }} />
              </span>
            </div>
          ))}
        </div>
      ) : groups.length === 0 ? (
        <div className="empty-state" role="status">
          <Clock3 size={40} strokeWidth={1.5} aria-hidden="true" />
          <h2 className="empty-state__title">Nothing played yet</h2>
          <p className="empty-state__text">Albums, playlists and songs you play here show up in this list.</p>
        </div>
      ) : (
        groups.map((g, gi) => (
          <section key={g.label} className="recents__day" aria-labelledby={`recents-day-${gi}`}>
            <h2 id={`recents-day-${gi}`} className="recents__day-title">
              {g.label}
            </h2>
            <ul className="recents__list">
              {g.entries.map((e) => (
                <EntryRow key={entryKey(e)} entry={e} />
              ))}
            </ul>
          </section>
        ))
      )}
    </div>
  );
}
