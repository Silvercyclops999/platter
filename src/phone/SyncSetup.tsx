/**
 * Copy from my computer, on the phone: a setup step after the first
 * sign-in (and from the You tab later). With no computer linked yet it waits
 * for one: every Platter on the same Plex account finds the others by
 * itself, so there's no code to type. Then it copies one group at a
 * time with a progress line, and can be stopped. What came stays, and the
 * next round with the computer brings the rest. The words are the ones
 * Settings uses on the computer; only the arrangement is the phone's.
 */
import { useEffect, useRef, useState } from "react";
import { create } from "zustand";
import { backend } from "../lib/backend";
import { errorMessage } from "../lib/errorText";
import type { DeviceCopyProgress, DeviceCopyResult } from "../lib/types";
import { tap } from "./bits";
import "../pages/ledger.css";
import "./sync-setup.css";

/** Whether the step is showing, and where it starts. */
export const useSyncStep = create<{ open: boolean; firstRun: boolean }>(() => ({ open: false, firstRun: false }));

/** Show the step: `firstRun` right after signing in for the first time. */
export function openSyncStep(firstRun = false) {
  useSyncStep.setState({ open: true, firstRun });
}

export function closeSyncStep() {
  useSyncStep.setState({ open: false, firstRun: false });
}

export type SyncStepPhase = "choose" | "looking" | "copying" | "done" | "stopped" | "failed";

/** How often the phone checks whether its computer has turned up. */
const LOOK_MS = 3000;

/** "Copying your pins and folders… 2 of 7". */
export function progressLine(p: DeviceCopyProgress): string {
  return `${p.label}… ${p.step} of ${p.of}`;
}

/** A group's count for the list at the end. */
export function countWords(n: number): string {
  return n === 0 ? "None" : n.toLocaleString();
}

export function SyncStepHost() {
  const open = useSyncStep((s) => s.open);
  const firstRun = useSyncStep((s) => s.firstRun);
  return open ? <SyncStep firstRun={firstRun} onClose={closeSyncStep} /> : null;
}

