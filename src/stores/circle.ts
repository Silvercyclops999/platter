/**
 * Shared playlists and Blends as the UI sees them: the core's view (kept
 * fresh by `circle://changed`), its notices as toasts, and the actions with
 * the short toasts they end in. The dialogs (invite, permissions, leave)
 * open from here so every menu that offers them opens the same ones.
 */
import { create } from "zustand";
import { backend, type Backend } from "../lib/backend";
import type { AppError, CirclePerson, CircleView, JoinResult, Playlist, SharedSummary } from "../lib/types";
import { useLibraryView } from "./library";
import { player } from "./player";
import { nav, useRouter } from "./router";
import { copyText, toast } from "./toasts";

export const NO_CIRCLE: CircleView = { relay: "ok", playlists: [], blends: [], incoming: [], joining: 0 };

export type CircleDialog =
  | { kind: "invite"; playlistId: string; title: string }
  | { kind: "permissions"; playlistId: string }
  | { kind: "leave"; playlistId: string }
  | { kind: "removePerson"; playlistId: string; person: CirclePerson }
  | { kind: "leaveBlend"; id: string; title: string };

interface CircleState {
  view: CircleView;
  /** Bumped on every change: open pages read again. */
  version: number;
  dialog: CircleDialog | null;
}

export const useCircle = create<CircleState>()(() => ({ view: NO_CIRCLE, version: 0, dialog: null }));

export const errText = (e: unknown, fallback = "something went wrong") =>
  e && typeof e === "object" && "message" in e ? String((e as AppError).message) : fallback;

/** The page showing now. */
const here = () => {
  const r = useRouter.getState();
  return r.entries[r.index]?.route;
};

export const isShared = (id: string) => id.startsWith("shared:");
export const isBlend = (id: string) => id.startsWith("blend:");

function take(v: CircleView) {
  const before = useCircle.getState().view;
  useCircle.setState((s) => ({ view: v, version: s.version + 1 }));
  // What's in the library changed: the sidebar reads again.
  const key = (x: CircleView) => JSON.stringify([x.playlists.map((p) => [p.id, p.title, p.inLibrary, p.gone, p.role]), x.blends.map((b) => [b.id, b.title])]);
  if (key(before) !== key(v)) void useLibraryView.getState().loadItems();
}

let started: Promise<void> | null = null;
const unlistens: (() => void)[] = [];

/** Read the view and follow its events (once). */
export function startCircle(b: Backend = backend): Promise<void> {
  started ??= (async () => {
    unlistens.push(await b.listen("circle://changed", take));
    unlistens.push(await b.listen("circle://notice", (n) => toast.info(n.message)));
    try {
      take(await b.circleState());
    } catch {
      /* the view stays empty */
    }
  })();
  return started;
}

/** Tests: start again from nothing. */
export function resetCircle() {
  unlistens.splice(0).forEach((u) => u());
  started = null;
  useCircle.setState({ view: NO_CIRCLE, version: 0, dialog: null });
}

export function openCircleDialog(d: CircleDialog | null) {
  useCircle.setState({ dialog: d });
}

/** The sharing of a playlist, yours (Plex id) or someone else's (`shared:`). */
export function useShared(id: string | undefined): SharedSummary | null {
  return useCircle((s) => (id ? (s.view.playlists.find((p) => p.id === id) ?? null) : null));
}

export function sharedNow(id: string): SharedSummary | null {
  return useCircle.getState().view.playlists.find((p) => p.id === id) ?? null;
}

/** "Robin and Jules" or "Robin and 3 others" (the owner first). */
export function bylineText(s: SharedSummary): string {
  const [owner, ...rest] = s.people;
  const name = (p: CirclePerson) => (p.you ? "you" : p.name);
  if (!rest.length) return owner.you ? "You" : owner.name;
  const first = owner.you ? "You" : owner.name;
  return rest.length === 1 ? `${first} and ${name(rest[0])}` : `${first} and ${rest.length} others`;
}

/** Your role says you can add songs to it (yours, or a collaborator's). */
export function canAdd(s: SharedSummary | null): boolean {
  return !s || (!s.gone && (s.role === "owner" || s.role === "collaborator"));
}

// ---------------------------------------------------------------- actions

/** Play a playlist shared with you: the songs you have, from `start`. */
export async function playShared(id: string, start?: string, b: Backend = backend) {
  try {
    const p = await b.collabPage(id);
    const ids = p.rows.flatMap((r) => (r.track ? [r.track.id] : []));
    if (ids.length) await player.playTracks(ids, start);
    else toast.info("None of these songs are in your library.");
  } catch (e) {
    toast.error(`Couldn't play it: ${errText(e)}`);
  }
}

