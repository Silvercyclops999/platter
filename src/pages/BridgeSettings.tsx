/**
 * Settings > Bridge: Platter Bridge, the optional home side for a server
 * (docs/BRIDGE.md). Pair one with a code, see what it's doing and when it
 * last caught up with Lidarr, unpair it, and say yes or no to Lidarr
 * settings changes it queued (it never makes those itself).
 */
import { useCallback, useEffect, useState } from "react";
import { Button } from "../components/Button";
import { Skeleton } from "../components/Skeleton";
import { backend } from "../lib/backend";
import { timeAgo } from "../lib/relativeTime";
import type { BridgeInfo, BridgeMusicAccess, BridgeOverview } from "../lib/types";
import { toast } from "../stores/toasts";
import "./feed-settings.css";
import { failText } from "../lib/errorText";

export const BRIDGE_DESCRIPTION =
  "Optional. A small Docker container for your server that does Platter's Lidarr work while no computer has Platter open: catching up after downloads, scanning just the new folders, and adding protected albums when it can reach your music folder. Platter works fully without it, and nothing is installed on your server unless you set it up. It talks to Platter only through the same sealed relay as Listen along, and opens no ports.";

const SECTION_WORDS = "bridge server docker unraid container home side lidarr pair code unpair headless relay";

export function bridgeMatches(query: string): boolean {
  const words = query.trim().toLowerCase().split(/\s+/).filter(Boolean);
  const hay = `${SECTION_WORDS} ${BRIDGE_DESCRIPTION}`.toLowerCase();
  return words.every((w) => hay.includes(w));
}

const MUSIC: Record<BridgeMusicAccess, string> = {
  readWrite: "Music folder mounted read-write: it adds new protected albums itself.",
  readOnly: "Music folder mounted read-only: additions wait for a computer.",
  none: "No music folder mounted: additions wait for a computer.",
};

/** Kept current from `bridge://changed`. */
export function useBridge(): [BridgeOverview | null, (o: BridgeOverview) => void, () => void] {
  const [overview, setOverview] = useState<BridgeOverview | null>(null);
  const load = useCallback(() => {
    backend.bridgeOverview().then(setOverview, () => {});
  }, []);
  useEffect(() => {
    load();
    const un = backend.listen("bridge://changed", load);
    return () => void un.then((f) => f());
  }, [load]);
  return [overview, setOverview, load];
}

function BridgeRow({ b, onUnpair }: { b: BridgeInfo; onUnpair: () => void }) {
  const [sure, setSure] = useState(false);
  const st = b.status;
  const last = st?.lastCatchUp;
  return (
    <div className="setting-row" data-testid="bridge-row">
      <div className="setting-row__text">
        <div className="setting-row__title">{b.name}</div>
        <p className="setting-row__desc">
          {b.online ? "Answering" : "Not heard from lately"}
          {b.lastSeen ? `, last seen ${timeAgo(b.lastSeen)}` : ""}. {MUSIC[b.music]}
        </p>
        {st && <p className="setting-row__desc">Now: {st.doing}</p>}
        <p className="setting-row__desc">
          {last
            ? last.error
              ? `Last catch-up ${timeAgo(last.at)} didn't finish: ${last.error}`
              : `Last catch-up ${timeAgo(last.at)}: ${last.handled} Lidarr ${last.handled === 1 ? "event" : "events"}, ${last.acted} acted on${last.announced.length ? `, new: ${last.announced.join(", ")}` : ""}.`
            : "No catch-up yet."}
        </p>
        {st?.problem && (
          <p className="info-key__error" role="alert">
            {st.problem}
          </p>
        )}
      </div>
      {sure ? (
        <span>
          <Button size="sm" variant="primary" onClick={onUnpair}>
            Unpair {b.name}
          </Button>{" "}
          <Button size="sm" variant="ghost" onClick={() => setSure(false)}>
            Cancel
          </Button>
        </span>
      ) : (
        <Button size="sm" onClick={() => setSure(true)}>
          Unpair
        </Button>
      )}
    </div>
  );
}

export function BridgeSettings() {
  const [ov, setOv, load] = useBridge();
  const pair = () =>
    backend.bridgePairStart().then(
      () => load(),
      () => toast.error("Couldn't make a pair code"),
    );
  const cancel = () => void backend.bridgePairCancel().then(load);
  const unpair = (b: BridgeInfo) =>
    backend.bridgeUnpair(b.id).then(
      (o) => {
        setOv(o);
        toast.info(`${b.name} was unpaired. Remove its container if you don't need it any more`);
      },
      () => toast.error("Couldn't unpair the Bridge"),
    );
  const answer = (id: string, yes: boolean) =>
    (yes ? backend.bridgeQueuedApply(id) : backend.bridgeQueuedDismiss(id)).then(setOv, (e: unknown) =>
      toast.error(failText(yes ? "Couldn't apply the Bridge's queued change" : "Couldn't dismiss the Bridge's queued change", e)),
    );
  return (
    <section className="settings__group" aria-labelledby="settings-bridge" data-testid="bridge-settings">
      <h2 id="settings-bridge" className="settings__heading">
        Bridge
      </h2>
      <div className="setting-row">
        <div className="setting-row__text">
          <div className="setting-row__title">Platter Bridge</div>
          <p className="setting-row__desc">{BRIDGE_DESCRIPTION}</p>
          {ov?.pairing && (
            <div data-testid="bridge-code">
              <p className="setting-row__desc">
                Start the Bridge with BRIDGE_PAIR_CODE set to this code. It works once, for the next hour, while Platter is open here:
              </p>
              <code className="bridge-code" style={{ wordBreak: "break-all", userSelect: "all" }}>
                {ov.pairing.code}
              </code>{" "}
              <Button size="sm" variant="ghost" onClick={() => void navigator.clipboard?.writeText(ov.pairing?.code ?? "")}>
                Copy
              </Button>{" "}
              <Button size="sm" variant="ghost" onClick={cancel}>
                Cancel
              </Button>
            </div>
          )}
        </div>
        {!ov ? (
          <Skeleton width={120} height={32} radius="pill" />
        ) : (
          !ov.pairing && (
            <Button size="sm" variant="primary" onClick={() => void pair()}>
              Pair a Bridge
            </Button>
          )
        )}
      </div>
      {ov?.bridges.map((b) => <BridgeRow key={b.id} b={b} onUnpair={() => void unpair(b)} />)}
      {ov?.queued.map((q) => (
        <div className="setting-row" key={q.id} data-testid="bridge-queued">
          <div className="setting-row__text">
            <div className="setting-row__title">Waiting for you: {q.what}</div>
            <p className="setting-row__desc">
              {q.device} didn't make this change to Lidarr's settings, because a Bridge never does that on its own. Exactly what it would send:{" "}
              <code style={{ wordBreak: "break-all" }}>
                {q.method} {q.path} {q.body}
              </code>
            </p>
          </div>
          <span>
            <Button size="sm" onClick={() => void answer(q.id, true)}>
              Make this change
            </Button>{" "}
            <Button size="sm" variant="ghost" onClick={() => void answer(q.id, false)}>
              Dismiss
            </Button>
          </span>
        </div>
      ))}
    </section>
  );
}
