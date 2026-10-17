/**
 * Sharing your music through Plex, as the UI sees it: what can be shared,
 * the people with access, the invites you received, and the share dialog
 * (Settings, Sharing, or "Let them listen from my server" in a session).
 */
import { create } from "zustand";
import { backend, type Backend } from "../lib/backend";
import type { AppError, PersonWithAccess, ReceivedInvite, ShareLinkAsk, ShareLinkView, ShareOptions, ShareStatus, SharingOverview } from "../lib/types";
import { useSettings } from "./settings";
import { useSocial } from "./social";
import { copyText, toast } from "./toasts";

/** Who the share dialog is for: someone in your session, or anyone. */
export type ShareTarget = { kind: "guest"; personId: string; name: string } | { kind: "anyone" };

interface SharingState {
  overview: SharingOverview | null;
  people: PersonWithAccess[] | null;
  /** Invites you received (server shares only). */
  invites: ReceivedInvite[];
  /** Invites being accepted. */
  accepting: ReadonlySet<string>;
  /** The share dialog, and who it's for. */
  dialog: ShareTarget | null;
  link: ShareLinkView | null;
  /** People who opened your share links and wait for a yes. */
  asks: ShareLinkAsk[];
}

export const useSharing = create<SharingState>()(() => ({
  overview: null,
  people: null,
  invites: [],
  accepting: new Set(),
  dialog: null,
  link: null,
  asks: [],
}));

const errText = (e: unknown) => (e && typeof e === "object" && "message" in e ? String((e as AppError).message) : "");

let started: Backend | null = null;
let unsubs: Array<() => void> = [];

function takeInvites(all: ReceivedInvite[]) {
  const before = new Set(useSharing.getState().invites.map((i) => i.id));
  useSharing.setState({ invites: all });
  const accept = useSettings.getState().settings?.shareAcceptInvites ?? true;
  for (const i of all) if (!before.has(i.id) && accept) toast.info(`${i.from} shared their music with you`);
}

/** Follow invites and share-link notices (once, at startup). */
export async function startSharing(b: Backend = backend): Promise<void> {
  if (started === b) return;
  started = b;
  unsubs.push(await b.listen("sharing://invites", takeInvites));
  unsubs.push(await b.listen("sharing://notice", (n) => toast.info(n.message)));
  unsubs.push(await b.listen("sharing://link-asks", (asks) => useSharing.setState({ asks })));
  try {
    useSharing.setState({ invites: await b.sharingInvites(false) });
  } catch {
    /* signed out */
  }
}

/** For tests. */
export function resetSharing() {
  unsubs.forEach((u) => u());
  unsubs = [];
  started = null;
  useSharing.setState({ overview: null, people: null, invites: [], accepting: new Set(), dialog: null, link: null });
}

export async function loadOverview(b: Backend = backend): Promise<SharingOverview | null> {
  try {
    const o = await b.sharingOverview();
    useSharing.setState({ overview: o });
    return o;
  } catch (e) {
    toast.error(errText(e) || "Couldn't read your libraries");
    return null;
  }
}

export async function loadPeople(b: Backend = backend): Promise<void> {
  try {
    useSharing.setState({ people: await b.sharingPeople() });
  } catch {
    useSharing.setState({ people: [] });
  }
}

export async function loadInvites(b: Backend = backend): Promise<void> {
  try {
    takeInvites(await b.sharingInvites(true));
  } catch {
    /* Plex not answering: keep what's shown */
  }
}

export async function removeAccess(p: PersonWithAccess, b: Backend = backend): Promise<void> {
  try {
    await b.sharingRemove(p.id);
    useSharing.setState({ people: (useSharing.getState().people ?? []).filter((x) => x.id !== p.id) });
    toast.info(`${p.name} can't reach your server anymore`);
  } catch (e) {
    toast.error(errText(e) || "Couldn't remove their access");
  }
}

/** The options a share starts with: the defaults from Settings. */
export function defaultOptions(): ShareOptions {
  const s = useSettings.getState().settings;
  return { libraries: s?.shareDefaultLibraries ?? [], hideUnreleased: s?.shareHideUnreleased ?? true, allowDownloads: s?.shareAllowDownloads ?? false };
}

/** Unreleased albums in the chosen libraries (null: one isn't read yet). */
export function unreleasedIn(o: SharingOverview | null, ids: string[]): number | null {
  if (!o) return null;
  let n = 0;
  for (const l of o.libraries.filter((x) => ids.includes(x.id))) {
    if (l.unreleased === null) return null;
    n += l.unreleased;
  }
  return n;
}

/** Words for a share's outcome. */
export function statusText(st: ShareStatus, who: string): string {
  switch (st.status) {
    case "invited":
      return st.hidden ? `Plex sent ${who} an invite. ${st.hidden === 1 ? "1 unreleased album stays" : `${st.hidden} unreleased albums stay`} hidden.` : `Plex sent ${who} an invite.`;
    case "alreadyShared":
      return `${who} already has access to your server.`;
    case "needsPlexPass":
      return "Keeping unreleased music out needs Plex Pass.";
    default:
      return st.message;
  }
}