/** Play a Blend from `start`. */
export async function playBlend(id: string, start?: string, b: Backend = backend) {
  try {
    const p = await b.blendPage(id);
    if (p.rows.length) await player.playTracks(p.rows.map((r) => r.track.id), start);
  } catch (e) {
    toast.error(`Couldn't play the Blend: ${errText(e)}`);
  }
}

export async function copyInviteLink(playlistId: string, role: "collaborator" | "listener", b: Backend = backend): Promise<void> {
  try {
    const l = await b.collabInviteLink(playlistId, role);
    copyText(l.link);
  } catch (e) {
    toast.error(`Couldn't make an invite link: ${errText(e)}`);
  }
}

export async function invitePerson(playlistId: string, key: string, name: string, role: "collaborator" | "listener", b: Backend = backend): Promise<boolean> {
  try {
    await b.collabInvitePerson(playlistId, key, role);
    toast.info(`Invited ${name}`);
    return true;
  } catch (e) {
    toast.error(`Couldn't invite ${name}: ${errText(e)}`);
    return false;
  }
}

export async function setRole(playlistId: string, p: CirclePerson, role: "collaborator" | "listener", b: Backend = backend) {
  try {
    await b.collabSetRole(playlistId, p.id, role);
    toast.info(role === "collaborator" ? `${p.name} can add songs now` : `${p.name} can listen only now`);
  } catch (e) {
    toast.error(`Couldn't change ${p.name}'s role: ${errText(e)}`);
  }
}

export async function removePerson(playlistId: string, p: CirclePerson, b: Backend = backend) {
  try {
    await b.collabRemovePerson(playlistId, p.id);
    toast.info(`Removed ${p.name} from the playlist`);
  } catch (e) {
    toast.error(`Couldn't remove ${p.name}: ${errText(e)}`);
  }
}

export async function setPublic(playlistId: string, on: boolean, b: Backend = backend) {
  try {
    await b.collabSetPublic(playlistId, on);
    toast.info(on ? "The playlist is public" : "The playlist is private");
  } catch (e) {
    toast.error(`Couldn't change that: ${errText(e)}`);
  }
}

/** Leave a playlist shared with you; on a private one it goes from your library. */
export async function leavePlaylist(id: string, b: Backend = backend): Promise<void> {
  const s = sharedNow(id);
  try {
    await b.collabLeave(id);
    toast.info(`You left ${s?.title ?? "the playlist"}`);
    const r = here();
    if (s && !s.public && r?.name === "playlist" && r.id === id) nav.go({ name: "home" });
  } catch (e) {
    toast.error(`Couldn't leave: ${errText(e)}`);
  }
}

export async function setInLibrary(id: string, on: boolean, b: Backend = backend) {
  try {
    await b.collabSetInLibrary(id, on);
    toast.info(on ? "Added to Your Library" : "Removed from Your Library");
  } catch (e) {
    toast.error(`Couldn't change that: ${errText(e)}`);
  }
}

/** Add songs to a playlist shared with you (they go to its owner). */
export async function addToShared(p: Pick<Playlist, "id" | "title">, trackIds: string[], b: Backend = backend): Promise<void> {
  try {
    await b.collabAdd(p.id, trackIds);
    const s = sharedNow(p.id);
    const n = trackIds.length;
    toast.info(s?.ownerUnreachable ? `Added to ${p.title}. It goes to ${s.owner.name} when they're back online.` : n === 1 ? `Added to ${p.title}` : `Added ${n} songs to ${p.title}`);
  } catch (e) {
    toast.error(`Couldn't add to ${p.title}: ${errText(e)}`);
  }
}

export async function leaveBlend(id: string, title: string, b: Backend = backend) {
  try {
    await b.blendLeave(id);
    toast.info(`You left ${title}`);
    const r = here();
    if (r?.name === "playlist" && r.id === id) nav.go({ name: "home" });
  } catch (e) {
    toast.error(`Couldn't leave the Blend: ${errText(e)}`);
  }
}

/** Join from a link: opens it when in, says so when it waits for the owner. */
export async function joinCircle(link: string, b: Backend = backend): Promise<JoinResult | null> {
  try {
    const r = await b.circleJoin(link);
    if (r.state === "joined") nav.go({ name: "playlist", id: r.id });
    else toast.info(r.kind === "blend" ? "Asked to join. You're in once whoever invited you is online." : "Asked to join. You're in once the owner is online.");
    return r;
  } catch (e) {
    toast.error(errText(e, "Couldn't join"));
    return null;
  }
}

export async function acceptIncoming(id: string, b: Backend = backend) {
  try {
    const r = await b.circleAccept(id);
    if (r.state === "joined") nav.go({ name: "playlist", id: r.id });
  } catch (e) {
    toast.error(errText(e, "Couldn't join"));
  }
}
