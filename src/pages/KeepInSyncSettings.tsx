/**
 * Settings parts for Keep my devices in sync: the paired devices and how
 * each is doing, adding a device (the codes to show), pairing with a code
 * another device shows, and changes held back from a clock far ahead.
 */
import { useCallback, useEffect, useRef, useState } from "react";
import { Button } from "../components/Button";
import { Dialog } from "../components/Dialog";
import { QrCode } from "../components/QrCode";
import { backend } from "../lib/backend";
import { failText } from "../lib/errorText";
import { timeAgo } from "../lib/relativeTime";
import type { AppError, DevicePairState, DeviceSyncDevice, DeviceSyncOverview, Settings } from "../lib/types";
import { toast } from "../stores/toasts";
import "./sync-settings.css";

const errText = (e: unknown) => (e && typeof e === "object" && "message" in e ? String((e as AppError).message) : "");
const spaced = (code: string) => `${code.slice(0, 3)} ${code.slice(3)}`;

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
  if (ov.devices.length === 0) return "No other device paired yet. Add one below.";
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

function useCountdown(until: number | null) {
  const [now, setNow] = useState(Date.now());
  useEffect(() => {
    if (!until) return;
    const t = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(t);
  }, [until]);
  if (!until) return null;
  const left = Math.max(0, Math.ceil((until - now) / 1000));
  return `${Math.floor(left / 60)}:${String(left % 60).padStart(2, "0")}`;
}

/** "Add a device": shows a 6-digit code for a device on this network, and a long code with its QR code for one somewhere else. */
export function AddDeviceDialog({ open, onClose }: { open: boolean; onClose: () => void }) {
  const [st, setSt] = useState<DevicePairState | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [answering, setAnswering] = useState(false);
  const alive = useRef(false);
  const showing = st?.phase === "waiting" || st?.phase === "confirm";
  const left = useCountdown(showing ? st.expiresAt : null);
  const start = () => {
    setErr(null);
    backend.devicePairStart().then(
      (s) => alive.current && setSt(s),
      (e) => alive.current && setErr(errText(e) || "Couldn't show a code."),
    );
  };
  const cancel = () =>
    backend.devicePairCancel().catch((e) => {
      console.warn("pairing not cancelled", e);
    });
  useEffect(() => {
    if (!open) return;
    alive.current = true;
    // Listening first, so nothing that happens right after the codes show is missed.
    const off = backend.listen("device-sync://pair", (s) => alive.current && setSt(s));
    void off.then(() => alive.current && start());
    return () => {
      alive.current = false;
      void off.then((u) => u());
      void cancel();
    };
  }, [open]);
  useEffect(() => {
    if (left !== "0:00") return;
    // The time is up: the codes stop here even before the other side says so.
    setSt((s) => (s && (s.phase === "waiting" || s.phase === "confirm") ? { ...s, phase: "expired", code: null, longCode: null, qr: null, expiresAt: null } : s));
    backend.devicePairState().then(
      (s) => alive.current && s.phase !== "waiting" && setSt(s),
      () => {},
    );
  }, [left]);
  const answer = (yes: boolean) => {
    setAnswering(true);
    backend.devicePairConfirm(yes).then(
      (s) => {
        if (!alive.current) return;
        setAnswering(false);
        setSt(s);
      },
      (e) => {
        if (!alive.current) return;
        setAnswering(false);
        setErr(errText(e) || "Couldn't answer.");
      },
    );
  };
  const close = () => {
    if (showing) void cancel();
    setSt(null);
    onClose();
  };
  const copyLong = () => {
    if (!st?.longCode) return;
    navigator.clipboard?.writeText(st.longCode).then(
      () => toast.info("Long code copied"),
      () => toast.error("Couldn't copy the long code"),
    );
  };
  return (
    <Dialog open={open} onClose={close} label="Add a device" className="confirm keys-dialog">
      <h2 className="confirm__title">Add a device</h2>
      {err ? (
        <p className="confirm__body" role="alert">
          {err}
        </p>
      ) : st?.phase === "paired" ? (
        <p className="confirm__body" role="status">
          Paired with {st.with ?? "your other device"}. The two keep in sync from now on, at home and away.
        </p>
      ) : st?.phase === "confirm" ? (
        <>
          <p className="confirm__body" role="status">
            Pair with {st.with ?? "your other device"}? It used the long code. Say yes only if it's yours.
          </p>
          <div className="confirm__actions">
            <Button variant="ghost" disabled={answering} onClick={() => answer(false)}>
              Don't pair
            </Button>
            <Button variant="primary" disabled={answering} onClick={() => answer(true)}>
              Pair
            </Button>
          </div>
        </>
      ) : st?.phase === "expired" || st?.phase === "locked" || st?.phase === "declined" ? (
        <p className="confirm__body" role="status">
          {st.phase === "expired"
            ? "Those codes ran out."
            : st.phase === "declined"
              ? "Not paired."
              : "Too many wrong codes were tried, so these codes stopped working."}{" "}
          Get new codes to try again.
        </p>
      ) : (
        <>
          <p className="confirm__body">
            On your other device, signed in to the same Plex account, open Settings, choose Pair with a code under Keep my devices in sync, and enter one
            of these.
          </p>
          <div className="keys-code">
            <div>
              <p className="device-pair__label">On the same network</p>
              <p className="keys-code__digits" aria-label={st?.code ? `Code ${[...st.code].join(" ")}` : "Getting a code"}>
                {st?.code ? spaced(st.code) : "··· ···"}
              </p>
              <p className="keys-code__left">{left ? `Works for ${left}` : " "}</p>
            </div>
          </div>
          <p className="device-pair__label">Somewhere else</p>
          <div className="keys-code">
            {st?.qr && <QrCode qr={st.qr} label="QR code for the long code" className="keys-qr" />}
            <div className="device-pair__long">
              <p className="device-pair__text" role={st?.note ? "status" : undefined}>
                {st?.note ?? "Scan this QR code, or paste the long code. It goes through the sealed relay."}
              </p>
              {st?.longCode && (
                <Button size="sm" variant="secondary" onClick={copyLong}>
                  Copy the long code
                </Button>
              )}
            </div>
          </div>
        </>
      )}
      <div className="confirm__actions">
        {(st?.phase === "expired" || st?.phase === "locked" || st?.phase === "declined") && (
          <Button variant="secondary" onClick={start}>
            New codes
          </Button>
        )}
        <Button variant={st?.phase === "paired" ? "primary" : "ghost"} onClick={close}>
          {st?.phase === "paired" ? "Done" : "Cancel"}
        </Button>
      </div>
    </Dialog>
  );
}

