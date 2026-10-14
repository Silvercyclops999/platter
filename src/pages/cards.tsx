/** Card wrappers with navigation, play and context menus wired in. */
import type { MouseEvent } from "react";
import { Card } from "../components/Card";
import { openContextMenu } from "../components/ContextMenu";
import { releaseTypeLabel } from "../lib/format";
import type { Album, Artist, PlayContextKind, Playlist } from "../lib/types";
import { usePins } from "../stores/pins";
import { player, usePlayer } from "../stores/player";
import { nav } from "../stores/router";

/** Whether this album / artist / playlist is the current play context, and playing. */
export function useContextState(kind: PlayContextKind, id: string): { isContext: boolean; playing: boolean } {
  const isContext = usePlayer((s) => s.state?.queue.context?.kind === kind && s.state.queue.context.id === id);
  const playing = usePlayer((s) => isContext && (s.state?.status === "playing" || s.state?.status === "buffering" || s.state?.status === "loading"));
  return { isContext, playing };
}

export function AlbumCard({ album, subtitle = "artist" }: { album: Album; subtitle?: "artist" | "year" }) {
  const pinned = usePins((s) => s.isPinned("album", album.id));
  const { isContext, playing } = useContextState("album", album.id);
  const sub =
    subtitle === "year"
      ? `${album.year ?? "Unreleased"} · ${releaseTypeLabel(album)}`
      : `${album.year ? album.year + " · " : ""}${album.artistTitle}`;
  const onContextMenu = (e: MouseEvent<HTMLElement>) => {
    e.preventDefault();
    openContextMenu(
      `Options for ${album.title}`,
      [
        { label: playing ? "Pause" : "Play", onSelect: () => (isContext ? player.toggle() : player.playAlbum(album.id)) },
        { label: "Add to queue", onSelect: () => player.queueAlbum(album.id) },
        { label: "Go to artist", onSelect: () => nav.go({ name: "artist", id: album.artistId }) },
        {
          label: pinned ? "Unpin from sidebar" : "Pin to sidebar",
          onSelect: () =>
            usePins.getState().toggle({ kind: "album", id: album.id, title: album.title, subtitle: `Album · ${album.artistTitle}`, thumb: album.thumb }),
        },
      ],
      e,
    );
  };
  return (
    <Card
      kind="album"
      title={album.title}
      subtitle={sub}
      thumb={album.thumb}
      playing={playing}
      onOpen={() => nav.go({ name: "album", id: album.id })}
      onPlay={() => (isContext ? player.toggle() : player.playAlbum(album.id))}
      onContextMenu={onContextMenu}
    />
  );
}

export function ArtistCard({ artist }: { artist: Artist }) {
  const pinned = usePins((s) => s.isPinned("artist", artist.id));
  const { isContext, playing } = useContextState("artist", artist.id);
  return (
    <Card
      kind="artist"
      title={artist.title}
      subtitle="Artist"
      thumb={artist.thumb}
      playing={playing}
      onOpen={() => nav.go({ name: "artist", id: artist.id })}
      onPlay={() => (isContext ? player.toggle() : player.playArtist(artist.id))}
      onContextMenu={(e) => {
        e.preventDefault();
        openContextMenu(
          `Options for ${artist.title}`,
          [
            { label: playing ? "Pause" : "Play", onSelect: () => (isContext ? player.toggle() : player.playArtist(artist.id)) },
            {
              label: pinned ? "Unpin from sidebar" : "Pin to sidebar",
              onSelect: () => usePins.getState().toggle({ kind: "artist", id: artist.id, title: artist.title, subtitle: "Artist", thumb: artist.thumb }),
            },
          ],
          e,
        );
      }}
    />
  );
}

export function PlaylistCard({ playlist }: { playlist: Playlist }) {
  const pinned = usePins((s) => s.isPinned("playlist", playlist.id));
  const { isContext, playing } = useContextState("playlist", playlist.id);
  return (
    <Card
      kind="playlist"
      title={playlist.title}
      subtitle={`${playlist.smart ? "Smart playlist" : "Playlist"} · ${playlist.trackCount} songs`}
      thumb={playlist.thumb}
      playing={playing}
      onOpen={() => nav.go({ name: "playlist", id: playlist.id })}
      onPlay={() => (isContext ? player.toggle() : player.playPlaylist(playlist.id))}
      onContextMenu={(e) => {
        e.preventDefault();
        openContextMenu(
          `Options for ${playlist.title}`,
          [
            { label: playing ? "Pause" : "Play", onSelect: () => (isContext ? player.toggle() : player.playPlaylist(playlist.id)) },
            { label: "Add to queue", onSelect: () => player.queuePlaylist(playlist.id) },
            {
              label: pinned ? "Unpin from sidebar" : "Pin to sidebar",
              onSelect: () =>
                usePins.getState().toggle({
                  kind: "playlist",
                  id: playlist.id,
                  title: playlist.title,
                  subtitle: `Playlist · ${playlist.trackCount} songs`,
                  thumb: playlist.thumb,
                }),
            },
          ],
          e,
        );
      }}
    />
  );
}
