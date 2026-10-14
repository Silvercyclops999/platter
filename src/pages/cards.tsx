/** Card wrappers with navigation, play and context menus wired in. */
import type { MouseEvent } from "react";
import { Card } from "../components/Card";
import { openContextMenu } from "../components/ContextMenu";
import { releaseTypeLabel } from "../lib/format";
import type { Album, Artist, Playlist } from "../lib/types";
import { usePins } from "../stores/pins";
import { player } from "../stores/player";
import { nav } from "../stores/router";

export function AlbumCard({ album, subtitle = "artist" }: { album: Album; subtitle?: "artist" | "year" }) {
  const pinned = usePins((s) => s.isPinned("album", album.id));
  const sub =
    subtitle === "year"
      ? `${album.year ?? "Unreleased"} · ${releaseTypeLabel(album)}`
      : `${album.year ? album.year + " · " : ""}${album.artistTitle}`;
  const onContextMenu = (e: MouseEvent<HTMLElement>) => {
    e.preventDefault();
    openContextMenu(
      `Options for ${album.title}`,
      [
        { label: "Play", onSelect: () => player.playAlbum(album.id) },
        { label: "Go to artist", separator: true, onSelect: () => nav.go({ name: "artist", id: album.artistId }) },
        {
          label: pinned ? "Unpin from sidebar" : "Pin to sidebar",
          separator: true,
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
      onOpen={() => nav.go({ name: "album", id: album.id })}
      onPlay={() => player.playAlbum(album.id)}
      onContextMenu={onContextMenu}
    />
  );
}

export function ArtistCard({ artist }: { artist: Artist }) {
  const pinned = usePins((s) => s.isPinned("artist", artist.id));
  return (
    <Card
      kind="artist"
      title={artist.title}
      subtitle="Artist"
      thumb={artist.thumb}
      onOpen={() => nav.go({ name: "artist", id: artist.id })}
      onPlay={() => player.playArtist(artist.id)}
      onContextMenu={(e) => {
        e.preventDefault();
        openContextMenu(
          `Options for ${artist.title}`,
          [
            { label: "Play", onSelect: () => player.playArtist(artist.id) },
            {
              label: pinned ? "Unpin from sidebar" : "Pin to sidebar",
              separator: true,
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
  return (
    <Card
      kind="playlist"
      title={playlist.title}
      subtitle={`${playlist.smart ? "Smart playlist" : "Playlist"} · ${playlist.trackCount} songs`}
      thumb={playlist.thumb}
      onOpen={() => nav.go({ name: "playlist", id: playlist.id })}
      onPlay={() => player.playPlaylist(playlist.id)}
      onContextMenu={(e) => {
        e.preventDefault();
        openContextMenu(
          `Options for ${playlist.title}`,
          [
            { label: "Play", onSelect: () => player.playPlaylist(playlist.id) },
            {
              label: pinned ? "Unpin from sidebar" : "Pin to sidebar",
              separator: true,
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
