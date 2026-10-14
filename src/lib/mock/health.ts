/**
 * Mock app states for demos and screenshots: `?health=<name>` in the
 * browser. Wording matches what the Rust core sends (src-tauri/src/plex/health.rs).
 */
import type { Health, HealthStatus } from "../types";

export type HealthDemo =
  | HealthStatus
  | "noInternet"
  | "startTimeout"
  | "signIn"
  | "accessRemoved"
  | "filesUnavailable"
  | "noMusicLibrary"
  | "diskFull"
  | "scanning";

export const HEALTH_DEMOS: HealthDemo[] = [
  "local",
  "remote",
  "relay",
  "offline",
  "noInternet",
  "starting",
  "updating",
  "startTimeout",
  "signIn",
  "accessRemoved",
  "filesUnavailable",
  "noMusicLibrary",
  "diskFull",
  "scanning",
];

const STATUS: Record<HealthDemo, HealthStatus> = {
  local: "local",
  remote: "remote",
  relay: "relay",
  offline: "offline",
  noInternet: "offline",
  starting: "starting",
  updating: "updating",
  startTimeout: "starting",
  signIn: "noAuth",
  noAuth: "noAuth",
  accessRemoved: "noAuth",
  filesUnavailable: "local",
  noMusicLibrary: "local",
  diskFull: "local",
  scanning: "local",
};

const BANNER: Partial<Record<HealthDemo, { kind: string; message: string }>> = {
  noInternet: { kind: "noInternet", message: "You're offline." },
  starting: { kind: "starting", message: "Server is starting…" },
  updating: { kind: "updating", message: "Server is updating…" },
  startTimeout: { kind: "startTimeout", message: "Your server is taking too long to come back. Check that Plex is running." },
  relay: {
    kind: "relay",
    message: "Connected through Plex Relay, so quality is reduced. Turn on Remote Access in Plex for full quality.",
  },
  signIn: { kind: "signIn", message: "Please sign in again." },
  noAuth: { kind: "signIn", message: "Please sign in again." },
  accessRemoved: { kind: "accessRemoved", message: "You no longer have access to this server." },
  filesUnavailable: { kind: "filesUnavailable", message: "Your server is up, but your music files aren't available." },
  noMusicLibrary: { kind: "noMusicLibrary", message: "No music library found." },
  diskFull: { kind: "diskFull", message: "Not enough space." },
};

export function isHealthDemo(v: string | null): v is HealthDemo {
  return !!v && (HEALTH_DEMOS as string[]).includes(v);
}

export function demoHealth(demo: HealthDemo, server: { name: string; version: string }, now: number = Date.now()): Health {
  const status = STATUS[demo];
  const connected = status === "local" || status === "remote" || status === "relay";
  const offlineFor = 4 * 60_000;
  const h: Health = {
    status,
    serverName: server.name,
    connection: connected ? status : null,
    latencyMs: status === "local" ? 4 : status === "remote" ? 38 : status === "relay" ? 160 : null,
    serverVersion: server.version,
    lastSeenAt: status === "offline" ? now - offlineFor : now,
    scan: demo === "scanning" ? { done: 120, total: 340 } : null,
    banner: BANNER[demo] ?? null,
    filesAvailable: demo !== "filesUnavailable",
    cachedOnly: !connected || demo === "filesUnavailable",
  };
  if (demo === "offline") h.banner = { kind: "offline", message: "Your server is offline. Last seen 4 min ago." };
  return h;
}
