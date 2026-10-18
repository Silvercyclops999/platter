/** Card wrappers with navigation, play and context menus wired in. */
import type { MouseEvent } from "react";
import { Card } from "../components/Card";
import { openContextMenu } from "../components/ContextMenu";
import { addToPlaylistItem, albumSongs, playlistEditItems } from "../components/PlaylistPicker";
import { shareItem } from "../components/ShareMenu";
import { plexEditItem } from "../components/PlexEditMenu";
import { tagAlbumItems, tagArtistItem } from "../components/TagEditMenu";
import { ShelfMark, physicalItem } from "../components/PhysicalDialog";
import { isCreditId } from "../lib/credits";
import { ExplicitBadge } from "../components/FileTagBadges";
import { releaseTypeLabel } from "../lib/format";
import { albumDrag, playlistDrag } from "../stores/drag";
import { useAlbumBadges, useEditions } from "../stores/fileTags";
import { useOwned } from "../stores/physical";
import { useSettings } from "../stores/settings";
import type { Album, Artist, HomeItem, PlayContextKind, Playlist } from "../lib/types";
import { usePins } from "../stores/pins";
import { player, usePlayer } from "../stores/player";
import { nav } from "../stores/router";
import { moveToSectionItem } from "../stores/archive";

/** Whether this album / artist / playlist is the current play context, and playing. */
export function useContextState(kind: PlayContextKind, id: string): { isContext: boolean; playing: boolean } {
  const isContext = usePlayer((s) => s.state?.queue.context?.kind === kind && s.state.queue.context.id === id);
  const playing = usePlayer((s) => isContext && (s.state?.status === "playing" || s.state?.status === "buffering" || s.state?.status === "loading"));
  return { isContext, playing };
}

/** Open, play / pause and the context menu for one album, artist or playlist. */
export interface ItemActions {
  playing: boolean;
  open: () => void;
  play: () => void;
  onContextMenu: (e: MouseEvent<HTMLElement>) => void;
}

export function useAlbumActions(album: Album): ItemActions & { openMenu: (at: { clientX: number; clientY: number } | HTMLElement) => void } {
  const pinned = usePins((s) => s.isPinned("album", album.id));
  const { isContext, playing } = useContextState("album", album.id);
  const play = () => (isContext ? player.toggle() : player.playAlbum(album.id));
  return {
    playing,
    play,
    open: () => nav.go({ name: "album", id: album.id }),
    onContextMenu: (e) => {
      e.preventDefault();
      openMenu(e);
    },
    openMenu,
  };

  /** The album's menu at the pointer, or under a button. */
  function openMenu(at: { clientX: number; clientY: number } | HTMLElement) {
    openContextMenu(
      `Options for ${album.title}`,
      [
        { label: playing ? "Pause" : "Play", onSelect: play },
        { label: "Add to queue", onSelect: () => player.queueAlbum(album.id) },
        addToPlaylistItem(albumSongs(album.id)),
        { label: "Go to artist", onSelect: () => nav.go({ name: "artist", id: album.artistId }) },
        { label: "Go to artist radio", onSelect: () => nav.go({ name: "radio", seed: `artist:${album.artistId}` }) },
        moveToSectionItem(album),
        shareItem("album", album.id, album.thumb),
        physicalItem({ albumId: album.id, title: album.title, artist: album.artistTitle, thumb: album.thumb }),
        {
          label: pinned ? "Unpin from sidebar" : "Pin to sidebar",
          onSelect: () =>
            usePins.getState().toggle({ kind: "album", id: album.id, title: album.title, subtitle: `Album · ${album.artistTitle}`, thumb: album.thumb }),
        },
        plexEditItem({ id: album.id, kind: "album", title: album.title, thumb: album.thumb, artistTitle: album.artistTitle }),
        ...tagAlbumItems(album),
      ],
      at,
    );
  }
}

export function useArtistActions(artist: Artist): ItemActions {
  const pinned = usePins((s) => s.isPinned("artist", artist.id));
  const { isContext, playing } = useContextState("artist", artist.id);
  const play = () => (isContext ? player.toggle() : player.playArtist(artist.id));
  return {
    playing,
    play,
    open: () => nav.go({ name: "artist", id: artist.id }),
    onContextMenu: (e) => {
      e.preventDefault();
      openContextMenu(
        `Options for ${artist.title}`,
        [
          { label: playing ? "Pause" : "Play", onSelect: play },
          { label: "Go to artist radio", onSelect: () => nav.go({ name: "radio", seed: `artist:${artist.id}` }) },
          shareItem("artist", artist.id, artist.thumb),
          {
            label: pinned ? "Unpin from sidebar" : "Pin to sidebar",
            onSelect: () => usePins.getState().toggle({ kind: "artist", id: artist.id, title: artist.title, subtitle: "Artist", thumb: artist.thumb }),
          },
          ...(isCreditId(artist.id) ? [] : [plexEditItem({ id: artist.id, kind: "artist", title: artist.title, thumb: artist.thumb })]),
          ...(isCreditId(artist.id) ? [] : tagArtistItem(artist)),
        ],
        e,
      );
    },
  };
}

