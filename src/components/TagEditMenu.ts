/**
 * Menu entries for the tag editor: "Edit tags..." and "Check tags..." on
 * albums, "Edit tags..." on a song, "Check tags..." on artists, and marking
 * an album or era explicit or clean (once the music folder is set).
 */
import { ClipboardCheck, ShieldAlert, ShieldCheck, Tags } from "lucide-react";
import { useSettings } from "../stores/settings";
import { useTagEdit } from "../stores/tagEdit";
import type { MenuItem } from "../stores/ui";

/** The music folder is set, so files can be changed. */
export function canWriteTags(): boolean {
  const s = useSettings.getState().settings;
  return !!s?.musicFolder && !!s?.musicFolderServer;
}

/** Credit-only and other servers' items have no files Platter can reach the same way. */
function editable(id: string): boolean {
  return !id.startsWith("credit:");
}

export function tagAlbumItems(album: { id: string; title: string }): MenuItem[] {
  if (!editable(album.id)) return [];
  const mark = (explicit: boolean): MenuItem => ({
    label: explicit ? "Mark as explicit…" : "Mark as clean…",
    icon: explicit ? ShieldAlert : ShieldCheck,
    when: canWriteTags,
    onSelect: () => useTagEdit.getState().openExplicit({ albumIds: [album.id], explicit, title: album.title }),
  });
  return [
    { label: "Edit tags…", icon: Tags, separator: true, onSelect: () => useTagEdit.getState().openEditor({ albumId: album.id, trackIds: [] }) },
    { label: "Check tags…", icon: ClipboardCheck, onSelect: () => useTagEdit.getState().openCheck({ kind: "album", id: album.id }, album.title) },
    mark(true),
    mark(false),
  ];
}

export function tagTrackItem(track: { id: string }): MenuItem {
  return { label: "Edit tags…", icon: Tags, onSelect: () => useTagEdit.getState().openEditor({ albumId: null, trackIds: [track.id] }) };
}

export function tagArtistItem(artist: { id: string; title: string }): MenuItem[] {
  if (!editable(artist.id)) return [];
  return [{ label: "Check tags…", icon: ClipboardCheck, onSelect: () => useTagEdit.getState().openCheck({ kind: "artist", id: artist.id }, artist.title) }];
}

/** An era's albums, marked explicit or clean together. */
export function tagEraItems(artistId: string, era: string): MenuItem[] {
  const mark = (explicit: boolean): MenuItem => ({
    label: explicit ? "Mark era as explicit…" : "Mark era as clean…",
    icon: explicit ? ShieldAlert : ShieldCheck,
    when: canWriteTags,
    onSelect: () => useTagEdit.getState().openExplicit({ albumIds: [], era: { artistId, era }, explicit, title: era }),
  });
  return [mark(true), mark(false)];
}
