/**
 * Asks before acting on an invite or share link that arrived from outside
 * the app (a browser, a chat app). Mounted once in the shell; what's
 * waiting lives in stores/share.
 */
import { useRef } from "react";
import { confirmLink, declineLink, relayName, useLinkAsk, type LinkAsk } from "../stores/share";
import { Button } from "./Button";
import { Dialog } from "./Dialog";

const WORDS: Record<LinkAsk["what"], { title: string; does: string; yes: string }> = {
  join: { title: "Join a listening session?", does: "join their listening session", yes: "Join" },
  circle: { title: "Join a shared playlist?", does: "join their shared playlist or Blend", yes: "Join" },
  share: { title: "Ask for someone's music?", does: "ask them to share their Plex music with you, which tells them your Plex username", yes: "Ask" },
};

export function LinkAskDialog() {
  const ask = useLinkAsk((s) => s.ask);
  const no = useRef<HTMLButtonElement>(null);
  if (!ask) return null;
  const w = WORDS[ask.what];
  return (
    <Dialog open onClose={declineLink} label={w.title} className="confirm" initialFocus={no}>
      <h2 className="confirm__title">{w.title}</h2>
      <p className="confirm__body">
        A link opened Platter. Going ahead will {w.does}, connecting through {relayName(ask.relay)}. Links don't say who made them, so only go ahead if
        you know who sent it.
      </p>
      <div className="confirm__actions">
        <Button ref={no} variant="ghost" onClick={declineLink}>
          Not now
        </Button>
        <Button variant="primary" onClick={() => void confirmLink()}>
          {w.yes}
        </Button>
      </div>
    </Dialog>
  );
}