export function usePlaylistActions(playlist: Playlist): ItemActions {
  const pinned = usePins((s) => s.isPinned("playlist", playlist.id));
  const { isContext, playing } = useContextState("playlist", playlist.id);
  const play = () => (isContext ? player.toggle() : player.playPlaylist(playlist.id));
  return {
    playing,
    play,
    open: () => nav.go({ name: "playlist", id: playlist.id }),
    onContextMenu: (e) => {
      e.preventDefault();
      openContextMenu(
        `Options for ${playlist.title}`,
        [
          { label: playing ? "Pause" : "Play", onSelect: play },
          { label: "Add to queue", onSelect: () => player.queuePlaylist(playlist.id) },
          ...playlistEditItems(playlist),
          shareItem("playlist", playlist.id, playlist.thumb),
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
    },
  };
}

export function playlistSubtitle(playlist: Playlist): string {
  return `${playlist.smart ? "Smart playlist" : "Playlist"} · ${playlist.trackCount} songs`;
}

export function AlbumCard({ album, subtitle = "artist", note }: { album: Album; subtitle?: "artist" | "year" | "type"; note?: string }) {
  const a = useAlbumActions(album);
  const tags = useAlbumBadges(album.id);
  const owned = useOwned(album.id);
  const editions = useEditions(album.id).length;
  const groupEditions = useSettings((s) => s.settings?.groupEditions !== false);
  const text = note
    ? `${note} · ${releaseTypeLabel(album)}`
    : subtitle === "year"
      ? `${album.year ?? "Unreleased"} · ${releaseTypeLabel(album)}`
      : subtitle === "type"
        ? `${releaseTypeLabel(album)} · ${album.artistTitle}`
        : `${album.year ? album.year + " · " : ""}${album.artistTitle}`;
  // A collapsed group says how many editions sit behind it.
  const withEditions = groupEditions && editions > 1 ? `${text} · ${editions} editions` : text;
  const sub = tags?.explicit || owned ? (
    <>
      {owned && <ShelfMark formats={owned.formats} />}
      {tags?.explicit && <ExplicitBadge />}
      {withEditions}
    </>
  ) : (
    withEditions
  );
  return (
    <Card
      kind="album"
      title={album.title}
      subtitle={sub}
      thumb={album.thumb}
      sleeve={{ title: album.title, artist: album.artistTitle }}
      playing={a.playing}
      onOpen={a.open}
      onPlay={a.play}
      onContextMenu={a.onContextMenu}
      drag={() => albumDrag(album.id, album.title)}
    />
  );
}

export function ArtistCard({ artist }: { artist: Artist }) {
  const a = useArtistActions(artist);
  return (
    <Card
      kind="artist"
      title={artist.title}
      subtitle="Artist"
      thumb={artist.thumb}
      playing={a.playing}
      onOpen={a.open}
      onPlay={a.play}
      onContextMenu={a.onContextMenu}
    />
  );
}

export function PlaylistCard({ playlist }: { playlist: Playlist }) {
  const a = usePlaylistActions(playlist);
  return (
    <Card
      kind="playlist"
      title={playlist.title}
      subtitle={playlistSubtitle(playlist)}
      thumb={playlist.thumb}
      playing={a.playing}
      onOpen={a.open}
      onPlay={a.play}
      onContextMenu={a.onContextMenu}
      drag={() => playlistDrag(playlist.id, playlist.title, playlist.trackCount)}
    />
  );
}

/**
 * Card for a mixed Home item. In mixed rows the subtitle leads with the type
 * ("Album · artist", "Artist", "Playlist · 12 songs").
 */
export function HomeItemCard({ item }: { item: HomeItem }) {
  switch (item.kind) {
    case "album":
      return <AlbumCard album={item.album} subtitle="type" />;
    case "artist":
      return <ArtistCard artist={item.artist} />;
    case "playlist":
      return <PlaylistCard playlist={item.playlist} />;
  }
}

/** Key, title and art path of a Home item. */
export function homeItemInfo(item: HomeItem): { key: string; title: string; thumb: string | null } {
  switch (item.kind) {
    case "album":
      return { key: `album:${item.album.id}`, title: item.album.title, thumb: item.album.thumb };
    case "artist":
      return { key: `artist:${item.artist.id}`, title: item.artist.title, thumb: item.artist.thumb };
    case "playlist":
      return { key: `playlist:${item.playlist.id}`, title: item.playlist.title, thumb: item.playlist.thumb };
  }
}