/** A code typed here: 6 digits, or a long code (PB1-...), or the link a QR code holds. */
export function pairCodeReady(raw: string): boolean {
  const t = raw.trim();
  return /^\d{6}$/.test(t.replace(/[\s-]/g, "")) || /^(platter:\/\/pair-device\/)?PB1-\S{20,}$/.test(t);
}

/** "Pair with a code": the code another device shows. */
export function JoinDeviceDialog({ open, onClose, onPaired }: { open: boolean; onClose: () => void; onPaired: () => void }) {
  const [code, setCode] = useState("");
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const [done, setDone] = useState<string | null>(null);
  const field = useRef<HTMLInputElement>(null);
  // Bumped on open, close and unmount: an answer to an earlier try is dropped.
  const attempt = useRef(0);
  useEffect(() => {
    attempt.current++;
    if (!open) return;
    setCode("");
    setErr(null);
    setDone(null);
    setBusy(false);
  }, [open]);
  useEffect(
    () => () => {
      attempt.current++;
    },
    [],
  );
  const join = () => {
    const n = ++attempt.current;
    setBusy(true);
    setErr(null);
    backend.devicePairJoin(code.trim()).then(
      (p) => {
        // Paired even if the dialog closed meanwhile: the list still updates.
        onPaired();
        if (n !== attempt.current) return;
        setBusy(false);
        setDone(`Paired with ${p.name}. The two keep in sync from now on.`);
      },
      (e) => {
        if (n !== attempt.current) return;
        setBusy(false);
        setErr(errText(e) || "Couldn't pair.");
      },
    );
  };
  return (
    <Dialog open={open} onClose={onClose} label="Pair with a code" className="confirm keys-dialog" initialFocus={field}>
      <h2 className="confirm__title">Pair with a code</h2>
      {done ? (
        <p className="confirm__body" role="status">
          {done}
        </p>
      ) : (
        <form
          onSubmit={(e) => {
            e.preventDefault();
            if (pairCodeReady(code) && !busy) join();
          }}
        >
          <p className="confirm__body">
            On the other device, choose Add a device. On the same network, type its 6 digits. Somewhere else, paste its long code.
          </p>
          <label className="keys-send__label" htmlFor="device-pair-code">
            Code
          </label>
          <input
            id="device-pair-code"
            ref={field}
            className="keys-send__code device-pair__input"
            autoComplete="off"
            autoCapitalize="off"
            spellCheck={false}
            placeholder="000 000 or PB1-…"
            value={code}
            onChange={(e) => setCode(e.target.value)}
          />
          {err && (
            <p className="keys-send__error" role="alert">
              {err}
            </p>
          )}
          <div className="confirm__actions">
            <Button variant="ghost" onClick={onClose}>
              Cancel
            </Button>
            <Button type="submit" variant="primary" disabled={busy || !pairCodeReady(code)}>
              {busy ? "Pairing…" : "Pair"}
            </Button>
          </div>
        </form>
      )}
      {done && (
        <div className="confirm__actions">
          <Button variant="primary" onClick={onClose}>
            Done
          </Button>
        </div>
      )}
    </Dialog>
  );
}

export function PairButton({ kind, onPaired }: { kind: "add" | "join"; onPaired: () => void }) {
  const [open, setOpen] = useState(false);
  return (
    <>
      <Button size="sm" onClick={() => setOpen(true)}>
        {kind === "add" ? "Show codes" : "Enter a code"}
      </Button>
      {kind === "add" ? (
        <AddDeviceDialog
          open={open}
          onClose={() => {
            setOpen(false);
            onPaired();
          }}
        />
      ) : (
        <JoinDeviceDialog open={open} onClose={() => setOpen(false)} onPaired={onPaired} />
      )}
    </>
  );
}
