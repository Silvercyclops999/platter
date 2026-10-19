/**
 * Tells device sync when the network changes (back online, another
 * Wi-Fi, mobile data), so paired devices catch up at once instead of at
 * the next round, and big things wait while on mobile data. Coming back to
 * the foreground is the core's own to see.
 */
import { backend } from "./backend";

/** What the web view says about the connection, where it says anything. */
interface Connection {
  type?: string;
  addEventListener?: (type: "change", f: () => void) => void;
  removeEventListener?: (type: "change", f: () => void) => void;
}

function connection(): Connection | undefined {
  return typeof navigator === "undefined" ? undefined : (navigator as Navigator & { connection?: Connection }).connection;
}

/** On mobile data (true), Wi-Fi or a cable (false), or can't tell (null). */
export function meteredNow(c: Connection | undefined = connection()): boolean | null {
  switch (c?.type) {
    case "cellular":
      return true;
    case "wifi":
    case "ethernet":
      return false;
    default:
      return null;
  }
}

/** Follow the network until the returned function is called. */
export function followNetwork(): () => void {
  if (typeof window === "undefined") return () => {};
  const c = connection();
  const tell = () => {
    backend.deviceSyncNetwork(meteredNow(c)).catch((e: unknown) => console.debug("device sync network", e));
  };
  window.addEventListener("online", tell);
  c?.addEventListener?.("change", tell);
  // Already on mobile data when the app opens: big things wait from the start.
  if (meteredNow(c) === true) tell();
  return () => {
    window.removeEventListener("online", tell);
    c?.removeEventListener?.("change", tell);
  };
}
