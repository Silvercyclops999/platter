import { Pause, Play } from "lucide-react";
import { Art } from "../components/Art";
import type { Album, Artist, HomeItem, Playlist } from "../lib/types";
import { useAlbumActions, useArtistActions, usePlaylistActions, type ItemActions } from "./cards";

interface TileProps {
  /** Pointer or keyboard focus arrived on the tile (drives the colour wash). */
  onHover: () => void;
}

function Tile({ kind, title, thumb, actions, onHover }: TileProps & { kind: HomeItem["kind"]; title: string; thumb: string | null; actions: ItemActions }) {
  return (
    <div
      role="listitem"
      className={`shortcut ${actions.playing ? "is-playing" : ""}`}
      data-kind={kind}
      onPointerEnter={onHover}
      onFocus={onHover}
      onContextMenu={actions.onContextMenu}
    >
      <span className="shortcut__art">
        <Art path={thumb} size={kind === "artist" ? 56 : 64} round={kind === "artist"} kind={kind} />
      </span>
      <button type="button" className="shortcut__link" onClick={actions.open} title={title}>
        <span className="shortcut__title">{title}</span>
      </button>
      <button
        type="button"
        className="shortcut__play"
        aria-label={`${actions.playing ? "Pause" : "Play"} ${title}`}
        onClick={(e) => {
          e.stopPropagation();
          actions.play();
        }}
      >
        {actions.playing ? <Pause size={16} fill="currentColor" aria-hidden="true" /> : <Play size={16} fill="currentColor" aria-hidden="true" className="nudge-right" />}
      </button>
    </div>
  );
}

function AlbumTile({ album, onHover }: TileProps & { album: Album }) {
  return <Tile kind="album" title={album.title} thumb={album.thumb} actions={useAlbumActions(album)} onHover={onHover} />;
}
function ArtistTile({ artist, onHover }: TileProps & { artist: Artist }) {
  return <Tile kind="artist" title={artist.title} thumb={artist.thumb} actions={useArtistActions(artist)} onHover={onHover} />;
}
function PlaylistTile({ playlist, onHover }: TileProps & { playlist: Playlist }) {
  return <Tile kind="playlist" title={playlist.title} thumb={playlist.thumb} actions={usePlaylistActions(playlist)} onHover={onHover} />;
}

/**
 * One shortcut in Home's grid: cover square on the left, bold title, a
 * translucent tile that lightens on hover, and a play button that fades in
 * at the right end.
 */
export function ShortcutTile({ item, onHover }: TileProps & { item: HomeItem }) {
  switch (item.kind) {
    case "album":
      return <AlbumTile album={item.album} onHover={onHover} />;
    case "artist":
      return <ArtistTile artist={item.artist} onHover={onHover} />;
    case "playlist":
      return <PlaylistTile playlist={item.playlist} onHover={onHover} />;
  }
}

export function ShortcutSkeleton() {
  return (
    <div className="shortcut shortcut--skeleton" aria-hidden="true">
      <span className="shortcut__art">
        <span className="art" style={{ width: 64, height: 64 }}>
          <span className="art__skeleton skeleton" />
        </span>
      </span>
      <span className="shortcut__skeleton-line skeleton" />
    </div>
  );
}
