/**
 * What the banner at the top of the main view says for each app state. The
 * core decides the state and the wording (Health.banner); this maps it to a
 * tone and an action button, keeps "Last seen N min ago" current, and shows
 * library scan progress when nothing else needs saying. Plain offline has
 * no banner: a small icon in the top bar says it (`offlineMessage`).
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

/**
 * What the top bar's offline icon says (its tooltip), or null while the
 * server can be reached. Shown for every offline state, banner or not.
 */
export function offlineMessage(h: Health | null, now: number = Date.now()): string | null {
  if (!h || h.status !== "offline") return null;
  return h.banner ? messageOf(h, now) : "Can't reach your server right now.";
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
