import { ListPlus, Plus, UserPlus, Users } from "lucide-react";
import { useMemo, useRef, useState, type KeyboardEvent } from "react";
import { backend } from "../lib/backend";
import type { Playlist, SharedSummary } from "../lib/types";
import { useApp } from "../stores/app";
import { openCircleDialog, sharedNow, useCircle } from "../stores/circle";
import { useSettings } from "../stores/settings";
import { addToPlaylist, askDelete, createPlaylist, duplicatePlaylist, editDetails, lastPlaylistId } from "../stores/playlists";
import type { MenuItem, SubmenuContext } from "../stores/ui";
import { Art } from "./Art";

/** Playlists shared with you that you can add to, as playlists. */
export function collaboratorPlaylists(shared: SharedSummary[]): Playlist[] {
  return shared
    .filter((s) => s.role === "collaborator" && !s.gone)
    .map((s) => ({ id: s.id, title: s.title, smart: false, trackCount: s.trackCount, durationMs: 0, thumb: null, updatedAt: 0 }));
}

/** Playlists you can add to, most recently changed first. */
export function editablePlaylists(all: Playlist[], query: string, exclude?: string): Playlist[] {
  const q = query.trim().toLowerCase();
  return all
    .filter((p) => !p.smart && p.id !== exclude && (!q || p.title.toLowerCase().includes(q)))
    .sort((a, b) => (b.updatedAt ?? 0) - (a.updatedAt ?? 0) || a.title.localeCompare(b.title));
}

/**
 * The "Add to playlist" submenu: a find field, New playlist, then your
 * playlists. `songs` gives the track ids when one is picked (an album's
 * tracks are loaded then).
 */
export function PlaylistPicker({ songs, ctx, exclude }: { songs: () => Promise<string[]> | string[]; ctx: SubmenuContext; exclude?: string }) {
  const all = useApp((s) => s.playlists);
  const shared = useCircle((s) => s.view.playlists);
  const [query, setQuery] = useState("");
  // Your playlists, and the ones shared with you as a collaborator (a
  // listener can't add, so theirs aren't here).
  const list = useMemo(() => editablePlaylists([...(all ?? []), ...collaboratorPlaylists(shared)], query, exclude), [all, shared, query, exclude]);
  const root = useRef<HTMLDivElement>(null);

  const pick = (run: (ids: string[]) => unknown) => {
    ctx.close();
    Promise.resolve(songs()).then((ids) => ids.length && run(ids));
  };

  const focusables = () => Array.from(root.current?.querySelectorAll<HTMLElement>("input, [role='menuitem']") ?? []);
  const onKeyDown = (e: KeyboardEvent) => {
    if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
    e.preventDefault();
    const els = focusables();
    const i = els.indexOf(document.activeElement as HTMLElement);
    els[(i + (e.key === "ArrowDown" ? 1 : -1) + els.length) % els.length]?.focus();
  };

  return (
    <div ref={root} className="pl-picker" onKeyDown={onKeyDown}>
      <input
        className="pl-picker__find"
        type="search"
        placeholder="Find a playlist"
        aria-label="Find a playlist"
        value={query}
        spellCheck={false}
        autoComplete="off"
        onChange={(e) => setQuery(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter" && list[0]) {
            e.preventDefault();
            const first = list[0];
            pick((ids) => addToPlaylist(first, ids));
          }
        }}
      />
      <button type="button" role="menuitem" tabIndex={-1} className="context-menu__item pl-picker__new" onClick={() => pick((ids) => createPlaylist({ trackIds: ids }))}>
        <span className="pl-picker__plus" aria-hidden="true">
          <Plus size={16} />
        </span>
        <span className="truncate">New playlist</span>
      </button>
      <div className="context-menu__sep" role="separator" />
      <div className="pl-picker__list">
        {list.map((p) => (
          <button key={p.id} type="button" role="menuitem" tabIndex={-1} className="context-menu__item pl-picker__item" onClick={() => pick((ids) => addToPlaylist(p, ids))}>
            <Art path={p.thumb} size={24} kind="playlist" />
            <span className="truncate">{p.title}</span>
          </button>
        ))}
        {list.length === 0 && <p className="pl-picker__none">{query ? `No playlists match "${query}"` : "No playlists yet"}</p>}
      </div>
    </div>
  );
}

/** An album's track ids, loaded when a playlist is picked. */
export const albumSongs = (albumId: string) => () => backend.albumPage(albumId).then((p) => p.tracks.map((t) => t.id));
/** A playlist's track ids, loaded when a playlist is picked. */
export const playlistSongs = (playlistId: string) => () => backend.playlistPage(playlistId).then((p) => p.tracks.map((t) => t.id));

/**
 * Editing entries for a playlist's menus (page, card, sidebar): edit
 * details, delete, add its songs to another playlist. A smart playlist is
 * read only: it can be duplicated into an ordinary one.
 */
export function playlistEditItems(p: Playlist): MenuItem[] {
  if (p.smart) return [{ label: "Duplicate", onSelect: () => void duplicatePlaylist(p) }];
  return [
    ...collabItems(p),
    { label: "Edit details", onSelect: () => editDetails(p) },
    { label: "Delete", onSelect: () => askDelete(p) },
    ...addToPlaylistItems(playlistSongs(p.id), { label: "Add to other playlist", exclude: p.id }),
  ];
}

/** Invite collaborators (and the permissions, once it's shared) for your own playlist. */
export function collabItems(p: Pick<Playlist, "id" | "title">): MenuItem[] {
  if (useSettings.getState().settings?.collabPlaylists === false) return [];
  const shared = sharedNow(p.id);
  return [
    { label: "Invite collaborators", icon: UserPlus, onSelect: () => openCircleDialog({ kind: "invite", playlistId: p.id, title: p.title }) },
    ...(shared ? [{ label: "Playlist permissions", icon: Users, onSelect: () => openCircleDialog({ kind: "permissions", playlistId: p.id }) }] : []),
  ];
}

/**
 * "Add to <the playlist you last added to>" in one click, when it's still
 * there and can take songs, then the entry that opens the picker.
 */
export function addToPlaylistItems(songs: () => Promise<string[]> | string[], opts: { label?: string; exclude?: string } = {}): MenuItem[] {
  const picker = addToPlaylistItem(songs, opts);
  const id = lastPlaylistId();
  if (!id) return [picker];
  const all = [...(useApp.getState().playlists ?? []), ...collaboratorPlaylists(useCircle.getState().view.playlists)];
  const last = editablePlaylists(all, "", opts.exclude).find((p) => p.id === id);
  if (!last) return [picker];
  return [
    {
      label: `Add to ${last.title}`,
      icon: ListPlus,
      onSelect: () => void Promise.resolve(songs()).then((ids) => (ids.length ? addToPlaylist(last, ids) : undefined)),
    },
    picker,
  ];
}

/** The menu entry that opens the picker. */
export function addToPlaylistItem(songs: () => Promise<string[]> | string[], opts: { label?: string; exclude?: string } = {}): MenuItem {
  return {
    label: opts.label ?? "Add to playlist",
    submenu: (ctx) => <PlaylistPicker songs={songs} ctx={ctx} exclude={opts.exclude} />,
  };
}
