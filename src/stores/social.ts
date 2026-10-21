/**
 * Friend activity, the follow list, the What's New badge and the listening
 * session, as the UI sees them. Follows and the session follow their core
 * events; friend activity is read when its panel opens and every minute
 * while it's open.
 */
import { create } from "zustand";
import { backend, type Backend } from "../lib/backend";
import type { ActivityPanel, AppError, FollowedArtist, ListenOp, ListenView } from "../lib/types";
import { useSettings } from "./settings";
import { copyText, toast } from "./toasts";
import type { MenuItem } from "./ui";

export const NO_SESSION: ListenView = {
  role: "none",
  sessionId: null,
  hostName: null,
  invite: null,
  people: [],
  listeners: [],
  guestsCanControl: false,
  nowBy: null,
  queue: [],
  status: null,
  relay: "ok",
  cantHear: [],
  missing: null,
  anotherVersion: false,
};

interface SocialState {
  follows: FollowedArtist[];
  followIds: ReadonlySet<string>;
  /** Unseen What's New items (0 while announcements are off). */
  unseen: number;
  activity: ActivityPanel | null;
  listen: ListenView;
  /** The invite dialog is open. */
  inviteOpen: boolean;
}

export const useSocial = create<SocialState>()(() => ({
  follows: [],
  followIds: new Set(),
  unseen: 0,
  activity: null,
  listen: NO_SESSION,
  inviteOpen: false,
}));

const errText = (e: unknown) => (e && typeof e === "object" && "message" in e ? String((e as AppError).message) : "");

function takeFollows(all: FollowedArtist[]) {
  useSocial.setState({ follows: all, followIds: new Set(all.map((f) => f.id)) });
}

/** A new session view; a guest whose session stopped is told why. */
function takeListen(v: ListenView) {
  const before = useSocial.getState().listen;
  useSocial.setState({ listen: v });
  if (v.role === "none") useSocial.setState({ inviteOpen: false });
  if (v.role !== "guest" || before.status === v.status) return;
  const host = v.hostName ?? "The host";
  if (v.status === "hostGone") toast.error(`${host} stopped answering. You're no longer listening along`);
  if (v.status === "live" && before.status === "hostGone") toast.info(`${host} is back`);
  if (v.status === "ended") toast.info("The listening session ended");
  if (v.status === "removed") toast.info(`${host} removed you from the listening session`);
}

let started: Backend | null = null;
let unsubs: Array<() => void> = [];

/** Load and follow everything (once, at startup). */
export async function startSocial(b: Backend = backend): Promise<void> {
  if (started === b) return;
  started = b;
  unsubs.push(await b.listen("follows://changed", takeFollows));
  unsubs.push(await b.listen("whatsnew://changed", (p) => useSocial.setState({ unseen: p.unseen })));
  unsubs.push(await b.listen("listen://state", takeListen));
  unsubs.push(await b.listen("listen://notice", (n) => toast.info(n.message)));
  await Promise.all([
    b.followsList().then(takeFollows, () => {}),
    b.whatsNew().then((w) => useSocial.setState({ unseen: w.unseen }), () => {}),
    b.listenState().then((v) => useSocial.setState({ listen: v }), () => {}),
  ]);
}

/** For tests: forget the subscriptions and state. */
export function resetSocial() {
  unsubs.forEach((u) => u());
  unsubs = [];
  started = null;
  useSocial.setState({ follows: [], followIds: new Set(), unseen: 0, activity: null, listen: NO_SESSION, inviteOpen: false });
}

export const isFollowing = (id: string) => useSocial.getState().followIds.has(id);

/** Follow or unfollow; the pill changes at once and goes back if the core refuses. */
export async function setFollow(artist: { id: string; title: string; thumb?: string | null }, on: boolean, b: Backend = backend): Promise<void> {
  const before = useSocial.getState().follows;
  const now = Math.floor(Date.now() / 1000);
  takeFollows(on ? [...before.filter((f) => f.id !== artist.id), { id: artist.id, title: artist.title, thumb: artist.thumb ?? null, followedAt: now }] : before.filter((f) => f.id !== artist.id));
  try {
    takeFollows(await b.followSet(artist, on));
  } catch (e) {
    takeFollows(before);
    toast.error(errText(e) || (on ? "Couldn't follow" : "Couldn't unfollow"));
    return;
  }
  if (on) void sayFollowGets(b);
}

const FOLLOW_GETS_SAID = "platter.followGetsSaid";

/** The first follow that gets new albums says so, once. */
async function sayFollowGets(b: Backend) {
  try {
    if (localStorage.getItem(FOLLOW_GETS_SAID)) return;
  } catch {
    return;
  }
  if (useSettings.getState().settings?.followGet === false) return;
  const ready = await b.lidarrStatus().then(
    (s) => s.ready,
    () => false,
  );
  if (!ready) return;
  toast.info("Following. Their new albums will come in by themselves.");
  try {
    localStorage.setItem(FOLLOW_GETS_SAID, "1");
  } catch {
    // Said again next time; harmless.
  }
}

