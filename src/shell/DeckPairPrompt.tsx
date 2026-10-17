/**
 * "Allow Stream Deck to control Platter?": shown when the Platter plugin in
 * the Stream Deck app asks to pair (`deck://pair-request`). Only Allow here
 * pairs it; closing the prompt is Don't allow.
 */
import { useEffect, useRef, useState } from "react";
import { Button } from "../components/Button";
import { Dialog } from "../components/Dialog";
import { backend } from "../lib/backend";
import { toast } from "../stores/toasts";

export function DeckPairPrompt() {
  const [id, setId] = useState<string | null>(null);
  const deny = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    let alive = true;
    backend.deckPairPrompt().then(
      (p) => alive && setId(p),
      () => {},
    );
    const un = backend.listen("deck://pair-request", (p) => alive && setId(p.id));
    return () => {
      alive = false;
      void un.then((f) => f());
    };
  }, []);
  if (!id) return null;
  const answer = (allow: boolean) => {
    setId(null);
    backend.deckPairAnswer(id, allow).then(
      (done) => {
        if (allow && done) toast.info("Stream Deck paired. Its keys control Platter now");
        else if (allow) toast.info("That request had already gone. Press a Platter key on the Stream Deck to ask again");
      },
      () => toast.error("Couldn't pair the Stream Deck"),
    );
  };
  return (
    <Dialog open onClose={() => answer(false)} label="Allow Stream Deck to control Platter?" className="confirm" initialFocus={deny}>
      <h2 className="confirm__title">Allow Stream Deck to control Platter?</h2>
      <p className="confirm__body">
        The Platter plugin in the Stream Deck app asked to pair. It will show what's playing and control playback on this computer. Allow it only if you're
        setting up your Stream Deck.
      </p>
      <div className="confirm__actions">
        <Button ref={deny} variant="ghost" onClick={() => answer(false)}>
          Don't allow
        </Button>
        <Button variant="primary" onClick={() => answer(true)}>
          Allow
        </Button>
      </div>
    </Dialog>
  );
}
