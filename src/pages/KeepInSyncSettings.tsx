/**
 * Settings parts for Keep my devices in sync: the linked devices and how
 * each is doing, and changes held back from a clock far ahead. Every
 * Platter signed in to the same Plex account finds the others by itself,
 * so there's nothing to pair here.
 */
import { useCallback, useEffect, useRef, useState } from "react";
import { Button } from "../components/Button";
import { Dialog } from "../components/Dialog";
import { backend } from "../lib/backend";
import { failText } from "../lib/errorText";
import { timeAgo } from "../lib/relativeTime";
import type { DeviceSyncDevice, DeviceSyncOverview, Settings } from "../lib/types";
import { toast } from "../stores/toasts";
import "./sync-settings.css";


/** Paired devices and pairing, kept current while Settings is open.
 * `failed`: the status couldn't be read (with `reload` to try again). */
export function useDeviceSync() {
  const [ov, setOv] = useState<DeviceSyncOverview | null>(null);
  const [failed, setFailed] = useState(false);
  const alive = useRef(true);
  // Events that come before the first status is read are kept, not lost.
  const early = useRef<Partial<DeviceSyncOverview>>({});
  const seq = useRef(0);
  const reload = useCallback(() => {
    const n = ++seq.current;
    return backend.deviceSyncStatus().then(
      (o) => {
        if (!alive.current || n !== seq.current) return;
        setFailed(false);
        setOv({ ...o, ...early.current });
        early.current = {};
      },
      (e) => {
        if (!alive.current || n !== seq.current) return;
        console.warn("device sync status", e);
        setFailed(true);
      },
    );
  }, []);
  useEffect(() => {
    alive.current = true;
    const offStatus = backend.listen("device-sync://status", (devices) => {
      if (!alive.current) return;
      setOv((o) => {
        if (!o) early.current.devices = devices;
        return o ? { ...o, devices } : o;
      });
    });
    const offPair = backend.listen("device-sync://pair", (pairing) => {
      if (!alive.current) return;
      setOv((o) => {
        if (!o) early.current.pairing = pairing;
        return o ? { ...o, pairing } : o;
      });
      if (pairing.phase === "paired") void reload();
    });
    void reload();
    return () => {
      alive.current = false;
      void offStatus.then((u) => u());
      void offPair.then((u) => u());
    };
  }, [reload]);
  return [ov, setOv, { failed, reload }] as const;
}

/** One line under the switch. */
export function deviceSyncNote(s: Settings | null, ov: DeviceSyncOverview | null): string | undefined {
  if (!s || !ov) return undefined;
  if (!s.deviceSync) return ov.devices.length ? "Off. Your paired devices stay paired." : undefined;
  if (ov.devices.length === 0) return "Your other devices signed in to this Plex account show up here by themselves.";
  const behind = ov.devices.find((d) => d.words !== "In sync");
  if (behind) return behind.words + ".";
  return `In sync with ${ov.devices.map((d) => d.name).join(", ")}.`;
}

/** What a paired device's row says under its name. */
export function deviceLine(d: DeviceSyncDevice): string {
  const when = d.lastSyncedAt ? `, last ${timeAgo(d.lastSyncedAt / 1000).toLowerCase()}` : "";
  return d.words === "In sync" ? `In sync${when}` : d.words;
}

/** Under a device: a newer Platter there, clocks far apart, held changes. */
export function deviceNote(d: DeviceSyncDevice): string | undefined {
  const parts = [d.updateOther, d.clock, d.held ? `${d.held === 1 ? "1 change" : `${d.held} changes`} from it held back` : null].filter(Boolean);
  return parts.length ? parts.join(". ") + "." : undefined;
}

export function ForgetDeviceButton({ device, onOv }: { device: DeviceSyncDevice; onOv: (o: DeviceSyncOverview) => void }) {
  const [asking, setAsking] = useState(false);
  const [busy, setBusy] = useState(false);
  const forget = () => {
    setBusy(true);
    backend.deviceSyncForget(device.id).then(
      (o) => {
        setBusy(false);
        setAsking(false);
        onOv(o);
        toast.info(`${device.name} is no longer paired`);
      },
      (e) => {
        setBusy(false);
        toast.error(failText(`Couldn't unpair ${device.name}`, e));
      },
    );
  };
  return (
    <>
      <Button size="sm" variant="ghost" onClick={() => setAsking(true)}>
        Unpair
      </Button>
      <Dialog open={asking} onClose={() => setAsking(false)} label={`Unpair ${device.name}`} className="confirm keys-dialog">
        <h2 className="confirm__title">Unpair {device.name}?</h2>
        <p className="confirm__body">
          The two stop keeping in sync. Everything already on each stays there. {device.name} is told, and forgets this device too.
        </p>
        <div className="confirm__actions">
          <Button variant="ghost" onClick={() => setAsking(false)}>
            Cancel
          </Button>
          <Button variant="primary" disabled={busy} onClick={forget}>
            {busy ? "Unpairing…" : "Unpair"}
          </Button>
        </div>
      </Dialog>
    </>
  );
}

export function DeviceSyncNowButton({ ov, on, onOv }: { ov: DeviceSyncOverview | null; on: boolean; onOv: (o: DeviceSyncOverview) => void }) {
  const [busy, setBusy] = useState(false);
  const run = () => {
    setBusy(true);
    backend.deviceSyncNow().then(
      (o) => {
        setBusy(false);
        onOv(o);
        // Say how it went: the first device not in step, or all in step.
        const behind = o.devices.find((d) => d.words !== "In sync");
        if (behind) toast.info(`${behind.words}.`);
        else toast.info(o.devices.length === 1 ? `In sync with ${o.devices[0].name}` : `In sync with all ${o.devices.length} devices`);
      },
      (e) => {
        setBusy(false);
        toast.error(failText("Couldn't sync", e));
      },
    );
  };
  return (
    <Button size="sm" disabled={busy || !on || !ov?.devices.length} onClick={run}>
      {busy ? "Syncing…" : "Sync now"}
    </Button>
  );
}

export function TakeHeldButton({ onOv }: { onOv: (o: DeviceSyncOverview) => void }) {
  const [busy, setBusy] = useState(false);
  return (
    <Button
      size="sm"
      disabled={busy}
      onClick={() => {
        setBusy(true);
        backend.deviceSyncTakeQuarantined().then(
          (o) => {
            setBusy(false);
            onOv(o);
            toast.info("Held changes taken");
          },
          (e) => {
            setBusy(false);
            toast.error(failText("Couldn't take the held changes", e));
          },
        );
      }}
    >
      Take them anyway
    </Button>
  );
}
