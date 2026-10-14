import { Pause, Play } from "lucide-react";
import type { ReactNode } from "react";
import { Art } from "../components/Art";
import { LikedCover } from "../components/LikedCover";
import type { Album, Artist, HomeItem, Playlist } from "../lib/types";
import { player } from "../stores/player";
import { nav } from "../stores/router";
import { useAlbumActions, useArtistActions, useContextState, usePlaylistActions, type ItemActions } from "./cards";

interface TileProps {
  /** Pointer or keyboard focus arrived on the tile (drives the colour wash). */
  onHover: () => void;
}

function Tile({ kind, title, art, actions, onHover }: TileProps & { kind: HomeItem["kind"] | "liked"; title: string; art: ReactNode; actions: Omit<ItemActions, "onContextMenu"> & Partial<Pick<ItemActions, "onContextMenu">> }) {
  return (
    <div
      role="listitem"
      className={`shortcut ${actions.playing ? "is-playing" : ""}`}
      data-kind={kind}
      onPointerEnter={onHover}
      onFocus={onHover}
      onContextMenu={actions.onContextMenu}
    >
      <span className="shortcut__art">{art}</span>
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
  return <Tile kind="album" title={album.title} art={<Art path={album.thumb} size={64} />} actions={useAlbumActions(album)} onHover={onHover} />;
}
function ArtistTile({ artist, onHover }: TileProps & { artist: Artist }) {
  return <Tile kind="artist" title={artist.title} art={<Art path={artist.thumb} size={56} round kind="artist" />} actions={useArtistActions(artist)} onHover={onHover} />;
}
function PlaylistTile({ playlist, onHover }: TileProps & { playlist: Playlist }) {
  return <Tile kind="playlist" title={playlist.title} art={<Art path={playlist.thumb} size={64} kind="playlist" />} actions={usePlaylistActions(playlist)} onHover={onHover} />;
}

/** Liked Songs, always the first tile once something is liked. */
export function LikedTile({ onHover }: TileProps) {
  const { isContext, playing } = useContextState("liked", "liked");
  return (
    <Tile
      kind="liked"
      title="Liked Songs"
      art={<LikedCover size={64} />}
      actions={{ playing, open: () => nav.go({ name: "liked" }), play: () => (isContext ? player.toggle() : player.playLiked()) }}
      onHover={onHover}
    />
  );
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
