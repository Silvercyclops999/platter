/**
 * Settings > Stream Deck: allow the Stream Deck plugin (off by default),
 * the pairing code to type into it, and Revoke. This device only.
 */
import { useEffect, useId, useState } from "react";
import { Button } from "../components/Button";
import { Skeleton } from "../components/Skeleton";
import { Switch } from "../components/Switch";
import { backend } from "../lib/backend";
import type { DeckStatus } from "../lib/types";
import { updateSettings, useSettings } from "../stores/settings";
import { toast } from "../stores/toasts";
import "./feed-settings.css";

export const DECK_DESCRIPTION =
  "Lets the Platter plugin for Elgato Stream Deck show the cover and control playback: play and pause, skip, like, volume, request the album through Lidarr and Listen along. Only a Stream Deck you pair on this computer can use it, never your network. This device only.";

const SECTION_WORDS = "stream deck elgato plugin keys buttons pairing code revoke control";

/** Whether the settings search shows this section. */
export function streamDeckMatches(query: string): boolean {
  const words = query.trim().toLowerCase().split(/\s+/).filter(Boolean);
  if (!words.length) return true;
  const hay = `${SECTION_WORDS} ${DECK_DESCRIPTION}`.toLowerCase();
  return words.every((w) => hay.includes(w));
}

/** The control API's status, kept current from `deck://status`. */
export function useDeckStatus(): [DeckStatus | null, (s: DeckStatus) => void] {
  const [status, setStatus] = useState<DeckStatus | null>(null);
  useEffect(() => {
    let alive = true;
    backend.deckStatus().then(
      (s) => alive && setStatus(s),
      () => {},
    );
    const un = backend.listen("deck://status", (s) => alive && setStatus(s));
    return () => {
      alive = false;
      void un.then((f) => f());
    };
  }, []);
  return [status, setStatus];
}

/** Under the switch: the code, or why the port couldn't open. */
export function DeckPairing({ status, onRevoke }: { status: DeckStatus | null; onRevoke: () => void }) {
  if (!status || status.state === "off") return null;
  if (status.state === "portInUse")
    return (
      <span className="info-key__error" role="alert">
        Port {status.port} is in use by another program, so the Stream Deck can't reach Platter.
      </span>
    );
  if (status.state === "failed")
    return (
      <span className="info-key__error" role="alert">
        Couldn't open port {status.port}: {status.message}
      </span>
    );
  return (
    <span className="feed-note deck-pairing" data-testid="deck-pairing">
      <span>
        Pairing code: <code className="deck-pairing__code">{status.code}</code>
      </span>
      <span>{status.paired ? "A Stream Deck is paired." : "Type this code into the Platter plugin's settings in the Stream Deck app."}</span>
      {status.paired && (
        <Button size="sm" variant="ghost" onClick={onRevoke}>
          Revoke
        </Button>
      )}
    </span>
  );
}

export function StreamDeckSettings() {
  const s = useSettings((x) => x.settings);
  const [status, setStatus] = useDeckStatus();
  const id = useId();
  const on = !!s?.allowStreamDeck;
  const revoke = () =>
    backend.deckRevoke().then(
      (st) => {
        setStatus(st);
        toast.info("The Stream Deck was unpaired. Pair it again with the new code.");
      },
      () => toast.error("Couldn't unpair the Stream Deck"),
    );
  return (
    <section className="settings__group" aria-labelledby="settings-stream-deck" data-testid="stream-deck-settings">
      <h2 id="settings-stream-deck" className="settings__heading">
        Stream Deck
      </h2>
      <div className="setting-row" data-setting="allowStreamDeck">
        <div className="setting-row__text">
          <div id={`${id}-t`} className="setting-row__title">
            Allow Stream Deck
          </div>
          <p id={`${id}-d`} className="setting-row__desc">
            {DECK_DESCRIPTION}
          </p>
          {on && <DeckPairing status={status} onRevoke={() => void revoke()} />}
        </div>
        {s ? (
          <Switch checked={on} onChange={(v) => void updateSettings({ allowStreamDeck: v })} labelledBy={`${id}-t`} describedBy={`${id}-d`} />
        ) : (
          <Skeleton width={40} height={24} radius="pill" />
        )}
      </div>
    </section>
  );
}