/** Get a followed artist's new albums, or just show them. */
export async function setJustShow(id: string, justShow: boolean, b: Backend = backend): Promise<void> {
  const before = useSocial.getState().follows;
  takeFollows(before.map((f) => (f.id === id ? { ...f, justShow: justShow || undefined } : f)));
  try {
    takeFollows(await b.followJustShow(id, justShow));
  } catch (e) {
    takeFollows(before);
    toast.error(errText(e) || "Couldn't change that");
  }
}

/** Whether a followed artist's new albums only show (false when not followed). */
export const useJustShow = (id: string) => useSocial((s) => s.follows.some((f) => f.id === id && !!f.justShow));

/** Read friend activity (the panel calls this on open and every minute). */
export async function loadActivity(fresh = false, b: Backend = backend): Promise<void> {
  try {
    useSocial.setState({ activity: await b.friendActivity(fresh) });
  } catch {
    useSocial.setState({ activity: { state: "offline", friends: [] } });
  }
}

// ---------------------------------------------------------------- listen along

/** Host a session and show its invite. */
export async function startSession(b: Backend = backend): Promise<void> {
  try {
    const v = await b.listenStart();
    useSocial.setState({ listen: v, inviteOpen: true });
    toast.info("Listening session started");
  } catch (e) {
    toast.error(errText(e) || "Couldn't start a listening session");
  }
}

/** Open the invite dialog (host): the open invite, or a new one when it was used. `fresh` always makes a new link. */
export async function openInvite(fresh = false, b: Backend = backend): Promise<void> {
  try {
    const v = await b.listenInvite(fresh);
    useSocial.setState({ listen: v, inviteOpen: true });
  } catch (e) {
    toast.error(errText(e) || "Couldn't make an invite");
  }
}

export function closeInvite() {
  useSocial.setState({ inviteOpen: false });
}

/** Remove someone from the session you host. */
export async function removePerson(id: string, b: Backend = backend): Promise<void> {
  const name = useSocial.getState().listen.people.find((p) => p.id === id)?.name ?? "They";
  try {
    useSocial.setState({ listen: await b.listenRemove(id) });
    toast.info(`${name} was removed from the session`);
  } catch (e) {
    toast.error(errText(e) || "Couldn't remove them");
  }
}

/** A guest in a live session whose controls go to the host. */
export function guestLive(): boolean {
  const v = useSocial.getState().listen;
  return v.role === "guest" && v.status === "live";
}

/** Play, pause, skip, seek or reorder for everyone in the session. */
export async function controlSession(op: ListenOp, b: Backend = backend): Promise<void> {
  try {
    await b.listenControl(op);
  } catch (e) {
    toast.error(errText(e) || "Couldn't reach the host");
  }
}

export async function endSession(b: Backend = backend): Promise<void> {
  const role = useSocial.getState().listen.role;
  try {
    await b.listenStop();
    useSocial.setState({ listen: NO_SESSION });
    toast.info(role === "host" ? "Listening session ended" : "You left the listening session");
  } catch (e) {
    toast.error(errText(e) || "Couldn't end the session");
  }
}

export async function joinSession(link: string, b: Backend = backend): Promise<void> {
  try {
    const v = await b.listenJoin(link);
    useSocial.setState({ listen: v });
    toast.info(`Listening along with ${v.hostName ?? "the host"}`);
  } catch (e) {
    toast.error(errText(e) || "Couldn't join the listening session");
  }
}

/** Songs into the session's queue. */
export async function addToSession(ids: string[], b: Backend = backend): Promise<void> {
  try {
    await b.listenAdd(ids);
    toast.info(ids.length === 1 ? "Added to the session's queue" : `${ids.length} songs added to the session's queue`);
  } catch (e) {
    toast.error(errText(e) || "Couldn't add to the session");
  }
}

export function copyInvite() {
  const link = useSocial.getState().listen.invite?.link;
  if (link) copyText(link);
}

/**
 * The listening session items for a song, album or playlist menu: "Start a
 * listening session" (plays it, then hosts) while there's no session, and
 * "Add to the session's queue" for a guest (always allowed).
 */
export function sessionItems(trackIds: () => string[], play: () => void): MenuItem[] {
  return [
    startSessionItem(play),
    {
      label: "Add to the session's queue",
      when: guestLive,
      onSelect: () => void addToSession(trackIds()),
    },
  ];
}

/**
 * Listening sessions on or off for the whole app. They go through a relay,
 * so participants never connect to each other directly.
 */
export const listenAlong = { enabled: true };

/** "Start a listening session": plays the item, then hosts. Only while there's no session. */
export function startSessionItem(play: () => void): MenuItem {
  return {
    label: "Start a listening session",
    when: () => listenAlong.enabled && useSocial.getState().listen.role === "none",
    onSelect: () => {
      play();
      void startSession();
    },
  };
}