export function SyncStep({ firstRun, onClose }: { firstRun: boolean; onClose: () => void }) {
  const [phase, setPhase] = useState<SyncStepPhase>(firstRun ? "choose" : "copying");
  const [paired, setPaired] = useState<boolean | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [progress, setProgress] = useState<DeviceCopyProgress | null>(null);
  const [result, setResult] = useState<DeviceCopyResult | null>(null);
  const [stopping, setStopping] = useState(false);
  // After signing in: shown once it's known no computer is paired yet.
  const [ready, setReady] = useState(!firstRun);
  const alive = useRef(true);
  const started = useRef(false);

  useEffect(() => {
    alive.current = true;
    const off = backend.listen("device-sync://copy", (p) => {
      if (alive.current) setProgress(p);
    });
    return () => {
      alive.current = false;
      void off.then((u) => u());
    };
  }, []);

  const copy = () => {
    setPhase("copying");
    setProgress(null);
    setError(null);
    setStopping(false);
    backend.pairCopySettings().then(
      (r) => {
        if (!alive.current) return;
        setResult(r);
        setPhase(r.cancelled ? "stopped" : "done");
      },
      (e: unknown) => {
        if (!alive.current) return;
        setError(errorMessage(e) || "Couldn't copy from your computer.");
        setPhase("failed");
      },
    );
  };

  /** Copy now if a computer is linked; wait for one to turn up if not. */
  const begin = () => {
    tap();
    backend.deviceSyncStatus().then(
      (o) => {
        if (!alive.current) return;
        const any = o.devices.length > 0;
        setPaired(any);
        if (!o.on) {
          setError("Turn on Keep my devices in sync first, in Settings under Sync.");
          setPhase("failed");
        } else if (any) copy();
        else setPhase("looking");
      },
      (e: unknown) => {
        if (!alive.current) return;
        setError(errorMessage(e) || "Couldn't read sync status.");
        setPhase("failed");
      },
    );
  };

  // Opened from You: go straight on. After signing in, only when no
  // computer is paired yet (signing in again changes nothing).
  useEffect(() => {
    if (started.current) return;
    started.current = true;
    if (!firstRun) begin();
    else
      backend.deviceSyncStatus().then(
        (o) => {
          if (!alive.current) return;
          if (o.devices.length > 0) onClose();
          else setReady(true);
        },
        () => {
          if (alive.current) setReady(true);
        },
      );
    // Once, on open.
  }, []);

  // Waiting for the computer: it links by itself once it's open and
  // signed in to the same Plex account.
  useEffect(() => {
    if (phase !== "looking") return;
    let done = false;
    const check = () =>
      backend.deviceSyncStatus().then(
        (o) => {
          if (!alive.current || done || o.devices.length === 0) return;
          done = true;
          setPaired(true);
          copy();
        },
        () => {},
      );
    const off = backend.listen("device-sync://status", (devices) => {
      if (devices.length > 0) void check();
    });
    // Look once more when listening starts: one that linked in between doesn't wait for the next look.
    void off.then(() => check());
    const t = window.setInterval(() => void check(), LOOK_MS);
    return () => {
      done = true;
      window.clearInterval(t);
      void off.then((u) => u());
    };
    // copy is stable enough for this: it only sets state.
  }, [phase]);

  const stop = () => {
    tap();
    setStopping(true);
    backend.pairCopyCancel().catch((e: unknown) => console.warn("copy cancel", e));
  };

  const close = () => {
    tap();
    onClose();
  };

  if (!ready) return null;

  return (
    <div className="ph-sync-step ph-signin" data-testid="phone-sync-step" data-step={phase} role="dialog" aria-modal="true" aria-labelledby="ph-sync-title">
      <main className="ph-signin__scroll">
        {phase === "choose" && (
          <>
            <div className="ph-signin__words">
              <p className="ph-eyebrow">Set up this phone</p>
              <h1 id="ph-sync-title" className="ph-title ph-signin__title">
                Bring your setup over
              </h1>
              <p className="ph-signin__lead">Copy your look, pins, follows and lyrics from Platter on your computer, or start fresh. You can copy later from You.</p>
            </div>
            <div className="ph-signin__actions">
              <button type="button" className="ph-big-btn is-primary" onClick={begin}>
                Copy from my computer
              </button>
              <button type="button" className="ph-big-btn is-quiet" onClick={close}>
                Start fresh
              </button>
              <p className="ph-signin__fine">Your sign-in and keys stay on each device.</p>
            </div>
          </>
        )}

        {phase === "looking" && (
          <>
            <div className="ph-signin__words">
              <p className="ph-eyebrow">Copy from my computer</p>
              <h1 id="ph-sync-title" className="ph-title ph-signin__title">
                Looking for your computer
              </h1>
              <p className="ph-sync-step__line" role="status" aria-live="polite">
                Open Platter on your computer, signed in to the same Plex account. This phone finds it by itself.
              </p>
            </div>
            <div className="ph-signin__actions">
              <button type="button" className="ph-big-btn is-quiet" onClick={close}>
                Not now
              </button>
            </div>
          </>
        )}

        {phase === "copying" && (
          <>
            <div className="ph-signin__words">
              <p className="ph-eyebrow">{progress ? `Copying from ${progress.name}` : "Copy from my computer"}</p>
              <h1 id="ph-sync-title" className="ph-title ph-signin__title">
                {stopping ? "Stopping" : "Copying"}
              </h1>
              <p className="ph-sync-step__line" role="status" aria-live="polite">
                {progress ? (
                  <>
                    {progress.label}… <span className="num">{`${progress.step} of ${progress.of}`}</span>
                  </>
                ) : (
                  "Getting in touch with your computer…"
                )}
              </p>
              <div
                className="ph-sync-step__rule"
                role="progressbar"
                aria-label="Copied so far"
                aria-valuemin={0}
                aria-valuemax={progress?.of ?? 1}
                aria-valuenow={progress ? progress.step - 1 : 0}
              >
                <span style={{ width: `${progress ? ((progress.step - 1) / progress.of) * 100 : 0}%` }} />
              </div>
              <p className="ph-signin__lead">You can use Platter while it copies. Stop it any time: what came stays, and the rest follows next time.</p>
            </div>
            <div className="ph-signin__actions">
              <button type="button" className="ph-big-btn is-quiet" onClick={stop} disabled={stopping}>
                Stop
              </button>
            </div>
          </>
        )}

        {(phase === "done" || phase === "stopped") && result && (
          <>
            <div className="ph-signin__words">
              <p className="ph-eyebrow">{phase === "done" ? "Done" : "Stopped"}</p>
              <h1 id="ph-sync-title" className="ph-title ph-signin__title">
                {phase === "done" ? `In sync with ${result.name}` : "Copy stopped"}
              </h1>
              <p className="ph-signin__lead">
                {phase === "done"
                  ? (result.later ?? "From now on, what you change on either one shows up on the other.")
                  : `What came over stays. The rest comes from ${result.name} the next time both are open.`}
              </p>
            </div>
            <ul className="ledger ph-sync-step__counts" aria-label="Copied">
              {result.groups.map((g) => (
                <li key={g.group} className="ledger__row">
                  <span className="ledger__label">{g.label}</span>
                  <span className="ledger__value">{countWords(g.count)}</span>
                </li>
              ))}
            </ul>
            <div className="ph-signin__actions">
              <button type="button" className="ph-big-btn is-primary" onClick={close}>
                Done
              </button>
            </div>
          </>
        )}

        {phase === "failed" && (
          <>
            <div className="ph-signin__words">
              <p className="ph-eyebrow">Copy from my computer</p>
              <h1 id="ph-sync-title" className="ph-title ph-signin__title">
                That didn't work
              </h1>
              <p className="ph-signin__lead" role="alert">
                {error}
              </p>
            </div>
            <div className="ph-signin__actions">
              <button type="button" className="ph-big-btn is-primary" onClick={paired === false ? () => setPhase("looking") : begin}>
                Try again
              </button>
              <button type="button" className="ph-big-btn is-quiet" onClick={close}>
                {firstRun ? "Start fresh" : "Close"}
              </button>
            </div>
          </>
        )}
      </main>
    </div>
  );
}