/** Share with a Plex username or email. Resolves to how it went (null: an error was shown). */
export async function shareWith(invitee: string, options: ShareOptions, b: Backend = backend): Promise<ShareStatus | null> {
  try {
    const st = await b.sharingShare(invitee, options);
    if (st.status === "invited" || st.status === "alreadyShared") {
      toast.info(statusText(st, invitee.trim()));
      void loadPeople(b);
    }
    return st;
  } catch (e) {
    toast.error(errText(e) || "Couldn't share");
    return null;
  }
}

/** "Let them listen from my server" for someone in your session. */
export async function offerServer(personId: string, b: Backend = backend): Promise<void> {
  try {
    useSocial.setState({ listen: await b.listenOfferServer(personId) });
  } catch (e) {
    toast.error(errText(e) || "Couldn't share your server");
  }
}

/**
 * The button in a session: one click when default libraries are chosen and
 * nothing stands in the way; else the share dialog opens to pick.
 */
export async function letThemListen(personId: string, name: string, b: Backend = backend): Promise<void> {
  const opts = defaultOptions();
  if (opts.libraries.length) {
    const o = useSharing.getState().overview ?? (await loadOverview(b));
    const hidden = unreleasedIn(o, opts.libraries);
    const blocked = opts.hideUnreleased && (hidden === null || (hidden > 0 && o?.plexPass === false));
    if (o?.owner && !blocked) return offerServer(personId, b);
  }
  openShareDialog({ kind: "guest", personId, name }, b);
}

export function openShareDialog(target: ShareTarget, b: Backend = backend) {
  useSharing.setState({ dialog: target, link: null });
  void loadOverview(b);
}

export function closeShareDialog() {
  useSharing.setState({ dialog: null, link: null });
}

export async function makeShareLink(options: ShareOptions, b: Backend = backend): Promise<void> {
  try {
    useSharing.setState({ link: await b.sharingLink(options) });
  } catch (e) {
    toast.error(errText(e) || "Couldn't make a link");
  }
}

export function copyShareLink() {
  const l = useSharing.getState().link?.link;
  if (l) copyText(l);
}

/** Open someone's share link (from a deep link or pasted). */
export async function openShareLink(link: string, b: Backend = backend): Promise<void> {
  try {
    const r = await b.sharingOpenLink(link);
    if (r.status === "failed") toast.error(r.message);
    else if (r.status === "asked") toast.info(`Asked ${r.host} for their music. Their invite shows here once they say yes.`);
    else toast.info(r.status === "invited" ? `Asked ${r.host} for their music. Their invite shows here in a moment.` : `You already have access to ${r.host}'s server.`);
  } catch (e) {
    toast.error(errText(e) || "Couldn't open that link");
  }
}

/** The share link requests waiting for a yes. */
export async function loadLinkAsks(b: Backend = backend): Promise<void> {
  try {
    useSharing.setState({ asks: await b.sharingLinkAsks() });
  } catch {
    /* not the owner, or signed out */
  }
}

/** Say yes to someone who opened your share link. */
export async function approveLinkAsk(a: ShareLinkAsk, b: Backend = backend): Promise<void> {
  try {
    toast.info(await b.sharingLinkApprove(a.id));
  } catch (e) {
    toast.error(errText(e) || "Couldn't share");
  }
  await loadLinkAsks(b);
}

export async function denyLinkAsk(a: ShareLinkAsk, b: Backend = backend): Promise<void> {
  try {
    await b.sharingLinkDeny(a.id);
  } catch {
    /* already gone */
  }
  await loadLinkAsks(b);
}

/** Accept an invite: their server joins your library and the connection race. */
export async function acceptInvite(inv: ReceivedInvite, b: Backend = backend): Promise<void> {
  const busy = new Set(useSharing.getState().accepting);
  busy.add(inv.id);
  useSharing.setState({ accepting: busy });
  try {
    const a = await b.sharingAccept(inv.id);
    useSharing.setState({ invites: useSharing.getState().invites.filter((i) => i.id !== inv.id) });
    toast.info(a.libraries ? `Added ${a.servers[0] ?? `${inv.from}'s server`}. Their music joins your library as it's read.` : `Accepted. ${inv.from}'s server shows up once Plex lists it.`);
  } catch (e) {
    toast.error(errText(e) || "Couldn't accept the invite");
  } finally {
    const left = new Set(useSharing.getState().accepting);
    left.delete(inv.id);
    useSharing.setState({ accepting: left });
  }
}

/** Hide an invite here for now (it stays on Plex). */
export function dismissInvite(id: string) {
  useSharing.setState({ invites: useSharing.getState().invites.filter((i) => i.id !== id) });
}

/** "2 hours ago", "3 days ago", or "Not yet" for someone's last visit. */
export function lastActive(at: number | null, now = Date.now() / 1000): string {
  if (!at) return "Not yet";
  const m = Math.max(0, Math.round((now - at) / 60));
  if (m < 2) return "Just now";
  if (m < 60) return `${m} minutes ago`;
  const h = Math.round(m / 60);
  if (h < 24) return h === 1 ? "1 hour ago" : `${h} hours ago`;
  const d = Math.round(h / 24);
  return d === 1 ? "Yesterday" : `${d} days ago`;
}

