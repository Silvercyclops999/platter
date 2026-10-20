/**
 * What the banner at the top of the main view says for each app state. The
 * core decides the state and the wording (Health.banner); this maps it to a
 * tone and an action button, keeps "Last seen N min ago" current, and shows
 * library scan progress when nothing else needs saying. Plain offline has
 * no banner: a small button in the top bar says it (`offlineCallout`).
 */
import type { BannerKind } from "../components/Banner";
import type { Health } from "../lib/types";

export type BannerAction = "signIn" | "signOut" | "libraryGuide" | "localNetwork";

export interface BannerView {
  kind: BannerKind;
  message: string;
  action?: BannerAction;
}

/** "just now", "4 min ago", "3 h ago", "2 days ago" for a duration in ms. */
export function agoText(ms: number): string {
  const mins = Math.floor(Math.max(0, ms) / 60_000);
  if (mins < 1) return "just now";
  if (mins < 60) return `${mins} min ago`;
  if (mins < 48 * 60) return `${Math.floor(mins / 60)} h ago`;
  return `${Math.floor(mins / (24 * 60))} days ago`;
}

const TONE: Record<string, BannerKind> = {
  offline: "offline",
  noInternet: "offline",
  starting: "progress",
  updating: "progress",
  startTimeout: "error",
  relay: "warning",
  signIn: "warning",
  accessRemoved: "error",
  filesUnavailable: "error",
  noMusicLibrary: "info",
  diskFull: "warning",
  localNetwork: "warning",
  goneOffline: "offline",
};

const ACTION: Record<string, BannerAction> = {
  signIn: "signIn",
  accessRemoved: "signOut",
  noMusicLibrary: "libraryGuide",
  localNetwork: "localNetwork",
};

/** States with nothing for the user to do: the top bar icon covers them (Go offline is undone from the app menu). */
const QUIET = new Set(["offline", "noInternet", "goneOffline"]);

/** The core's message, with "Last seen" kept current. */
function messageOf(h: Health, now: number): string {
  const b = h.banner!;
  // The core's text is as of when it was sent; keep the age current.
  if (b.kind === "offline" && h.lastSeenAt) return `Your server is offline. Last seen ${agoText(now - h.lastSeenAt)}.`;
  return b.message;
}

export interface OfflineCallout {
  title: string;
  body: string;
}

/**
 * What the callout under the top bar's offline button says, or null while
 * the server can be reached (the button goes away). Shown for every offline
 * state, banner or not. `mac` adds the menu bar route for Go offline.
 */
export function offlineCallout(h: Health | null, now: number = Date.now(), mac = false): OfflineCallout | null {
  if (!h || h.status !== "offline") return null;
  const kind = h.banner?.kind;
  if (kind === "goneOffline") {
    const where = mac ? "your profile menu, or File › Go Offline" : "your profile menu";
    return { title: "You chose Go offline.", body: `Only music on this computer plays. Turn it off in ${where}.` };
  }
  const title = "Platter is offline.";
  if (kind === "localNetwork") return { title, body: h.banner!.message };
  if (kind === "offline") {
    const seen = h.lastSeenAt ? ` (last seen ${agoText(now - h.lastSeenAt)})` : "";
    return { title, body: `Can't reach your server${seen}. Platter reconnects by itself.` };
  }
  return { title, body: "Check your connection; it reconnects by itself." };
}

/**
 * Why only music on this device can play, in one sentence, or null when
 * everything can. Shown wherever songs are dimmed for it (the phone's strip
 * over the tabs), so a dimmed list is never left unexplained.
 */
export function cachedOnlyReason(h: Health | null, device = "computer"): string | null {
  if (!h?.cachedOnly) return null;
  const only = `only songs on this ${device} play`;
  switch (h.banner?.kind) {
    case "goneOffline":
      return `You chose Go offline, so ${only}.`;
    case "noInternet":
      return `You're offline, so ${only}. Platter reconnects by itself.`;
    case "offline":
      return `Can't reach your server, so ${only}. Platter reconnects by itself.`;
    case "starting":
    case "updating":
    case "startTimeout":
      return `Your server is starting, so ${only} for now.`;
    case "filesUnavailable":
      return `Your server is up, but your music files aren't available, so ${only}.`;
    case "localNetwork":
      return h.banner.message;
    case "signIn":
    case "accessRemoved":
      return `${h.banner.message} Until then, ${only}.`;
    default:
      return `Can't reach your server right now, so ${only}.`;
  }
}

export function healthBanner(h: Health | null, now: number = Date.now()): BannerView | null {
  if (!h) return null;
  const b = h.banner;
  if (b && QUIET.has(b.kind)) return null;
  if (b) return { kind: TONE[b.kind] ?? "info", message: messageOf(h, now), action: ACTION[b.kind] };
  if (h.scan && h.scan.total > 0) {
    return {
      kind: "progress",
      message: `Adding new music… ${h.scan.done.toLocaleString()} of ${h.scan.total.toLocaleString()}`,
    };
  }
  return null;
}
