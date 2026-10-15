/**
 * Friend activity, the follow list, the What's New badge and the listening
 * session, as the UI sees them. Follows and the session follow their core
 * events; friend activity is read when its panel opens and every minute
 * while it's open.
 */
import { create } from "zustand";
import { backend, type Backend } from "../lib/backend";
import type { ActivityPanel, AppError, FollowedArtist, ListenView } from "../lib/types";
import { copyText, toast } from "./toasts";
import type { MenuItem } from "./ui";

const NO_SESSION: ListenView = { role: "none", sessionId: null, hostName: null, invite: null, listeners: [], guestsCanAdd: false, status: null };

interface SocialState {
  follows: FollowedArtist[];
  followIds: ReadonlySet<string>;
  /** Unseen What's New items (0 while announcements are off). */
  unseen: number;
  activity: ActivityPanel | null;
  listen: ListenView;
}

export const useSocial = create<SocialState>()(() => ({
  follows: [],
  followIds: new Set(),
  unseen: 0,
  activity: null,
  listen: NO_SESSION,
}));

const errText = (e: unknown) => (e && typeof e === "object" && "message" in e ? String((e as AppError).message) : "");

function takeFollows(all: FollowedArtist[]) {
  useSocial.setState({ follows: all, followIds: new Set(all.map((f) => f.id)) });
}

/** A new session view; a guest whose session stopped is told why. */
function takeListen(v: ListenView) {
  const before = useSocial.getState().listen;
  useSocial.setState({ listen: v });
  if (v.role !== "guest" || before.status === v.status) return;
  if (v.status === "hostGone") toast.error(`${v.hostName ?? "The host"} stopped answering. You're no longer listening along.`);
  if (v.status === "ended") toast.info("The listening session ended.");
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
  useSocial.setState({ follows: [], followIds: new Set(), unseen: 0, activity: null, listen: NO_SESSION });
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
  }
}

/** Read friend activity (the panel calls this on open and every minute). */
export async function loadActivity(fresh = false, b: Backend = backend): Promise<void> {
  try {
    useSocial.setState({ activity: await b.friendActivity(fresh) });
  } catch {
    useSocial.setState({ activity: { state: "offline", friends: [] } });
  }
}

// ---------------------------------------------------------------- listen along

/** Host a session and copy its invite link. */
export async function startSession(b: Backend = backend): Promise<void> {
  try {
    const v = await b.listenStart();
    useSocial.setState({ listen: v });
    if (v.invite) {
      const copied = await (navigator.clipboard?.writeText(v.invite).then(() => true, () => false) ?? Promise.resolve(false));
      toast.info(copied ? "Listening session started. The invite link is copied." : "Listening session started. Copy the invite link from the bar below.");
    }
  } catch (e) {
    toast.error(errText(e) || "Couldn't start a listening session");
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
  const link = useSocial.getState().listen.invite;
  if (link) copyText(link);
}

/**
 * The listening session items for a song, album or playlist menu: "Start a
 * listening session" (plays it, then hosts) while there's no session, and
 * "Add to the session's queue" for a guest whose host allows it.
 */
export function sessionItems(trackIds: () => string[], play: () => void): MenuItem[] {
  const listen = () => useSocial.getState().listen;
  return [
    startSessionItem(play),
    {
      label: "Add to the session's queue",
      when: () => listen().role === "guest" && listen().status === "live" && listen().guestsCanAdd,
      onSelect: () => void addToSession(trackIds()),
    },
  ];
}

/**
 * Listening sessions are switched off while their transport is rebuilt to go
 * through a relay, so participants never connect to each other directly.
 * Tests switch it on to keep covering the flow.
 */
export const listenAlong = { enabled: false };

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
